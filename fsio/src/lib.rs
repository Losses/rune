use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FileIoError {
    #[error("path not found: {0}")]
    PathNotFound(String),
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

use std::io::{Read, Write};

pub trait FileStream: Read + Write + Send {}
impl<T: Read + Write + Send> FileStream for T {}

#[async_trait]
pub trait FileIo: Send + Sync {
    async fn open(
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
