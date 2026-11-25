//! Linux keeps pinned folders in the GTK bookmarks file (GNOME, Xfce,
//! Cinnamon, MATE and every GTK file chooser) and in KDE's places file
//! (Dolphin and every KDE file dialog). Volumes come from the mount table,
//! labels from `/dev/disk/by-label`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use sysinfo::Disks;

use super::{KnownFolder, PinnedFolder, Volume, VolumeKind, VolumeSpace, user_known_folders};
use crate::location::{Location, percent_decode};

pub(super) fn known_folders() -> Vec<(KnownFolder, PathBuf)> {
    user_known_folders()
}

pub(super) fn pinned_folders() -> Vec<PinnedFolder> {
    let Some(base) = BaseDirs::new() else {
        return Vec::new();
    };
    let mut pinned = Vec::new();
    for file in [
        base.config_dir().join("gtk-3.0/bookmarks"),
        base.home_dir().join(".gtk-bookmarks"),
    ] {
        if let Ok(text) = fs::read_to_string(&file) {
            pinned.extend(parse_gtk_bookmarks(&text));
            break;
        }
    }
    if let Ok(text) = fs::read_to_string(base.data_dir().join("user-places.xbel")) {
        pinned.extend(parse_kde_places(&text));
    }
    pinned.retain(|folder| folder.path.is_dir());
    pinned
}

/// Local folders in a GTK bookmarks file: one `URI [label]` per line.
fn parse_gtk_bookmarks(text: &str) -> Vec<PinnedFolder> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let (uri, label) = match line.split_once(' ') {
                Some((uri, label)) => (uri, Some(label.trim().to_owned())),
                None => (line, None),
            };
            Some(PinnedFolder {
                path: file_uri_path(uri)?,
                label,
            })
        })
        .collect()
}

/// Local folders a person added to KDE's places. System entries (home,
/// trash, network) and hidden ones are left out: the pane lists the
/// account's folders on its own.
fn parse_kde_places(text: &str) -> Vec<PinnedFolder> {
    let mut pinned = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<bookmark ") {
        let after = &rest[start..];
        let end = after.find("</bookmark>").unwrap_or(after.len());
        let block = &after[..end];
        rest = &after[end..];
        let system = element_text(block, "isSystemItem").as_deref() == Some("true");
        let hidden = element_text(block, "IsHidden").as_deref() == Some("true");
        if system || hidden {
            continue;
        }
        let Some(path) = attribute(block, "href").and_then(|href| file_uri_path(&href)) else {
            continue;
        };
        pinned.push(PinnedFolder {
            path,
            label: element_text(block, "title"),
        });
    }
    pinned
}

fn attribute(block: &str, name: &str) -> Option<String> {
    let marker = format!("{name}=\"");
    let start = block.find(&marker)? + marker.len();
    let end = block[start..].find('"')?;
    Some(unescape_xml(&block[start..start + end]))
}

fn element_text(block: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let start = block.find(&open)? + open.len();
    let end = block[start..].find(&format!("</{name}>"))?;
    Some(unescape_xml(block[start..start + end].trim()))
}

fn unescape_xml(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The local path a `file://` URI names. Other schemes (sftp, smb, the
/// desktop's virtual folders) are not folders this dialog can read.
fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    rest.starts_with('/')
        .then(|| percent_decode(rest))
        .flatten()
        .map(PathBuf::from)
}

/// Top-level folders that hold the system's own mounts, which a file
/// browser does not list as drives. Removable media under `/run/media` stay.
const SYSTEM_MOUNT_ROOTS: [&str; 16] = [
    "/boot", "/efi", "/proc", "/sys", "/dev", "/run", "/snap", "/var", "/nix", "/gnu", "/tmp",
    "/usr", "/etc", "/opt", "/home", "/root",
];

fn is_shown_mount(mount: &Path) -> bool {
    if mount == Path::new("/") || mount.starts_with("/run/media") {
        return true;
    }
    !SYSTEM_MOUNT_ROOTS
        .iter()
        .any(|root| mount.starts_with(root))
}

fn volume_kind(mount: &Path, file_system: &str, removable: bool) -> VolumeKind {
    match file_system {
        _ if mount == Path::new("/") => VolumeKind::System,
        "iso9660" | "udf" => VolumeKind::Optical,
        "nfs" | "nfs4" | "cifs" | "smb3" | "smbfs" | "sshfs" | "fuse.sshfs" | "9p" | "davfs" => {
            VolumeKind::Network
        }
        _ if removable => VolumeKind::Removable,
        _ => VolumeKind::Fixed,
    }
}

pub(super) fn volumes() -> Vec<Volume> {
    let labels = device_labels();
    let disks = Disks::new_with_refreshed_list();
    // A device mounted more than once (btrfs subvolumes, bind mounts) is one
    // drive; its shortest mount point stands for it.
    let mut by_device: HashMap<PathBuf, Volume> = HashMap::new();
    for disk in disks.list() {
        let mount = disk.mount_point();
        if !is_shown_mount(mount) {
            continue;
        }
        let device = PathBuf::from(disk.name());
        let device = fs::canonicalize(&device).unwrap_or(device);
        let file_system = disk.file_system().to_string_lossy();
        let kind = volume_kind(mount, &file_system, disk.is_removable());
        // Removable media mount under their label, so the folder name is
        // the label when udev has none.
        let label = labels.get(&device).cloned().or_else(|| {
            (mount.starts_with("/run/media") || mount.starts_with("/media"))
                .then(|| {
                    mount
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                })
                .flatten()
        });
        let volume = Volume {
            location: Location::file_system(mount),
            label,
            kind,
            space: (disk.total_space() > 0).then(|| VolumeSpace {
                total: disk.total_space(),
                available: disk.available_space(),
            }),
        };
        let shorter = by_device
            .get(&device)
            .is_none_or(|kept| kept.location.path.as_os_str().len() > mount.as_os_str().len());
        if shorter {
            by_device.insert(device, volume);
        }
    }
    let mut volumes: Vec<_> = by_device.into_values().collect();
    volumes.sort_by(|a, b| {
        (a.kind != VolumeKind::System, &a.location.path)
            .cmp(&(b.kind != VolumeKind::System, &b.location.path))
    });
    volumes
}

/// Filesystem labels by the device that carries them.
fn device_labels() -> HashMap<PathBuf, String> {
    let Ok(entries) = fs::read_dir("/dev/disk/by-label") else {
        return HashMap::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let device = fs::canonicalize(entry.path()).ok()?;
            let label = decode_udev_escapes(&entry.file_name().to_string_lossy());
            Some((device, label))
        })
        .collect()
}

/// udev writes characters a link name cannot hold as `\xNN`.
fn decode_udev_escapes(name: &str) -> String {
    let mut bytes = Vec::with_capacity(name.len());
    let raw = name.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'\\'
            && raw.get(index + 1) == Some(&b'x')
            && let Some(byte) = name
                .get(index + 2..index + 4)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            bytes.push(byte);
            index += 4;
            continue;
        }
        bytes.push(raw[index]);
        index += 1;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gtk_bookmarks_keep_local_folders_and_labels() {
        let text = "file:///home/rune/Music\n\
                    file:///srv/My%20Media Media server\n\
                    sftp://nas/share NAS\n\
                    \n";
        let pinned: Vec<_> = parse_gtk_bookmarks(text)
            .into_iter()
            .map(|folder| (folder.path, folder.label))
            .collect();
        assert_eq!(
            pinned,
            [
                (PathBuf::from("/home/rune/Music"), None),
                (
                    PathBuf::from("/srv/My Media"),
                    Some("Media server".to_owned())
                ),
            ]
        );
    }

    #[test]
    fn kde_places_skip_system_and_hidden_entries() {
        let text = r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel>
 <bookmark href="file:///home/rune">
  <title>Home</title>
  <info><metadata owner="http://www.kde.org"><isSystemItem>true</isSystemItem></metadata></info>
 </bookmark>
 <bookmark href="file:///srv/Rock%20%26%20Roll">
  <title>Rock &amp; Roll</title>
  <info><metadata owner="http://www.kde.org"><ID>1</ID></metadata></info>
 </bookmark>
 <bookmark href="file:///srv/old">
  <title>Old</title>
  <info><metadata owner="http://www.kde.org"><IsHidden>true</IsHidden></metadata></info>
 </bookmark>
 <bookmark href="remote:/">
  <title>Network</title>
 </bookmark>
</xbel>"#;
        let pinned: Vec<_> = parse_kde_places(text)
            .into_iter()
            .map(|folder| (folder.path, folder.label))
            .collect();
        assert_eq!(
            pinned,
            [(
                PathBuf::from("/srv/Rock & Roll"),
                Some("Rock & Roll".to_owned())
            )]
        );
    }

    #[test]
    fn system_mounts_are_not_drives() {
        for shown in [
            "/",
            "/run/media/rune/USB",
            "/media/cdrom",
            "/mnt/data",
            "/data",
        ] {
            assert!(is_shown_mount(Path::new(shown)), "{shown}");
        }
        for hidden in [
            "/boot",
            "/boot/efi",
            "/home",
            "/nix/store",
            "/run/user/1000",
            "/var/lib/docker",
        ] {
            assert!(!is_shown_mount(Path::new(hidden)), "{hidden}");
        }
    }

    #[test]
    fn volume_kinds_follow_mount_and_file_system() {
        assert_eq!(
            volume_kind(Path::new("/"), "ext4", false),
            VolumeKind::System
        );
        assert_eq!(
            volume_kind(Path::new("/media/cd"), "iso9660", true),
            VolumeKind::Optical
        );
        assert_eq!(
            volume_kind(Path::new("/mnt/nas"), "nfs4", false),
            VolumeKind::Network
        );
        assert_eq!(
            volume_kind(Path::new("/run/media/u/S"), "exfat", true),
            VolumeKind::Removable
        );
        assert_eq!(
            volume_kind(Path::new("/data"), "xfs", false),
            VolumeKind::Fixed
        );
    }

    #[test]
    fn udev_escapes_decode() {
        assert_eq!(decode_udev_escapes(r"My\x20Disk"), "My Disk");
        assert_eq!(decode_udev_escapes(r"\xe9\x9f\xb3\xe4\xb9\x90"), "音乐");
        assert_eq!(decode_udev_escapes(r"odd\x2"), r"odd\x2");
    }
}
