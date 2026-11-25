//! Windows lists pinned folders under Quick access (Home on Windows 11) and
//! drives by their root path.

use std::env;
use std::path::PathBuf;

use windows::Win32::Storage::EnhancedStorage::PKEY_Home_IsPinned;
use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDriveStringsW, GetVolumeInformationW,
};
use windows::Win32::System::SystemServices::{SFGAO_FILESYSTEM, SFGAO_FOLDER};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, IEnumShellItems, IShellItem, IShellItem2, SHCreateItemFromParsingName,
    SIGDN_FILESYSPATH, SIGDN_NORMALDISPLAY,
};
use windows::core::{Interface, PCWSTR, w};

use super::{KnownFolder, PinnedFolder, Volume, VolumeKind, VolumeSpace, user_known_folders};
use crate::location::Location;
use crate::windows_com::{Apartment, display_name};

const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;
const DRIVE_RAMDISK: u32 = 6;

pub(super) fn known_folders() -> Vec<(KnownFolder, PathBuf)> {
    user_known_folders()
}

pub(super) fn pinned_folders() -> Vec<PinnedFolder> {
    let _apartment = Apartment::enter();
    quick_access().unwrap_or_else(|error| {
        log::warn!("Could not read Quick access: {error}");
        Vec::new()
    })
}

/// The file system folders in Quick access that are pinned there.
fn quick_access() -> windows::core::Result<Vec<PinnedFolder>> {
    let mut pinned = Vec::new();
    // SAFETY: the parsing name is a valid null-terminated string, and every
    // item comes from the enumerator.
    unsafe {
        let home: IShellItem = SHCreateItemFromParsingName(
            w!("shell:::{679f85cb-0220-4080-b29b-5540cc05aab6}"),
            None,
        )?;
        let items: IEnumShellItems = home.BindToHandler(None, &BHID_EnumItems)?;
        loop {
            let mut batch = [None];
            let mut fetched = 0;
            if items.Next(&mut batch, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(item) = batch[0].take() else {
                break;
            };
            let wanted = SFGAO_FILESYSTEM | SFGAO_FOLDER;
            let Ok(attributes) = item.GetAttributes(wanted) else {
                continue;
            };
            if attributes & wanted != wanted || !is_pinned(&item) {
                continue;
            }
            let Ok(path) = display_name(&item, SIGDN_FILESYSPATH) else {
                continue;
            };
            pinned.push(PinnedFolder {
                path: PathBuf::from(path),
                label: display_name(&item, SIGDN_NORMALDISPLAY).ok(),
            });
        }
    }
    Ok(pinned)
}

/// Windows 11 marks the folders pinned to Home. Windows 10 lists only pinned
/// and frequent folders there, without that mark, so all of them count.
fn is_pinned(item: &IShellItem) -> bool {
    let Ok(item) = item.cast::<IShellItem2>() else {
        return true;
    };
    // SAFETY: the key is a static PROPERTYKEY.
    match unsafe { item.GetBool(&PKEY_Home_IsPinned) } {
        Ok(pinned) => pinned.as_bool(),
        Err(_) => true,
    }
}

pub(super) fn volumes() -> Vec<Volume> {
    let mut buffer = [0u16; 512];
    // SAFETY: the buffer is writable for its whole length.
    let length = unsafe { GetLogicalDriveStringsW(Some(&mut buffer)) } as usize;
    if length == 0 || length > buffer.len() {
        return Vec::new();
    }
    let system_drive = env::var("SystemDrive")
        .unwrap_or_else(|_| "C:".to_owned())
        .to_uppercase();
    buffer[..length]
        .split(|unit| *unit == 0)
        .filter(|root| !root.is_empty())
        .filter_map(|root| volume(root, &system_drive))
        .collect()
}

/// The drive at `root` (`C:\`), or `None` for a drive letter with nothing
/// behind it.
fn volume(root: &[u16], system_drive: &str) -> Option<Volume> {
    let text = String::from_utf16_lossy(root);
    let mut wide = root.to_vec();
    wide.push(0);
    let root = PCWSTR(wide.as_ptr());
    // SAFETY: `root` points at the null-terminated `wide`, alive below.
    let drive_type = unsafe { GetDriveTypeW(root) };
    let kind = match drive_type {
        _ if text.to_uppercase().starts_with(system_drive) => VolumeKind::System,
        DRIVE_FIXED | DRIVE_RAMDISK => VolumeKind::Fixed,
        DRIVE_REMOVABLE => VolumeKind::Removable,
        DRIVE_CDROM => VolumeKind::Optical,
        DRIVE_REMOTE => VolumeKind::Network,
        _ => return None,
    };
    // A network drive can take seconds to answer when its server is gone, so
    // only local drives are asked for their label and space. An empty
    // optical drive has neither.
    let (label, space) = if kind == VolumeKind::Network {
        (None, None)
    } else {
        let mut label = [0u16; 261];
        // SAFETY: the buffers are writable for their whole length.
        let label =
            unsafe { GetVolumeInformationW(root, Some(&mut label), None, None, None, None) }
                .ok()
                .map(|()| wide_text(&label))
                .filter(|label| !label.is_empty());
        let (mut available, mut total) = (0u64, 0u64);
        // SAFETY: both out pointers point at live u64s.
        let space =
            unsafe { GetDiskFreeSpaceExW(root, Some(&mut available), Some(&mut total), None) }
                .ok()
                .filter(|()| total > 0)
                .map(|()| VolumeSpace { total, available });
        (label, space)
    };
    Some(Volume {
        location: Location::file_system(text),
        label,
        kind,
        space,
    })
}

fn wide_text(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}
