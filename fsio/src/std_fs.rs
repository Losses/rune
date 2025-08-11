use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::fs;
use walkdir::WalkDir;

use super::{FileIo, FileIoError, FileStream, FsNode};

pub(crate) struct StdFsIo;

impl StdFsIo {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[async_trait]
impl FileIo for StdFsIo {
    async fn open(&self, path: &Path, open_mode: &str) -> Result<Box<dyn FileStream>, FileIoError> {
        let mut options = fs::OpenOptions::new();
        options.read(open_mode.contains('r'));
        options.write(open_mode.contains('w'));
        options.append(open_mode.contains('a'));
        options.truncate(open_mode.contains('t'));
        options.create(true);
        let file = options.open(path).await?;
        Ok(Box::new(file.into_std().await))
    }

    async fn read(&self, path: &Path) -> Result<Vec<u8>, FileIoError> {
        fs::read(path).await.map_err(FileIoError::Io)
    }

    async fn write(&self, path: &Path, contents: &[u8]) -> Result<(), FileIoError> {
        fs::write(path, contents).await.map_err(FileIoError::Io)
    }

    async fn create_dir(&self, parent: &Path, name: &str) -> Result<PathBuf, FileIoError> {
        let new_path = parent.join(name);
        fs::create_dir(&new_path).await?;
        Ok(new_path)
    }

    async fn create_dir_all(&self, path: &Path) -> Result<(), FileIoError> {
        fs::create_dir_all(path).await.map_err(FileIoError::Io)
    }

    async fn read_dir(&self, path: &Path) -> Result<Vec<FsNode>, FileIoError> {
        let mut entries = fs::read_dir(path).await?;
        let mut nodes = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let metadata = entry.metadata().await?;
            nodes.push(FsNode {
                path,
                is_dir: metadata.is_dir(),
                is_file: metadata.is_file(),
                size: metadata.len(),
            });
        }
        Ok(nodes)
    }

    async fn remove_file(&self, path: &Path) -> Result<(), FileIoError> {
        fs::remove_file(path).await.map_err(FileIoError::Io)
    }

    async fn remove_dir_all(&self, path: &Path) -> Result<(), FileIoError> {
        fs::remove_dir_all(path).await.map_err(FileIoError::Io)
    }

    async fn walk_dir(&self, path: &Path, follow_links: bool) -> Result<Vec<FsNode>, FileIoError> {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            WalkDir::new(path)
                .follow_links(follow_links)
                .into_iter()
                .filter_map(|e| e.ok())
                .map(|entry| {
                    let path = entry.path().to_path_buf();
                    let metadata = entry.metadata().map_err(|e| FileIoError::Io(e.into()))?;
                    Ok(FsNode {
                        path,
                        is_dir: metadata.is_dir(),
                        is_file: metadata.is_file(),
                        size: metadata.len(),
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .await
        .unwrap()
    }

    async fn exists(&self, path: &Path) -> Result<bool, FileIoError> {
        Ok(fs::try_exists(path).await?)
    }

    async fn is_file(&self, path: &Path) -> Result<bool, FileIoError> {
        let metadata = fs::metadata(path).await?;
        Ok(metadata.is_file())
    }

    async fn is_dir(&self, path: &Path) -> Result<bool, FileIoError> {
        let metadata = fs::metadata(path).await?;
        Ok(metadata.is_dir())
    }
}
