use std::fs::File;
use std::{
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use async_trait::async_trait;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FileIoError {
    #[error("path not found: {0}")]
    PathNotFound(String),
    #[error("invalid path")]
    InvalidPath,
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
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

#[derive(Debug, Clone)]
pub struct FsNode {
    pub filename: String,
    pub path: PathBuf,
    pub raw_path: String,
    pub is_dir: bool,
    pub is_file: bool,
    pub size: u64,
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

impl FileStream for std::fs::File {
    fn sync_all(&mut self) -> Result<(), FileIoError> {
        File::sync_all(self).map_err(FileIoError::Io)
    }
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
    async fn read_dir(&self, path: &Path) -> Result<Vec<FsNode>, FileIoError>;
    async fn remove_file(&self, path: &Path) -> Result<(), FileIoError>;
    /// Rename a file within its current directory. Backends must finish the
    /// rename before reporting success; callers can then register a verified file.
    async fn rename_file(&self, _from: &Path, _to: &Path) -> Result<(), FileIoError> {
        Err(FileIoError::NotSupported("file rename".into()))
    }
    async fn remove_dir_all(&self, path: &Path) -> Result<(), FileIoError>;
    fn walk_dir(&self, path: &Path, follow_links: bool) -> Result<Vec<FsNode>, FileIoError>;
    fn exists(&self, path: &Path) -> Result<bool, FileIoError>;
    async fn is_file(&self, path: &Path) -> Result<bool, FileIoError>;
    async fn is_dir(&self, path: &Path) -> Result<bool, FileIoError>;
    fn canonicalize_path(&self, path: &Path) -> Result<PathBuf, FileIoError>;
    fn canonicalize_path_str(&self, path: &str) -> Result<PathBuf, FileIoError>;
    fn canonicalize(&self, path: &Path) -> Result<FsNode, FileIoError>;
    fn canonicalize_str(&self, path: &str) -> Result<FsNode, FileIoError>;
    /// Seconds since the UNIX epoch of the last modification.
    fn modified_time(&self, path: &Path) -> Result<u64, FileIoError>;
    /// Stable backend-provided signal used to skip unchanged files during scans.
    /// Backends without a stronger signal use modification time. `None` means
    /// the backend offers no signal, so the file must be treated as changed.
    fn change_token(&self, path: &Path) -> Result<Option<String>, FileIoError> {
        self.modified_time(path).map(|time| Some(time.to_string()))
    }
    async fn ensure_file(&self, path: &Path) -> Result<FsNode, FileIoError>;
    async fn ensure_directory(&self, path: &Path) -> Result<FsNode, FileIoError>;

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
    pub fn new_opendal(
        config: OpenDalConfig,
        handle: tokio::runtime::Handle,
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

impl std::fmt::Debug for FsIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FsIo")
            .field("inner", &self.inner.name())
            .finish()
    }
}

impl std::ops::Deref for FsIo {
    type Target = dyn FileIo;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

mod std_fs;
use std_fs::StdFsIo;

mod opendal_fs;
pub use opendal_fs::OpenDalConfig;
mod opendal_object_store;
pub use opendal_object_store::{ObjectStoreCapabilities, OpenDalObjectStore};
#[cfg(feature = "test-fixtures")]
pub mod test_fixtures;
use opendal_fs::OpenDalFsIo;

#[cfg(target_os = "android")]
mod android_fs;
#[cfg(target_os = "android")]
use android_fs::AndroidFsIo;

mod noop_fs;
use noop_fs::NoOpFsIo;

pub mod self_test;
