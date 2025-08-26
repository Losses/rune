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
    fn name(&self) -> &'static str {
        "Std"
    }

    fn open(&self, path: &Path, open_mode: &str) -> Result<Box<dyn FileStream>, FileIoError> {
        let mut options = std::fs::OpenOptions::new();
        options.read(open_mode.contains('r'));
        options.write(open_mode.contains('w'));
        options.append(open_mode.contains('a'));
        options.truncate(open_mode.contains('t'));

        if open_mode.contains('w') || open_mode.contains('a') || open_mode.contains('t') {
            options.create(true);
        }

        let file = options.open(path)?;
        Ok(Box::new(file))
    }

    async fn open_async(
        &self,
        path: &Path,
        open_mode: &str,
    ) -> Result<Box<dyn FileStream>, FileIoError> {
        let mut options = fs::OpenOptions::new();
        options.read(open_mode.contains('r'));
        options.write(open_mode.contains('w'));
        options.append(open_mode.contains('a'));
        options.truncate(open_mode.contains('t'));

        if open_mode.contains('w') || open_mode.contains('a') || open_mode.contains('t') {
            options.create(true);
        }

        let file = options.open(path).await?;
        Ok(Box::new(file.into_std().await))
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, FileIoError> {
        std::fs::read(path).map_err(FileIoError::Io)
    }

    fn read_to_string(&self, path: &Path) -> Result<String, FileIoError> {
        std::fs::read_to_string(path).map_err(FileIoError::Io)
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
        std::fs::create_dir_all(path).map_err(FileIoError::Io)
    }

    async fn read_dir(&self, path: &Path) -> Result<Vec<FsNode>, FileIoError> {
        let mut entries = fs::read_dir(path).await?;
        let mut nodes = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let metadata = entry.metadata().await?;
            nodes.push(FsNode {
                filename: entry.file_name().to_str().unwrap().to_string(),
                raw_path: path.to_str().unwrap_or_default().to_string(),
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

    fn walk_dir(&self, path: &Path, follow_links: bool) -> Result<Vec<FsNode>, FileIoError> {
        let path = path.to_path_buf();
        WalkDir::new(path)
            .follow_links(follow_links)
            .into_iter()
            .filter_map(|e| e.ok())
            .map(|entry| {
                let path = entry.path().to_path_buf();
                let metadata = entry.metadata().map_err(|e| FileIoError::Io(e.into()))?;
                Ok(FsNode {
                    filename: entry.file_name().to_str().unwrap().to_string(),
                    raw_path: path.to_str().unwrap_or_default().to_string(),
                    path,
                    is_dir: metadata.is_dir(),
                    is_file: metadata.is_file(),
                    size: metadata.len(),
                })
            })
            .collect::<Result<Vec<_>, _>>()
    }

    fn exists(&self, path: &Path) -> Result<bool, FileIoError> {
        Ok(std::fs::exists(path)?)
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
        let path = self.canonicalize_path(path)?;
        let metadata = std::fs::metadata(&path)?;
        Ok(FsNode {
            filename: path.file_name().unwrap().to_str().unwrap().to_string(),
            raw_path: path.to_str().unwrap_or_default().to_string(),
            path,
            is_dir: metadata.is_dir(),
            is_file: metadata.is_file(),
            size: metadata.len(),
        })
    }

    fn canonicalize_str(&self, path: &str) -> Result<FsNode, FileIoError> {
        let path = self.canonicalize_path_str(path)?;
        let metadata = std::fs::metadata(&path)?;
        Ok(FsNode {
            filename: path.file_name().unwrap().to_str().unwrap().to_string(),
            raw_path: path.to_str().unwrap_or_default().to_string(),
            path,
            is_dir: metadata.is_dir(),
            is_file: metadata.is_file(),
            size: metadata.len(),
        })
    }

    async fn ensure_file(&self, path: &Path) -> Result<FsNode, FileIoError> {
        if !self.exists(path)? {
            if let Some(parent) = path.parent() {
                self.create_dir_all(parent)?;
            }
            self.write(path, &[]).await?;
        }
        self.canonicalize(path)
    }

    async fn ensure_directory(&self, path: &Path) -> Result<FsNode, FileIoError> {
        if !self.exists(path)? {
            self.create_dir_all(path)?;
        }
        self.canonicalize(path)
    }
}
