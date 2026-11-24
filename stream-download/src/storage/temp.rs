//! Storage implementations for reading and writing to a temporary file. If the content length is
//! known, the buffer size will be initialized to the content length, but the buffer will expand
//! beyond that if required.

use std::ffi::OsString;
use std::fmt::Debug;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek};
use std::path::PathBuf;
use std::sync::Arc;

use educe::Educe;
pub use tempfile;
use tempfile::NamedTempFile;

use super::StorageProvider;
use crate::WrapIoResult;

type TempfileFnType = Arc<dyn Fn() -> io::Result<NamedTempFile> + Send + Sync + 'static>;

#[derive(Clone, Educe)]
#[educe(Debug)]
struct TempfileFn(#[educe(Debug = false)] TempfileFnType);

/// Creates a [`TempStorageReader`] backed by a temporary file
#[derive(Default, Clone, Debug)]
pub struct TempStorageProvider {
    storage_dir: Option<PathBuf>,
    prefix: Option<OsString>,
    tempfile_fn: Option<TempfileFn>,
}

impl TempStorageProvider {
    /// Creates a new [`TempStorageProvider`] that creates temporary files in the OS-specific
    /// default location.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new [`TempStorageProvider`] that creates temporary files in the specified
    /// location.
    pub fn new_in<T>(path: T) -> Self
    where
        T: Into<PathBuf>,
    {
        Self {
            storage_dir: Some(path.into()),
            tempfile_fn: None,
            prefix: None,
        }
    }

    /// Creates a new [`TempStorageProvider`] that creates temporary files using the specified
    /// filename prefix.
    pub fn with_prefix<T>(prefix: T) -> Self
    where
        T: Into<OsString>,
    {
        Self {
            storage_dir: None,
            tempfile_fn: None,
            prefix: Some(prefix.into()),
        }
    }

    /// Creates a new [`TempStorageProvider`] that creates temporary files in the specified location
    /// using the specified filename prefix.
    pub fn with_prefix_in<S, P>(prefix: S, path: P) -> Self
    where
        S: Into<OsString>,
        P: Into<PathBuf>,
    {
        Self {
            storage_dir: Some(path.into()),
            tempfile_fn: None,
            prefix: Some(prefix.into()),
        }
    }

    /// Creates a new [`TempStorageProvider`] with a custom [`tempfile::NamedTempFile`]
    /// builder.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use stream_download::storage::temp::{TempStorageProvider, tempfile};
    ///
    /// TempStorageProvider::with_tempfile_builder(|| {
    ///     tempfile::Builder::new().suffix("testfile").tempfile()
    /// });
    /// ```
    pub fn with_tempfile_builder<F: Fn() -> io::Result<NamedTempFile> + Send + Sync + 'static>(
        builder: F,
    ) -> Self {
        Self {
            storage_dir: None,
            prefix: None,
            tempfile_fn: Some(TempfileFn(Arc::new(builder))),
        }
    }
}

impl StorageProvider for TempStorageProvider {
    type Reader = TempStorageReader;
    type Writer = File;

    fn into_reader_writer(
        self,
        _content_length: Option<u64>,
    ) -> io::Result<(Self::Reader, Self::Writer)> {
        let tempfile = if let Some(tempfile_fn) = self.tempfile_fn {
            (tempfile_fn.0)()
        } else {
            let mut builder = tempfile::Builder::new();
            let mut builder_mut = &mut builder;
            if let Some(prefix) = &self.prefix {
                builder_mut = builder_mut.prefix(prefix);
            }
            self.storage_dir.map_or_else(
                || builder_mut.tempfile(),
                |dir| builder_mut.tempfile_in(dir),
            )
        }
        .wrap_err("error creating temp file")?;

        let handle = tempfile.reopen().wrap_err("error reopening temp file")?;
        let reader = TempStorageReader {
            reader: BufReader::new(tempfile),
        };
        let writer = handle
            .try_clone()
            .wrap_err("error cloning temporary file")?;
        Ok((reader, writer))
    }
}

/// Reader created by a [`TempStorageProvider`]. Reads from a temporary file.
#[derive(Debug)]
pub struct TempStorageReader {
    reader: BufReader<NamedTempFile>,
}

impl Read for TempStorageReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buf)
    }
}

impl Seek for TempStorageReader {
    fn seek(&mut self, position: io::SeekFrom) -> io::Result<u64> {
        self.reader.seek(position)
    }
}
