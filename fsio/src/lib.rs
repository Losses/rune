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
}

#[derive(Debug, Clone)]
pub struct FsNode {
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_file: bool,
    pub size: u64,
}

pub trait FileStream: Read + Write + Seek + Send + Sync {}
impl<T: Read + Write + Seek + Send + Sync> FileStream for T {}

#[async_trait]
pub trait FileIo: Send + Sync {
    fn open(&self, path: &Path, open_mode: &str) -> Result<Box<dyn FileStream>, FileIoError>;
    async fn open_async(
        &self,
        path: &Path,
        open_mode: &str,
    ) -> Result<Box<dyn FileStream>, FileIoError>;
    async fn read(&self, path: &Path) -> Result<Vec<u8>, FileIoError>;
    async fn write(&self, path: &Path, contents: &[u8]) -> Result<(), FileIoError>;
    async fn create_dir(&self, parent: &Path, name: &str) -> Result<PathBuf, FileIoError>;
    async fn create_dir_all(&self, path: &Path) -> Result<(), FileIoError>;
    async fn read_dir(&self, path: &Path) -> Result<Vec<FsNode>, FileIoError>;
    async fn remove_file(&self, path: &Path) -> Result<(), FileIoError>;
    async fn remove_dir_all(&self, path: &Path) -> Result<(), FileIoError>;
    async fn walk_dir(&self, path: &Path, follow_links: bool) -> Result<Vec<FsNode>, FileIoError>;
    async fn exists(&self, path: &Path) -> Result<bool, FileIoError>;
    async fn is_file(&self, path: &Path) -> Result<bool, FileIoError>;
    async fn is_dir(&self, path: &Path) -> Result<bool, FileIoError>;
    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FileIoError>;
}

pub struct FsIo {
    inner: Arc<dyn FileIo>,
}

impl FsIo {
    #[cfg(target_os = "android")]
    pub async fn new(db_path: &Path, root_uri: &str) -> Result<Self, FileIoError> {
        let inner = AndroidFsIo::new(db_path, root_uri).await?;
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
}

#[cfg(not(target_os = "android"))]
impl Default for FsIo {
    fn default() -> Self {
        Self::new()
    }
}

impl std::ops::Deref for FsIo {
    type Target = dyn FileIo;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

#[cfg(not(target_os = "android"))]
mod std_fs;
#[cfg(not(target_os = "android"))]
use std_fs::StdFsIo;

#[cfg(target_os = "android")]
mod android_fs;
#[cfg(target_os = "android")]
use android_fs::AndroidFsIo;

mod noop_fs;
use noop_fs::NoOpFsIo;
