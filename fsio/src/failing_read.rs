//! Standard filesystem fixtures for failed reads and unavailable Android documents.
use std::{
    io::{Error, ErrorKind, Read, Result as IoResult, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use async_trait::async_trait;

use crate::{
    DirWalk, FileIo, FileIoError, FileStream, FsIo, FsNode, OpenMode, StdFsIo, StorageLocation,
};

/// The standard file system, except that one read fails once, as when the
/// connection to a remote library drops partway through a file. Only files
/// opened for reading that have been sought can fail: hashing reads a file
/// straight through, while a decoder seeks, so the failure lands in the
/// decoder's file however many times the file was hashed first. In such a file,
/// the read that reaches byte `fail_at` stops just before it, and the next read
/// from `fail_at` fails with `ConnectionReset`. Only the first read to reach
/// `fail_at` fails; reads that start past it, every later read and every other
/// file are served as usual.
#[derive(Clone)]
pub struct OneFailingRead {
    fail_at: u64,
    failed: Arc<AtomicBool>,
}

impl OneFailingRead {
    pub fn new(fail_at: u64) -> Self {
        Self {
            fail_at,
            failed: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn fs(&self) -> FsIo {
        ReadFault::Stream(self.clone()).fs()
    }

    /// Whether the failing read has happened.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }

    fn wrap(&self, stream: Box<dyn FileStream>, open_mode: OpenMode) -> Box<dyn FileStream> {
        if open_mode != OpenMode::Read {
            return stream;
        }
        Box::new(FailingStream {
            inner: stream,
            position: 0,
            sought: false,
            fault: self.clone(),
        })
    }
}

struct FailingStream {
    inner: Box<dyn FileStream>,
    position: u64,
    sought: bool,
    fault: OneFailingRead,
}

impl Read for FailingStream {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        let fail_at = self.fault.fail_at;
        let armed = self.sought && !self.fault.failed();
        let len = if armed && self.position == fail_at {
            self.fault.failed.store(true, Ordering::SeqCst);
            return Err(Error::new(ErrorKind::ConnectionReset, "connection reset"));
        } else if armed && self.position < fail_at {
            buf.len().min((fail_at - self.position) as usize)
        } else {
            buf.len()
        };
        let count = self.inner.read(&mut buf[..len])?;
        self.position += count as u64;
        Ok(count)
    }
}

impl Seek for FailingStream {
    fn seek(&mut self, pos: SeekFrom) -> IoResult<u64> {
        self.position = self.inner.seek(pos)?;
        self.sought = true;
        Ok(self.position)
    }
}

impl Write for FailingStream {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> IoResult<()> {
        self.inner.flush()
    }
}

impl FileStream for FailingStream {
    fn sync_all(&mut self) -> Result<(), FileIoError> {
        self.inner.sync_all()
    }
}

/// The step at which an otherwise ordinary file loses provider access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentOperation {
    ModifiedTime,
    Open,
    Exists,
}

/// Lists the real directory, then returns a SAF error for one operation on this file.
pub fn unavailable_document(path: &Path, operation: DocumentOperation) -> FsIo {
    ReadFault::Document(path.to_owned(), operation).fs()
}

enum ReadFault {
    Stream(OneFailingRead),
    Document(PathBuf, DocumentOperation),
}

impl ReadFault {
    fn fs(self) -> FsIo {
        FsIo {
            inner: Arc::new(FailingReadIo {
                inner: StdFsIo::new(),
                fault: self,
            }),
        }
    }

    fn check(&self, path: &Path, operation: DocumentOperation) -> Result<(), FileIoError> {
        if matches!(self, Self::Document(document, step) if document == path && *step == operation)
        {
            return Err(FileIoError::Saf("document is unavailable".into()));
        }
        Ok(())
    }

    fn wrap(&self, stream: Box<dyn FileStream>, mode: OpenMode) -> Box<dyn FileStream> {
        match self {
            Self::Stream(failure) => failure.wrap(stream, mode),
            Self::Document(..) => stream,
        }
    }
}

struct FailingReadIo {
    inner: StdFsIo,
    fault: ReadFault,
}

#[async_trait]
impl FileIo for FailingReadIo {
    fn name(&self) -> &'static str {
        "Std with a read failure"
    }

    fn storage(&self) -> StorageLocation {
        self.inner.storage()
    }

    fn open(&self, path: &Path, open_mode: OpenMode) -> Result<Box<dyn FileStream>, FileIoError> {
        self.fault.check(path, DocumentOperation::Open)?;
        let stream = self.inner.open(path, open_mode)?;
        Ok(self.fault.wrap(stream, open_mode))
    }

    async fn open_async(
        &self,
        path: &Path,
        open_mode: OpenMode,
    ) -> Result<Box<dyn FileStream>, FileIoError> {
        self.fault.check(path, DocumentOperation::Open)?;
        let stream = self.inner.open_async(path, open_mode).await?;
        Ok(self.fault.wrap(stream, open_mode))
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, FileIoError> {
        self.inner.read(path)
    }

    fn read_to_string(&self, path: &Path) -> Result<String, FileIoError> {
        self.inner.read_to_string(path)
    }

    async fn write(&self, path: &Path, contents: &[u8]) -> Result<(), FileIoError> {
        self.inner.write(path, contents).await
    }

    async fn write_string(&self, path: &Path, contents: &str) -> Result<(), FileIoError> {
        self.inner.write_string(path, contents).await
    }

    async fn create_dir(&self, parent: &Path, name: &str) -> Result<PathBuf, FileIoError> {
        self.inner.create_dir(parent, name).await
    }

    fn create_dir_all(&self, path: &Path) -> Result<(), FileIoError> {
        self.inner.create_dir_all(path)
    }

    async fn read_dir(&self, path: &Path) -> Result<Vec<FsNode>, FileIoError> {
        self.inner.read_dir(path).await
    }

    async fn remove_file(&self, path: &Path) -> Result<(), FileIoError> {
        self.inner.remove_file(path).await
    }

    async fn rename_file(&self, from: &Path, to: &Path) -> Result<(), FileIoError> {
        self.inner.rename_file(from, to).await
    }

    async fn remove_dir_all(&self, path: &Path) -> Result<(), FileIoError> {
        self.inner.remove_dir_all(path).await
    }

    fn walk_dir(&self, path: &Path, follow_links: bool) -> Result<DirWalk, FileIoError> {
        self.inner.walk_dir(path, follow_links)
    }

    fn exists(&self, path: &Path) -> Result<bool, FileIoError> {
        self.fault.check(path, DocumentOperation::Exists)?;
        self.inner.exists(path)
    }

    fn entry_exists(&self, path: &Path) -> Result<bool, FileIoError> {
        self.inner.entry_exists(path)
    }

    async fn is_file(&self, path: &Path) -> Result<bool, FileIoError> {
        self.inner.is_file(path).await
    }

    async fn is_dir(&self, path: &Path) -> Result<bool, FileIoError> {
        self.inner.is_dir(path).await
    }

    fn canonicalize_path(&self, path: &Path) -> Result<PathBuf, FileIoError> {
        self.inner.canonicalize_path(path)
    }

    fn canonicalize_path_str(&self, path: &str) -> Result<PathBuf, FileIoError> {
        self.inner.canonicalize_path_str(path)
    }

    fn canonicalize(&self, path: &Path) -> Result<FsNode, FileIoError> {
        self.inner.canonicalize(path)
    }

    fn canonicalize_str(&self, path: &str) -> Result<FsNode, FileIoError> {
        self.inner.canonicalize_str(path)
    }

    fn modified_time(&self, path: &Path) -> Result<u64, FileIoError> {
        self.fault.check(path, DocumentOperation::ModifiedTime)?;
        self.inner.modified_time(path)
    }

    fn change_token(&self, path: &Path) -> Result<Option<String>, FileIoError> {
        self.inner.change_token(path)
    }

    async fn ensure_file(&self, path: &Path) -> Result<FsNode, FileIoError> {
        self.inner.ensure_file(path).await
    }

    async fn ensure_directory(&self, path: &Path) -> Result<FsNode, FileIoError> {
        self.inner.ensure_directory(path).await
    }
}
