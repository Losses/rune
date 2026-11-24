//! The places a platform's own file browser lists in its navigation pane:
//! folders the person pinned, the well-known folders of their account, and
//! the volumes the device can reach.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::location::Location;

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use apple as platform;
#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
use android as platform;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as platform;

/// A folder every account has, which the dialog names in its own language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnownFolder {
    Home,
    Desktop,
    Documents,
    Downloads,
    Music,
    Pictures,
    Videos,
}

/// How the navigation pane names a place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceName {
    /// One of the account's well-known folders.
    Known(KnownFolder),
    /// The name the system gives a pinned folder.
    Custom(String),
}

/// A folder in the navigation pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub location: Location,
    pub name: PlaceName,
}

/// What kind of storage a volume is, which decides its icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeKind {
    /// The volume the system runs from.
    System,
    Fixed,
    Removable,
    Optical,
    Network,
}

/// How much a volume holds, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeSpace {
    pub total: u64,
    pub available: u64,
}

/// A drive, partition or granted storage root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    pub location: Location,
    /// The label the volume carries, if it has one.
    pub label: Option<String>,
    pub kind: VolumeKind,
    pub space: Option<VolumeSpace>,
}

/// Everything the navigation pane lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Places {
    /// Folders pinned in the system's file browser, in its order.
    pub pinned: Vec<Place>,
    /// The account's well-known folders that exist.
    pub folders: Vec<Place>,
    pub volumes: Vec<Volume>,
}

/// A folder the system lists as pinned, with the name it gives it.
pub(crate) struct PinnedFolder {
    pub path: PathBuf,
    pub label: Option<String>,
}

/// The navigation pane's places as this system knows them. A source that
/// cannot be read contributes nothing.
pub fn system_places() -> Places {
    let known = platform::known_folders();
    let pinned = platform::pinned_folders();
    Places {
        pinned: name_pinned(pinned, &known),
        folders: known
            .into_iter()
            .map(|(folder, path)| Place {
                location: Location::file_system(path),
                name: PlaceName::Known(folder),
            })
            .collect(),
        volumes: platform::volumes(),
    }
}

/// Pinned folders named the way the pane shows them: a well-known folder by
/// its kind, any other by the system's label or its folder name. A folder
/// pinned twice is listed once.
fn name_pinned(pinned: Vec<PinnedFolder>, known: &[(KnownFolder, PathBuf)]) -> Vec<Place> {
    let mut seen = HashSet::new();
    pinned
        .into_iter()
        .filter(|folder| seen.insert(folder.path.clone()))
        .map(|folder| {
            let name = match known.iter().find(|(_, path)| *path == folder.path) {
                Some((kind, _)) => PlaceName::Known(*kind),
                None => PlaceName::Custom(
                    folder
                        .label
                        .filter(|label| !label.trim().is_empty())
                        .unwrap_or_else(|| folder_name(&folder.path)),
                ),
            };
            Place {
                location: Location::file_system(folder.path),
                name,
            }
        })
        .collect()
}

/// The last component of `path`, or the whole path for a root.
fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// The account's well-known folders that exist, as the platform's user
/// directory settings name them.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn user_known_folders() -> Vec<(KnownFolder, PathBuf)> {
    let Some(dirs) = directories::UserDirs::new() else {
        return Vec::new();
    };
    let home = dirs.home_dir().to_path_buf();
    let mut folders = vec![(KnownFolder::Home, Some(home.clone()))];
    folders.extend(
        [
            (KnownFolder::Desktop, dirs.desktop_dir()),
            (KnownFolder::Documents, dirs.document_dir()),
            (KnownFolder::Downloads, dirs.download_dir()),
            (KnownFolder::Music, dirs.audio_dir()),
            (KnownFolder::Pictures, dirs.picture_dir()),
            (KnownFolder::Videos, dirs.video_dir()),
        ]
        .map(|(kind, path)| (kind, path.map(Path::to_path_buf))),
    );
    folders
        .into_iter()
        .filter_map(|(kind, path)| path.map(|path| (kind, path)))
        // An unset XDG directory falls back to home itself; listing home
        // twice under two names would only confuse.
        .filter(|(kind, path)| *kind == KnownFolder::Home || *path != home)
        .filter(|(_, path)| path.is_dir())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_folders_take_known_names_labels_or_folder_names() {
        let known = vec![(KnownFolder::Music, PathBuf::from("/home/rune/Music"))];
        let pinned = vec![
            PinnedFolder {
                path: "/home/rune/Music".into(),
                label: Some("My tunes".into()),
            },
            PinnedFolder {
                path: "/srv/media".into(),
                label: Some("Media server".into()),
            },
            PinnedFolder {
                path: "/srv/rips".into(),
                label: Some("  ".into()),
            },
            PinnedFolder {
                path: "/srv/media".into(),
                label: None,
            },
        ];

        let names: Vec<_> = name_pinned(pinned, &known)
            .into_iter()
            .map(|place| place.name)
            .collect();
        assert_eq!(
            names,
            [
                PlaceName::Known(KnownFolder::Music),
                PlaceName::Custom("Media server".into()),
                PlaceName::Custom("rips".into()),
            ]
        );
    }
}
