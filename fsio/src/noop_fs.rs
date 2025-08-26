use std::{
    io::{Cursor, Read, Seek, Write},
    path::{Path, PathBuf},
};

use async_trait::async_trait;

use crate::{FileIo, FileIoError, FileStream, FsNode};

pub struct NoOpFsIo;

impl NoOpFsIo {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoOpFsIo {
    fn default() -> Self {
        Self::new()
    }
}

pub struct NoOpStream {
    cursor: Cursor<Vec<u8>>,
}

impl NoOpStream {
    pub fn new() -> Self {
        Self {
            cursor: Cursor::new(Vec::new()),
        }
    }
}

impl Default for NoOpStream {
    fn default() -> Self {
        Self::new()
    }
}

impl Read for NoOpStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.cursor.read(buf)
    }
}

impl Write for NoOpStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.cursor.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.cursor.flush()
    }
}

impl Seek for NoOpStream {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.cursor.seek(pos)
    }
}

#[async_trait]
impl FileIo for NoOpFsIo {
    fn name(&self) -> &'static str {
        "NoOp"
    }

    fn open(&self, _path: &Path, _open_mode: &str) -> Result<Box<dyn FileStream>, FileIoError> {
        Ok(Box::new(NoOpStream::new()))
    }

    async fn open_async(
        &self,
        _path: &Path,
        _open_mode: &str,
    ) -> Result<Box<dyn FileStream>, FileIoError> {
        Ok(Box::new(NoOpStream::new()))
    }

    fn read(&self, _path: &Path) -> Result<Vec<u8>, FileIoError> {
        Ok(Vec::new())
    }

    fn read_to_string(&self, _path: &Path) -> Result<String, FileIoError> {
        Ok(String::new())
    }

    async fn write(&self, _path: &Path, _contents: &[u8]) -> Result<(), FileIoError> {
        Ok(())
    }

    async fn write_string(&self, _path: &Path, _contents: &str) -> Result<(), FileIoError> {
        Ok(())
    }

    async fn create_dir(&self, _parent: &Path, name: &str) -> Result<PathBuf, FileIoError> {
        Ok(PathBuf::from(name))
    }

    fn create_dir_all(&self, _path: &Path) -> Result<(), FileIoError> {
        Ok(())
    }

    async fn read_dir(&self, _path: &Path) -> Result<Vec<FsNode>, FileIoError> {
        Ok(Vec::new())
    }

    async fn remove_file(&self, _path: &Path) -> Result<(), FileIoError> {
        Ok(())
    }

    async fn remove_dir_all(&self, _path: &Path) -> Result<(), FileIoError> {
        Ok(())
    }

    fn walk_dir(&self, _path: &Path, _follow_links: bool) -> Result<Vec<FsNode>, FileIoError> {
        Ok(Vec::new())
    }

    fn exists(&self, _path: &Path) -> Result<bool, FileIoError> {
        Ok(false)
    }

    async fn is_file(&self, _path: &Path) -> Result<bool, FileIoError> {
        Ok(false)
    }

    async fn is_dir(&self, _path: &Path) -> Result<bool, FileIoError> {
        Ok(false)
    }

    fn canonicalize_path(&self, path: &Path) -> Result<PathBuf, FileIoError> {
        Ok(path.to_path_buf())
    }

    fn canonicalize_path_str(&self, path: &str) -> Result<PathBuf, FileIoError> {
        Ok(PathBuf::from(path))
    }

    fn canonicalize(&self, path: &Path) -> Result<FsNode, FileIoError> {
        Ok(FsNode {
            filename: path.file_name().unwrap().to_str().unwrap().to_string(),
            raw_path: path.to_str().unwrap_or_default().to_string(),
            path: path.to_path_buf(),
            is_dir: false,
            is_file: false,
            size: 0,
        })
    }

    fn canonicalize_str(&self, path: &str) -> Result<FsNode, FileIoError> {
        Ok(FsNode {
            filename: Path::new(path)
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string(),
            raw_path: path.to_string(),
            path: PathBuf::from(path),
            is_dir: false,
            is_file: false,
            size: 0,
        })
    }

    async fn ensure_file(&self, path: &Path) -> Result<FsNode, FileIoError> {
        self.canonicalize(path)
    }

    async fn ensure_directory(&self, path: &Path) -> Result<FsNode, FileIoError> {
        self.canonicalize(path)
    }
}
