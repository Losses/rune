use std::io::ErrorKind;
use std::path::Path;

use fsio::{DirEntryDetails, FileIoError, FsIo};
use thiserror::Error;

/// Why a folder could not be shown. These are the failures a person can run
/// into while browsing, so each one gets its own message in the dialog.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BrowseError {
    #[error("no such file or folder")]
    NotFound,
    #[error("the location is a file")]
    NotADirectory,
    #[error("permission denied")]
    PermissionDenied,
    #[error("an item with that name already exists")]
    AlreadyExists,
    #[error("the name cannot be used here")]
    InvalidName,
    #[error("storage unavailable: {0}")]
    Unavailable(String),
}

impl From<FileIoError> for BrowseError {
    fn from(error: FileIoError) -> Self {
        match error {
            FileIoError::PathNotFound(_) => Self::NotFound,
            FileIoError::PermissionDenied(_) => Self::PermissionDenied,
            FileIoError::Io(error) => match error.kind() {
                ErrorKind::NotFound => Self::NotFound,
                ErrorKind::PermissionDenied => Self::PermissionDenied,
                ErrorKind::NotADirectory => Self::NotADirectory,
                ErrorKind::AlreadyExists => Self::AlreadyExists,
                ErrorKind::InvalidFilename => Self::InvalidName,
                _ => Self::Unavailable(error.to_string()),
            },
            other => Self::Unavailable(other.to_string()),
        }
    }
}

/// The entries of the folder at `path`, read through `fs`.
pub async fn list_directory(fs: &FsIo, path: &Path) -> Result<Vec<DirEntryDetails>, BrowseError> {
    match fs.read_dir_details(path).await {
        Ok(entries) => Ok(entries),
        Err(error) => {
            let error = BrowseError::from(error);
            // Some platforms report a file opened as a folder as a generic
            // failure; asking the backend tells the cases apart.
            if matches!(error, BrowseError::Unavailable(_))
                && fs.is_file(path).await.unwrap_or(false)
            {
                return Err(BrowseError::NotADirectory);
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use fsio::EntryKind;

    use super::*;

    #[tokio::test]
    async fn listing_reports_why_a_folder_cannot_be_shown() {
        let dir = tempfile::tempdir().unwrap();
        let fsio = FsIo::new();
        fs::write(dir.path().join("track.flac"), b"flac").unwrap();
        fs::create_dir(dir.path().join("Albums")).unwrap();

        let mut entries = list_directory(&fsio, dir.path()).await.unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let kinds: Vec<_> = entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                ("Albums", EntryKind::Directory),
                ("track.flac", EntryKind::File)
            ]
        );

        assert_eq!(
            list_directory(&fsio, &dir.path().join("missing")).await,
            Err(BrowseError::NotFound)
        );
        assert_eq!(
            list_directory(&fsio, &dir.path().join("track.flac")).await,
            Err(BrowseError::NotADirectory)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn listing_a_folder_without_read_permission_is_denied() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads anything, so the check only means something for others.
        let readable = fs::read_dir(&locked).is_ok();

        let result = list_directory(&FsIo::new(), &locked).await;
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        if !readable {
            assert_eq!(result, Err(BrowseError::PermissionDenied));
        }
    }
}
