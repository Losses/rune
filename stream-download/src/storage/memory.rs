//! Storage implementations for reading and writing to an in-memory buffer. If the content length is
//! known, the buffer size will be initialized to the content length, but the buffer will expand
//! beyond that if required.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use parking_lot::RwLock;

use super::StorageProvider;
use crate::WrapIoResult;

/// Creates a [`MemoryStorage`] with an initial size based on the supplied content length.
#[derive(Default, Clone, Debug)]
pub struct MemoryStorageProvider;

impl StorageProvider for MemoryStorageProvider {
    type Reader = MemoryStorage;
    type Writer = MemoryStorage;

    fn into_reader_writer(
        self,
        content_length: Option<u64>,
    ) -> io::Result<(Self::Reader, Self::Writer)> {
        let initial_buffer_size = content_length.unwrap_or(0);
        let initial_buffer_size: usize = initial_buffer_size
            .try_into()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
            .wrap_err(&format!(
                "Requested buffer size of {initial_buffer_size} exceeds the maximum value"
            ))?;

        let inner = Arc::new(AssertUnwindSafe(RwLock::new(vec![0; initial_buffer_size])));
        let written = Arc::new(AtomicUsize::new(0));
        let reader = MemoryStorage {
            inner: inner.clone(),
            position: 0,
            written: written.clone(),
        };
        let writer = MemoryStorage {
            inner,
            position: 0,
            written,
        };
        Ok((reader, writer))
    }
}

/// Simple thread-safe in-memory buffer that supports the standard IO traits.
#[derive(Debug)]
pub struct MemoryStorage {
    inner: Arc<AssertUnwindSafe<RwLock<Vec<u8>>>>,
    position: usize,
    written: Arc<AtomicUsize>,
}

impl Read for MemoryStorage {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let inner = self.inner.read();

        let available_len =
            (inner.len().saturating_sub(self.position)).min(self.written.load(Ordering::SeqCst));
        let read_len = available_len.min(buf.len());
        buf[..read_len].copy_from_slice(&inner[self.position..self.position + read_len]);
        self.position += read_len;
        Ok(read_len)
    }
}

impl Seek for MemoryStorage {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let len = self.inner.read().len();
        let new_position = match position {
            SeekFrom::Start(position) => position as usize,
            SeekFrom::Current(from_current) => (self.position as i64 + from_current) as usize,
            SeekFrom::End(from_end) => (len as i64 + from_end) as usize,
        };

        self.position = new_position;
        Ok(new_position as u64)
    }
}

impl Write for MemoryStorage {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut inner = self.inner.write();

        if self.position + buf.len() > inner.len() {
            inner.resize(self.position + buf.len(), 0);
        }

        inner[self.position..self.position + buf.len()].copy_from_slice(buf);

        self.position += buf.len();
        self.written.fetch_add(buf.len(), Ordering::SeqCst);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
