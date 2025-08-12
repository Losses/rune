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

    async fn write(&self, _path: &Path, _contents: &[u8]) -> Result<(), FileIoError> {
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

    async fn walk_dir(
        &self,
        _path: &Path,
        _follow_links: bool,
    ) -> Result<Vec<FsNode>, FileIoError> {
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

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FileIoError> {
        Ok(path.to_path_buf())
    }
}
