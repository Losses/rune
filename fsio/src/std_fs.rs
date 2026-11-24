use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::fs;
use walkdir::WalkDir;

use super::{
    DirEntryDetails, DirWalk, EntryKind, FileIo, FileIoError, FileStream, FsNode, OpenMode,
    StorageLocation,
};
use std::fs as std_fs;
use std::io::Error;
use std::io::ErrorKind;
use std::time as std_time;
use tokio::task;

/// The canonical form of `path` with its metadata, or `InvalidPath` when that
/// form is not valid Unicode.
fn canonical_node(path: &Path) -> Result<FsNode, FileIoError> {
    let path = dunce::canonicalize(path).map_err(FileIoError::Io)?;
    let metadata = std_fs::metadata(&path)?;
    Ok(FsNode {
        filename: path
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or(FileIoError::InvalidPath)?
            .to_string(),
        raw_path: path.to_str().ok_or(FileIoError::InvalidPath)?.to_string(),
        path,
        is_dir: metadata.is_dir(),
        is_file: metadata.is_file(),
        size: metadata.len(),
    })
}

/// [`canonical_node`] on a blocking thread, for async callers.
async fn canonical_node_async(path: &Path) -> Result<FsNode, FileIoError> {
    let path = path.to_path_buf();
    task::spawn_blocking(move || canonical_node(&path))
        .await
        .map_err(|error| FileIoError::Io(Error::other(error)))?
}

/// The entries of `dir` with their details. A link counts as what it points
/// to; one whose target is gone is listed as [`EntryKind::Other`]. An entry
/// removed while the directory is read is left out.
fn list_details(dir: &Path) -> Result<Vec<DirEntryDetails>, FileIoError> {
    let mut details = Vec::new();
    for entry in std_fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            log::warn!("Skipped {path:?}: its path is not valid Unicode");
            continue;
        };
        if path.to_str().is_none() {
            log::warn!("Skipped {path:?}: its path is not valid Unicode");
            continue;
        }
        let own = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let target = std_fs::metadata(&path).ok();
        let shown = target.as_ref().unwrap_or(&own);
        let kind = if shown.is_dir() {
            EntryKind::Directory
        } else if shown.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        details.push(DirEntryDetails {
            hidden: is_hidden(&name, &own),
            size: if kind == EntryKind::File {
                shown.len()
            } else {
                0
            },
            modified: shown
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std_time::UNIX_EPOCH).ok())
                .map(|age| age.as_secs()),
            name,
            path,
            kind,
        });
    }
    Ok(details)
}

/// Whether the platform's file browser hides an entry: a leading dot on
/// Unix-like systems, the hidden flag on macOS and iOS as well, and the hidden
/// attribute on Windows.
fn is_hidden(name: &str, metadata: &std_fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        let _ = name;
        metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        use std::os::darwin::fs::MetadataExt;
        const UF_HIDDEN: u32 = 0x8000;
        name.starts_with('.') || metadata.st_flags() & UF_HIDDEN != 0
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "ios")))]
    {
        let _ = metadata;
        name.starts_with('.')
    }
}

pub(crate) struct StdFsIo;

impl StdFsIo {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[async_trait]
impl FileIo for StdFsIo {
    fn name(&self) -> &'static str {
        "Std"
    }

    fn storage(&self) -> StorageLocation {
        StorageLocation::Local
    }

    fn open(&self, path: &Path, open_mode: OpenMode) -> Result<Box<dyn FileStream>, FileIoError> {
        let mut options = OpenOptions::new();
        options.read(open_mode.reads());
        options.write(open_mode.writes());
        options.append(open_mode.appends());
        options.truncate(open_mode.truncates());
        options.create(open_mode.creates());

        let file = options.open(path)?;
        Ok(Box::new(file))
    }

    async fn open_async(
        &self,
        path: &Path,
        open_mode: OpenMode,
    ) -> Result<Box<dyn FileStream>, FileIoError> {
        let mut options = fs::OpenOptions::new();
        options.read(open_mode.reads());
        options.write(open_mode.writes());
        options.append(open_mode.appends());
        options.truncate(open_mode.truncates());
        options.create(open_mode.creates());

        let file = options.open(path).await?;
        Ok(Box::new(file.into_std().await))
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, FileIoError> {
        std_fs::read(path).map_err(FileIoError::Io)
    }

    fn read_to_string(&self, path: &Path) -> Result<String, FileIoError> {
        std_fs::read_to_string(path).map_err(FileIoError::Io)
    }

    async fn write(&self, path: &Path, contents: &[u8]) -> Result<(), FileIoError> {
        fs::write(path, contents).await.map_err(FileIoError::Io)
    }

    async fn write_string(&self, path: &Path, contents: &str) -> Result<(), FileIoError> {
        fs::write(path, contents).await.map_err(FileIoError::Io)
    }

    async fn create_dir(&self, parent: &Path, name: &str) -> Result<PathBuf, FileIoError> {
        let new_path = parent.join(name);
        fs::create_dir(&new_path).await?;
        Ok(new_path)
    }

    fn create_dir_all(&self, path: &Path) -> Result<(), FileIoError> {
        std_fs::create_dir_all(path).map_err(FileIoError::Io)
    }

    async fn read_dir(&self, path: &Path) -> Result<Vec<FsNode>, FileIoError> {
        let mut entries = fs::read_dir(path).await?;
        let mut nodes = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let file_name = entry.file_name();
            let (Some(filename), Some(raw_path)) = (file_name.to_str(), path.to_str()) else {
                log::warn!("Skipped {path:?}: its path is not valid Unicode");
                continue;
            };
            let metadata = entry.metadata().await?;
            nodes.push(FsNode {
                filename: filename.to_string(),
                raw_path: raw_path.to_string(),
                path,
                is_dir: metadata.is_dir(),
                is_file: metadata.is_file(),
                size: metadata.len(),
            });
        }
        Ok(nodes)
    }

    async fn read_dir_details(&self, path: &Path) -> Result<Vec<DirEntryDetails>, FileIoError> {
        let path = path.to_path_buf();
        task::spawn_blocking(move || list_details(&path))
            .await
            .map_err(|error| FileIoError::Io(Error::other(error)))?
    }

    async fn remove_file(&self, path: &Path) -> Result<(), FileIoError> {
        fs::remove_file(path).await.map_err(FileIoError::Io)
    }

    async fn rename_file(&self, from: &Path, to: &Path) -> Result<(), FileIoError> {
        if from.parent() != to.parent() || !fs::metadata(from).await?.is_file() {
            return Err(FileIoError::InvalidPath);
        }
        fs::rename(from, to).await?;
        #[cfg(unix)]
        {
            let parent = to.parent().ok_or(FileIoError::InvalidPath)?;
            let directory = fs::File::open(parent).await?;
            directory.sync_all().await?;
        }
        Ok(())
    }

    async fn rename_entry(&self, from: &Path, to: &Path) -> Result<(), FileIoError> {
        if from.parent() != to.parent() {
            return Err(FileIoError::InvalidPath);
        }
        // rename replaces an existing file on Unix, so a taken name is
        // checked first. Another program creating the name in between is
        // the one case this cannot rule out.
        if fs::symlink_metadata(to).await.is_ok() {
            return Err(FileIoError::Io(Error::from(ErrorKind::AlreadyExists)));
        }
        fs::rename(from, to).await.map_err(FileIoError::Io)
    }

    async fn remove_dir_all(&self, path: &Path) -> Result<(), FileIoError> {
        fs::remove_dir_all(path).await.map_err(FileIoError::Io)
    }

    fn walk_dir(&self, path: &Path, follow_links: bool) -> Result<DirWalk, FileIoError> {
        let mut walk = DirWalk::default();
        let mut entries = WalkDir::new(path).follow_links(follow_links).into_iter();
        while let Some(entry) = entries.next() {
            let entry = match entry {
                Ok(entry) => entry,
                // A root that is missing or cannot be read would otherwise
                // look like an empty folder.
                Err(error) if error.depth() == 0 => return Err(FileIoError::Io(error.into())),
                Err(_) => continue,
            };
            let (Some(filename), Some(raw_path)) =
                (entry.file_name().to_str(), entry.path().to_str())
            else {
                // walkdir enters a root that links to a directory even without
                // follow_links, and that entry does not report itself as one.
                if entry.depth() == 0 || entry.file_type().is_dir() {
                    entries.skip_current_dir();
                }
                walk.skipped.push(entry.into_path());
                continue;
            };
            let metadata = entry.metadata().map_err(|e| FileIoError::Io(e.into()))?;
            walk.nodes.push(FsNode {
                filename: filename.to_string(),
                raw_path: raw_path.to_string(),
                path: entry.path().to_path_buf(),
                is_dir: metadata.is_dir(),
                is_file: metadata.is_file(),
                size: metadata.len(),
            });
        }
        Ok(walk)
    }

    fn exists(&self, path: &Path) -> Result<bool, FileIoError> {
        Ok(std_fs::exists(path)?)
    }

    fn entry_exists(&self, path: &Path) -> Result<bool, FileIoError> {
        match std_fs::symlink_metadata(path) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    async fn is_file(&self, path: &Path) -> Result<bool, FileIoError> {
        let metadata = fs::metadata(path).await?;
        Ok(metadata.is_file())
    }

    async fn is_dir(&self, path: &Path) -> Result<bool, FileIoError> {
        let metadata = fs::metadata(path).await?;
        Ok(metadata.is_dir())
    }

    fn canonicalize_path(&self, path: &Path) -> Result<PathBuf, FileIoError> {
        dunce::canonicalize(path).map_err(FileIoError::Io)
    }

    fn canonicalize_path_str(&self, path: &str) -> Result<PathBuf, FileIoError> {
        dunce::canonicalize(path).map_err(FileIoError::Io)
    }

    fn canonicalize(&self, path: &Path) -> Result<FsNode, FileIoError> {
        canonical_node(path)
    }

    fn canonicalize_str(&self, path: &str) -> Result<FsNode, FileIoError> {
        let path = self.canonicalize_path_str(path)?;
        let metadata = std_fs::metadata(&path)?;
        Ok(FsNode {
            filename: path
                .file_name()
                .and_then(OsStr::to_str)
                .ok_or(FileIoError::InvalidPath)?
                .to_string(),
            raw_path: path.to_str().ok_or(FileIoError::InvalidPath)?.to_string(),
            path,
            is_dir: metadata.is_dir(),
            is_file: metadata.is_file(),
            size: metadata.len(),
        })
    }

    fn modified_time(&self, path: &Path) -> Result<u64, FileIoError> {
        let modified = std_fs::metadata(path)?
            .modified()?
            .duration_since(std_time::UNIX_EPOCH)
            .unwrap_or_default();
        Ok(modified.as_secs())
    }

    async fn ensure_file(&self, path: &Path) -> Result<FsNode, FileIoError> {
        if !fs::try_exists(path).await? {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).await?;
            }
            self.write(path, &[]).await?;
        }
        canonical_node_async(path).await
    }

    async fn ensure_directory(&self, path: &Path) -> Result<FsNode, FileIoError> {
        if !fs::try_exists(path).await? {
            fs::create_dir_all(path).await?;
        }
        canonical_node_async(path).await
    }
}
