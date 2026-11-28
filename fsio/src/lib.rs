use std::fs::File;
use std::{
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use async_trait::async_trait;
use std::fmt;
use std::fs;
use std::io;
use std::ops;
use std::time;
use thiserror::Error;
use tokio::runtime;

#[derive(Error, Debug)]
pub enum FileIoError {
    #[error("path not found: {0}")]
    PathNotFound(String),
    #[error("invalid path")]
    InvalidPath,
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("database error: {0}")]
    Database(String),
    #[error("Android SAF error: {0}")]
    Saf(String),
    #[error("operation not supported: {0}")]
    NotSupported(String),
    #[error("unknown error")]
    Unknown,
    #[error("remote storage error: {0}")]
    Remote(String),
}

/// Whether Android's document provider failed to access a granted document or tree.
pub fn is_document_provider_error(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<FileIoError>(),
        Some(FileIoError::Saf(_))
    )
}

#[derive(Debug, Clone)]
pub struct FsNode {
    pub filename: String,
    pub path: PathBuf,
    pub raw_path: String,
    pub is_dir: bool,
    pub is_file: bool,
    pub size: u64,
}

/// What an entry listed by [`FileIo::read_dir_details`] is, after following links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    File,
    /// Neither: a link whose target is missing, a socket, a device.
    Other,
}

/// One entry of a directory with what a file browser shows about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntryDetails {
    pub name: String,
    pub path: PathBuf,
    pub kind: EntryKind,
    /// Bytes in a file; zero for anything else.
    pub size: u64,
    /// Seconds since the UNIX epoch of the last modification, when the backend
    /// reports it while listing.
    pub modified: Option<u64>,
    /// Whether file browsers on this platform leave the entry out unless asked
    /// to show hidden items.
    pub hidden: bool,
}

impl DirEntryDetails {
    /// The details a plain [`FsNode`] carries: no modification time, and
    /// hidden when the name starts with a dot.
    pub fn from_node(node: FsNode) -> Self {
        let kind = match (node.is_dir, node.is_file) {
            (true, _) => EntryKind::Directory,
            (false, true) => EntryKind::File,
            (false, false) => EntryKind::Other,
        };
        Self {
            hidden: node.filename.starts_with('.'),
            size: if kind == EntryKind::File {
                node.size
            } else {
                0
            },
            name: node.filename,
            path: node.path,
            kind,
            modified: None,
        }
    }
}

/// What [`FileIo::walk_dir`] found under a directory.
#[derive(Debug, Clone, Default)]
pub struct DirWalk {
    /// Every entry under the directory. Whether the directory itself is listed
    /// depends on the backend: the standard file system lists it, OpenDAL does not.
    pub nodes: Vec<FsNode>,
    /// Entries left out because their path is not valid Unicode. An [`FsNode`]
    /// holds names as text, and a lossy copy could name a different file.
    /// Nothing under a skipped directory is walked.
    pub skipped: Vec<PathBuf>,
}

/// Where a backend keeps its files, which decides how reads fail and how streams are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageLocation {
    /// On this device: the file system or a document provider on it.
    Local,
    /// Behind a network connection that can drop or reject access.
    Remote,
}

/// How a file is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenMode {
    /// Read an existing file.
    Read,
    /// Write, creating the file if missing, keeping what it holds.
    Write,
    /// Write from the start, creating the file if missing and emptying it.
    WriteTruncate,
    /// Write at the end, creating the file if missing.
    Append,
    /// Read and write an existing file, creating it if missing.
    ReadWrite,
}

impl OpenMode {
    /// The mode as the platform APIs that take one spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "r",
            Self::Write => "w",
            Self::WriteTruncate => "wt",
            Self::Append => "a",
            Self::ReadWrite => "rw",
        }
    }

    pub fn reads(self) -> bool {
        matches!(self, Self::Read | Self::ReadWrite)
    }

    pub fn writes(self) -> bool {
        matches!(self, Self::Write | Self::WriteTruncate | Self::ReadWrite)
    }

    pub fn appends(self) -> bool {
        self == Self::Append
    }

    pub fn truncates(self) -> bool {
        self == Self::WriteTruncate
    }

    /// Whether opening creates a file that is missing.
    pub fn creates(self) -> bool {
        self != Self::Read
    }
}

pub trait FileStream: Read + Write + Seek + Send + Sync {
    /// Persist file contents and metadata before publication.
    fn sync_all(&mut self) -> Result<(), FileIoError>;
}

impl FileStream for fs::File {
    fn sync_all(&mut self) -> Result<(), FileIoError> {
        File::sync_all(self).map_err(FileIoError::Io)
    }
}

/// The longest step in which a filesystem a library is likely to be on records
/// modification times: FAT volumes store them in two-second steps.
const MODIFICATION_TIME_STEP_SECS: u64 = 2;

/// The change token for a modification time in whole seconds.
///
/// A file can change again within the step its time was recorded in without
/// that time moving, so the time vouches for nothing until the step has
/// passed. A time in the future stays a token: a file with a wrong timestamp
/// would otherwise be scanned again on every scan.
fn modification_time_token(modified_secs: u64, now_secs: u64) -> Option<String> {
    let step_has_passed = now_secs
        .checked_sub(modified_secs)
        .is_none_or(|age| age >= MODIFICATION_TIME_STEP_SECS);
    step_has_passed.then(|| modified_secs.to_string())
}

#[async_trait]
pub trait FileIo: Send + Sync {
    /// A name for logs and messages, not for deciding anything.
    fn name(&self) -> &'static str;
    fn storage(&self) -> StorageLocation;
    fn open(&self, path: &Path, open_mode: OpenMode) -> Result<Box<dyn FileStream>, FileIoError>;
    async fn open_async(
        &self,
        path: &Path,
        open_mode: OpenMode,
    ) -> Result<Box<dyn FileStream>, FileIoError>;
    fn read(&self, path: &Path) -> Result<Vec<u8>, FileIoError>;
    fn read_to_string(&self, path: &Path) -> Result<String, FileIoError>;
    async fn write(&self, path: &Path, contents: &[u8]) -> Result<(), FileIoError>;
    async fn write_string(&self, path: &Path, contents: &str) -> Result<(), FileIoError>;
    async fn create_dir(&self, parent: &Path, name: &str) -> Result<PathBuf, FileIoError>;
    fn create_dir_all(&self, path: &Path) -> Result<(), FileIoError>;
    /// The entries directly inside `path`, leaving out any whose path is not
    /// valid Unicode.
    async fn read_dir(&self, path: &Path) -> Result<Vec<FsNode>, FileIoError>;
    /// The entries directly inside `path` as a file browser shows them: their
    /// kind after following links, their modification time where the backend
    /// has it at hand, and whether the platform hides them. Like
    /// [`FileIo::read_dir`], entries whose path is not valid Unicode are left
    /// out.
    async fn read_dir_details(&self, path: &Path) -> Result<Vec<DirEntryDetails>, FileIoError> {
        Ok(self
            .read_dir(path)
            .await?
            .into_iter()
            .map(DirEntryDetails::from_node)
            .collect())
    }
    async fn remove_file(&self, path: &Path) -> Result<(), FileIoError>;
    /// Rename a file within its current directory. Backends must finish the
    /// rename before reporting success; callers can then register a verified file.
    async fn rename_file(&self, _from: &Path, _to: &Path) -> Result<(), FileIoError> {
        Err(FileIoError::NotSupported("file rename".into()))
    }
    /// Give a file or folder a new name within its folder, failing with
    /// [`io::ErrorKind::AlreadyExists`] when the new name is taken rather
    /// than replacing what is there.
    async fn rename_entry(&self, _from: &Path, _to: &Path) -> Result<(), FileIoError> {
        Err(FileIoError::NotSupported("entry rename".into()))
    }
    async fn remove_dir_all(&self, path: &Path) -> Result<(), FileIoError>;
    /// Every entry under `path`, with entries whose path is not valid Unicode
    /// reported apart in [`DirWalk::skipped`]. The standard file system fails
    /// when `path` itself is missing or cannot be read, so it is not taken for
    /// an empty folder, and leaves out entries below it that it cannot read.
    /// OpenDAL and NoOp list a missing `path` as empty, and the Android cache
    /// returns what it last cached.
    fn walk_dir(&self, path: &Path, follow_links: bool) -> Result<DirWalk, FileIoError>;
    fn exists(&self, path: &Path) -> Result<bool, FileIoError>;
    /// Checks the entry itself, including links with a missing target.
    fn entry_exists(&self, _path: &Path) -> Result<bool, FileIoError> {
        Err(FileIoError::NotSupported("directory entry lookup".into()))
    }
    async fn is_file(&self, path: &Path) -> Result<bool, FileIoError>;
    async fn is_dir(&self, path: &Path) -> Result<bool, FileIoError>;
    fn canonicalize_path(&self, path: &Path) -> Result<PathBuf, FileIoError>;
    fn canonicalize_path_str(&self, path: &str) -> Result<PathBuf, FileIoError>;
    fn canonicalize(&self, path: &Path) -> Result<FsNode, FileIoError>;
    fn canonicalize_str(&self, path: &str) -> Result<FsNode, FileIoError>;
    /// Seconds since the UNIX epoch of the last modification.
    fn modified_time(&self, path: &Path) -> Result<u64, FileIoError>;
    /// Stable backend-provided signal used to skip unchanged files during scans.
    /// Backends without a stronger signal use modification time, which is no
    /// signal until the step it was recorded in has passed. `None` means the
    /// backend offers no signal, so the file must be treated as changed.
    fn change_token(&self, path: &Path) -> Result<Option<String>, FileIoError> {
        // The clock is read after the file, so a step that it reports as
        // passed cannot be given to a later modification.
        let modified = self.modified_time(path)?;
        let now = time::SystemTime::now()
            .duration_since(time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Ok(modification_time_token(modified, now))
    }
    async fn ensure_file(&self, path: &Path) -> Result<FsNode, FileIoError>;
    async fn ensure_directory(&self, path: &Path) -> Result<FsNode, FileIoError>;

    /// The content URI the platform's document provider knows the item by,
    /// for platform services that take one, such as Android's document
    /// thumbnails. Backends addressed by plain paths have none.
    fn content_uri(&self, _path: &Path) -> Result<Option<String>, FileIoError> {
        Ok(None)
    }

    /// Refresh any internal cache from the real filesystem. Backends that
    /// read through directly (std, noop) have nothing to refresh.
    fn refresh_cache(&self) -> Result<(), FileIoError> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct FsIo {
    inner: Arc<dyn FileIo>,
}

impl FsIo {
    #[cfg(target_os = "android")]
    pub fn new(db_path: &Path, root_uri: &str) -> Result<Self, FileIoError> {
        if !root_uri.starts_with("content://") {
            return Ok(Self {
                inner: Arc::new(StdFsIo::new()),
            });
        }
        let inner = AndroidFsIo::new(db_path, root_uri)?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    #[cfg(not(target_os = "android"))]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(StdFsIo::new()),
        }
    }

    pub fn new_noop() -> Self {
        Self {
            inner: Arc::new(NoOpFsIo::new()),
        }
    }

    /// Create a read-only WebDAV or S3 filesystem using the supplied runtime.
    /// Its sync methods, and reads of the files it opens, block the calling
    /// thread until that runtime has completed the request, so call them on a
    /// thread that drives no runtime, such as a `spawn_blocking` one. The files
    /// can be dropped on any thread.
    pub fn new_opendal(
        config: OpenDalConfig,
        handle: runtime::Handle,
    ) -> Result<Self, FileIoError> {
        Ok(Self {
            inner: Arc::new(OpenDalFsIo::new(config, handle)?),
        })
    }
}

#[cfg(not(target_os = "android"))]
impl Default for FsIo {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for FsIo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FsIo")
            .field("inner", &self.inner.name())
            .finish()
    }
}

impl ops::Deref for FsIo {
    type Target = dyn FileIo;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

mod std_fs;
use std_fs::StdFsIo;

#[cfg(feature = "endpoint-url")]
mod opendal_endpoint;
#[cfg(feature = "endpoint-url")]
pub use opendal_endpoint::OpenDalEndpointError;
mod opendal_fs;
pub use opendal_fs::OpenDalConfig;
mod opendal_object_store;
pub use opendal_object_store::{ObjectStoreCapabilities, OpenDalObjectStore};
#[cfg(feature = "test-fixtures")]
pub mod failing_read;
#[cfg(feature = "test-fixtures")]
pub mod test_fixtures;
use opendal_fs::OpenDalFsIo;

#[cfg(target_os = "android")]
mod android_fs;
#[cfg(target_os = "android")]
mod android_jni;
#[cfg(target_os = "android")]
pub use android_fs::read_android_document;
#[cfg(target_os = "android")]
use android_fs::AndroidFsIo;

mod noop_fs;
use noop_fs::NoOpFsIo;

pub mod self_test;

#[cfg(test)]
mod tests {
    use super::modification_time_token;

    #[test]
    fn modification_time_is_a_token_only_once_its_step_has_passed() {
        assert_eq!(modification_time_token(100, 100), None);
        assert_eq!(modification_time_token(100, 101), None);
        assert_eq!(modification_time_token(100, 102), Some("100".to_string()));
        assert_eq!(modification_time_token(101, 100), Some("101".to_string()));
    }
}
