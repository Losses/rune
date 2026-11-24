//! Storage implementations for reading and writing to an in-memory buffer. If the content length is
//! known, the buffer size will be initialized to the content length, but the buffer will expand
//! beyond that if required.
//!
//! A read returns only bytes that a writer stored: it stops at the end of the written range at the
//! reader's position, and reads nothing at a position that was never written, including one past
//! the end of the buffer. Seeking past the end is allowed; seeking before the start, or to a
//! position that does not fit in a `usize`, is an [`io::ErrorKind::InvalidInput`] error.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use parking_lot::RwLock;
use rangemap::RangeSet;

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

        let inner = Arc::new(AssertUnwindSafe(RwLock::new(Buffer {
            data: vec![0; initial_buffer_size],
            written: RangeSet::new(),
        })));
        let reader = MemoryStorage {
            inner: inner.clone(),
            position: 0,
        };
        let writer = MemoryStorage { inner, position: 0 };
        Ok((reader, writer))
    }
}

/// Simple thread-safe in-memory buffer that supports the standard IO traits.
#[derive(Debug)]
pub struct MemoryStorage {
    inner: Arc<AssertUnwindSafe<RwLock<Buffer>>>,
    position: usize,
}

/// The bytes shared by a reader and a writer, and the ranges of them the writer has stored. The
/// rest of `data` is preallocated or seek padding, not stream content.
#[derive(Debug)]
struct Buffer {
    data: Vec<u8>,
    written: RangeSet<usize>,
}

impl Read for MemoryStorage {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let inner = self.inner.read();

        let Some(written) = inner.written.get(&self.position) else {
            return Ok(0);
        };
        let read_len = (written.end - self.position).min(buf.len());
        let end = self.position + read_len;
        buf[..read_len].copy_from_slice(&inner.data[self.position..end]);
        self.position = end;
        Ok(read_len)
    }
}

impl Seek for MemoryStorage {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let target = match position {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::Current(from_current) => self.position as i128 + i128::from(from_current),
            SeekFrom::End(from_end) => self.inner.read().data.len() as i128 + i128::from(from_end),
        };
        let new_position = usize::try_from(target).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "invalid seek to {position:?} from position {}",
                    self.position
                ),
            )
        })?;

        self.position = new_position;
        Ok(new_position as u64)
    }
}

impl Write for MemoryStorage {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut inner = self.inner.write();
        let end = self.position + buf.len();

        if end > inner.data.len() {
            inner.data.resize(end, 0);
        }

        inner.data[self.position..end].copy_from_slice(buf);
        inner.written.insert(self.position..end);

        self.position = end;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
