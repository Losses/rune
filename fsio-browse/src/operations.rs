//! What a file dialog changes or asks about single items: what a typed path
//! names, new folders, and new names.

use std::path::{Path, PathBuf};

use fsio::FsIo;

use crate::listing::BrowseError;

/// What a path names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathTarget {
    Folder,
    File,
    /// Nothing, in a folder that exists: where a new file can be saved.
    Missing,
    /// Nothing, and its folder does not exist either.
    Unreachable,
}

/// What `path` names, read through `fs`.
pub async fn inspect(fs: &FsIo, path: &Path) -> Result<PathTarget, BrowseError> {
    match fs.is_dir(path).await.map_err(BrowseError::from) {
        Ok(true) => Ok(PathTarget::Folder),
        Ok(false) => Ok(PathTarget::File),
        Err(BrowseError::NotFound) => {
            let parent_exists = match path.parent() {
                Some(parent) if parent.as_os_str().is_empty() => true,
                Some(parent) => fs.is_dir(parent).await.unwrap_or(false),
                None => false,
            };
            Ok(if parent_exists {
                PathTarget::Missing
            } else {
                PathTarget::Unreachable
            })
        }
        Err(error) => Err(error),
    }
}

/// A new folder called `name` in `parent`.
pub async fn create_folder(fs: &FsIo, parent: &Path, name: &str) -> Result<PathBuf, BrowseError> {
    let name = valid_name(name)?;
    fs.create_dir(parent, name).await.map_err(BrowseError::from)
}

/// `path` renamed to `name` within its folder.
pub async fn rename_entry(fs: &FsIo, path: &Path, name: &str) -> Result<PathBuf, BrowseError> {
    let name = valid_name(name)?;
    let renamed = path.with_file_name(name);
    if renamed == path {
        return Ok(renamed);
    }
    fs.rename_entry(path, &renamed)
        .await
        .map_err(BrowseError::from)?;
    Ok(renamed)
}

/// Names Windows keeps for devices, whatever their extension.
const WINDOWS_DEVICE_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// `name` without surrounding spaces, if it can name an item on this
/// platform: non-empty, other than `.` and `..`, without a path separator, and on
/// Windows without the characters and device names Windows refuses.
pub fn valid_name(name: &str) -> Result<&str, BrowseError> {
    let name = name.trim();
    let refused = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\0'])
        || cfg!(windows) && windows_refuses(name);
    if refused {
        Err(BrowseError::InvalidName)
    } else {
        Ok(name)
    }
}

fn windows_refuses(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    name.contains(['\\', '<', '>', ':', '"', '|', '?', '*'])
        || name.chars().any(char::is_control)
        || name.ends_with('.')
        || WINDOWS_DEVICE_NAMES
            .iter()
            .any(|device| device.eq_ignore_ascii_case(stem))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[tokio::test]
    async fn typed_paths_name_folders_files_and_missing_items() {
        let dir = tempfile::tempdir().unwrap();
        let fsio = FsIo::new();
        fs::write(dir.path().join("song.flac"), b"x").unwrap();

        let target = |name: &str| {
            let path = dir.path().join(name);
            let fsio = fsio.clone();
            async move { inspect(&fsio, &path).await }
        };
        assert_eq!(target("").await, Ok(PathTarget::Folder));
        assert_eq!(target("song.flac").await, Ok(PathTarget::File));
        assert_eq!(target("new.flac").await, Ok(PathTarget::Missing));
        assert_eq!(target("gone/new.flac").await, Ok(PathTarget::Unreachable));
    }

    #[tokio::test]
    async fn folders_are_created_and_items_renamed_with_valid_names() {
        let dir = tempfile::tempdir().unwrap();
        let fsio = FsIo::new();

        let created = create_folder(&fsio, dir.path(), "  Live  ").await.unwrap();
        assert_eq!(created, dir.path().join("Live"));
        assert!(created.is_dir());
        assert_eq!(
            create_folder(&fsio, dir.path(), "Live").await,
            Err(BrowseError::AlreadyExists)
        );
        assert_eq!(
            create_folder(&fsio, dir.path(), "a/b").await,
            Err(BrowseError::InvalidName)
        );

        let renamed = rename_entry(&fsio, &created, "Live 2019").await.unwrap();
        assert_eq!(renamed, dir.path().join("Live 2019"));
        assert!(renamed.is_dir() && !created.exists());
        assert_eq!(
            rename_entry(&fsio, &renamed, "..").await,
            Err(BrowseError::InvalidName)
        );
    }

    #[test]
    fn names_follow_the_platform_rules() {
        assert_eq!(valid_name(" Album "), Ok("Album"));
        for refused in ["", "   ", ".", "..", "a/b", "a\0b"] {
            assert_eq!(
                valid_name(refused),
                Err(BrowseError::InvalidName),
                "{refused:?}"
            );
        }
        assert!(windows_refuses("CON"));
        assert!(windows_refuses("nul.txt"));
        assert!(windows_refuses("what?"));
        assert!(windows_refuses("trailing."));
        assert!(!windows_refuses("Console"));
        assert!(!windows_refuses("Live (2019)"));
    }
}
