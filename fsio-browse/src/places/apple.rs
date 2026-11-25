//! macOS keeps the Finder sidebar's favorites in a keyed archive of bookmark
//! data; volumes come from the mount table. An iOS app sees its own
//! container, whose Documents folder the Files app shows under the app's name.

#[cfg(target_os = "ios")]
use std::env;
use std::path::PathBuf;

#[cfg(target_os = "macos")]
use super::user_known_folders;
use super::{KnownFolder, PinnedFolder, Volume};

#[cfg(target_os = "macos")]
pub(super) fn known_folders() -> Vec<(KnownFolder, PathBuf)> {
    user_known_folders()
}

#[cfg(target_os = "ios")]
pub(super) fn known_folders() -> Vec<(KnownFolder, PathBuf)> {
    env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Documents"))
        .filter(|documents| documents.is_dir())
        .map(|documents| vec![(KnownFolder::Documents, documents)])
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
pub(super) fn pinned_folders() -> Vec<PinnedFolder> {
    finder::favorites().unwrap_or_default()
}

#[cfg(target_os = "ios")]
pub(super) fn pinned_folders() -> Vec<PinnedFolder> {
    Vec::new()
}

#[cfg(target_os = "macos")]
pub(super) fn volumes() -> Vec<Volume> {
    mounts::volumes()
}

#[cfg(target_os = "ios")]
pub(super) fn volumes() -> Vec<Volume> {
    Vec::new()
}

#[cfg(target_os = "macos")]
mod finder {
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::ptr;

    use objc2::msg_send;
    use objc2::rc::{Retained, autoreleasepool};
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::{
        NSData, NSKeyedUnarchiver, NSSet, NSString, NSURL, NSURLBookmarkResolutionOptions,
    };

    use crate::places::PinnedFolder;

    /// Newest format first: macOS 26 writes sfl4, macOS 13 to 15 sfl3.
    const ARCHIVES: [&str; 3] = [
        "com.apple.LSSharedFileList.FavoriteItems.sfl4",
        "com.apple.LSSharedFileList.FavoriteItems.sfl3",
        "com.apple.LSSharedFileList.FavoriteItems.sfl2",
    ];

    /// The folders in the Finder sidebar's Favorites, in its order. A
    /// sandboxed build cannot read the archive and gets none.
    pub(super) fn favorites() -> Option<Vec<PinnedFolder>> {
        let folder = PathBuf::from(env::var_os("HOME")?)
            .join("Library/Application Support/com.apple.sharedfilelist");
        let bytes = ARCHIVES
            .iter()
            .find_map(|name| fs::read(folder.join(name)).ok())?;
        autoreleasepool(|_| decode(&bytes))
    }

    fn decode(bytes: &[u8]) -> Option<Vec<PinnedFolder>> {
        let data = NSData::with_bytes(bytes);
        let classes: Vec<&AnyClass> = [
            c"NSDictionary",
            c"NSArray",
            c"NSData",
            c"NSString",
            c"NSNumber",
            c"NSUUID",
            c"NSDate",
            c"NSURL",
        ]
        .iter()
        .filter_map(|name| AnyClass::get(name))
        .collect();
        let classes = NSSet::from_slice(&classes);
        // SAFETY: the class set holds only plist and Foundation value
        // classes, so decoding cannot instantiate anything else.
        let root =
            unsafe { NSKeyedUnarchiver::unarchivedObjectOfClasses_fromData_error(&classes, &data) }
                .ok()?;
        let items = object_for_key(&root, "items")?;
        let dictionary = AnyClass::get(c"NSDictionary")?;
        let array = AnyClass::get(c"NSArray")?;
        if !is_kind_of(&root, dictionary) || !is_kind_of(&items, array) {
            return None;
        }
        // SAFETY: `items` is an NSArray, checked above.
        let count: usize = unsafe { msg_send![&*items, count] };
        let mut pinned = Vec::new();
        for index in 0..count {
            // SAFETY: `index` is below the array's count.
            let item: Retained<AnyObject> = unsafe { msg_send![&*items, objectAtIndex: index] };
            if !is_kind_of(&item, dictionary) {
                continue;
            }
            let Some(path) = object_for_key(&item, "Bookmark")
                .and_then(|bookmark| bookmark.downcast::<NSData>().ok())
                .and_then(|bookmark| resolve(&bookmark))
            else {
                continue;
            };
            pinned.push(PinnedFolder { path, label: None });
        }
        Some(pinned)
    }

    fn object_for_key(dictionary: &AnyObject, key: &str) -> Option<Retained<AnyObject>> {
        let key = NSString::from_str(key);
        // SAFETY: callers pass an NSDictionary; objectForKey: returns nil
        // for a missing key.
        unsafe { msg_send![dictionary, objectForKey: &*key] }
    }

    fn is_kind_of(object: &AnyObject, class: &AnyClass) -> bool {
        // SAFETY: isKindOfClass: is defined on every NSObject.
        unsafe { msg_send![object, isKindOfClass: class] }
    }

    /// The local folder a bookmark points to, without mounting anything or
    /// asking the person.
    fn resolve(bookmark: &NSData) -> Option<PathBuf> {
        let options = NSURLBookmarkResolutionOptions::WithoutUI
            | NSURLBookmarkResolutionOptions::WithoutMounting;
        // SAFETY: a null stale pointer is allowed.
        let url = unsafe {
            NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                bookmark,
                options,
                None,
                ptr::null_mut(),
            )
        }
        .ok()?;
        if !url.isFileURL() {
            return None;
        }
        let path = PathBuf::from(url.path()?.to_string());
        path.is_dir().then_some(path)
    }
}

#[cfg(target_os = "macos")]
mod mounts {
    use std::path::Path;

    use sysinfo::Disks;

    use crate::location::Location;
    use crate::places::{Volume, VolumeKind, VolumeSpace};

    fn volume_kind(mount: &Path, file_system: &str, removable: bool) -> VolumeKind {
        match file_system {
            _ if mount == Path::new("/") => VolumeKind::System,
            "cd9660" | "udf" => VolumeKind::Optical,
            "smbfs" | "afpfs" | "nfs" | "webdav" | "ftp" => VolumeKind::Network,
            _ if removable => VolumeKind::Removable,
            _ => VolumeKind::Fixed,
        }
    }

    /// The startup disk and everything under `/Volumes`, as Finder lists
    /// them. The system's own APFS volumes under `/System/Volumes` stay out.
    pub(super) fn volumes() -> Vec<Volume> {
        let disks = Disks::new_with_refreshed_list();
        let mut volumes: Vec<Volume> = disks
            .list()
            .iter()
            .filter(|disk| {
                let mount = disk.mount_point();
                mount == Path::new("/") || mount.starts_with("/Volumes")
            })
            .map(|disk| {
                let mount = disk.mount_point();
                let file_system = disk.file_system().to_string_lossy();
                let name = disk.name().to_string_lossy().into_owned();
                Volume {
                    location: Location::file_system(mount),
                    label: (!name.is_empty()).then_some(name),
                    kind: volume_kind(mount, &file_system, disk.is_removable()),
                    space: (disk.total_space() > 0).then(|| VolumeSpace {
                        total: disk.total_space(),
                        available: disk.available_space(),
                    }),
                }
            })
            .collect();
        volumes.sort_by(|a, b| {
            (a.kind != VolumeKind::System, &a.location.path)
                .cmp(&(b.kind != VolumeKind::System, &b.location.path))
        });
        volumes.dedup_by(|a, b| a.location == b.location);
        volumes
    }
}
