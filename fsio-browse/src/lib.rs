//! What a file dialog needs from the system, on top of [`fsio`]: directory
//! listings read through an [`fsio::FsIo`] backend, the places a platform's
//! own file browser shows in its navigation pane, and the thumbnails the
//! platform draws for files.
//!
//! Every function here works on each platform. What the platform cannot
//! provide comes back empty: a desktop without bookmarks has no pinned places,
//! and a file the platform cannot preview has no thumbnail.

#[cfg(target_os = "android")]
mod android_jni;
mod listing;
mod location;
mod operations;
mod places;
mod thumbnail;
#[cfg(target_os = "windows")]
mod windows_com;

pub use listing::{BrowseError, list_directory};
pub use location::{Location, Root, home_dir, normalize_lexically, resolve_typed_path};
pub use operations::{PathTarget, create_folder, inspect, rename_entry, valid_name};
pub use places::{
    KnownFolder, Place, PlaceName, Places, Volume, VolumeKind, VolumeSpace, system_places,
};
pub use thumbnail::{Thumbnail, ThumbnailSize, Thumbnailer};
