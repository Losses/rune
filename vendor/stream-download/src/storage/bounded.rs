//! Storage wrappers for restricting the size of the underlying storage layer.
//!
//! This is useful for dealing with infinite streams when you don't want the storage size to keep
//! growing indefinitely. It can also be used for downloading large files where you want to prevent
//! allocating too much space at one time.
//!
//! The underlying data is used as a circular buffer - once it reaches capacity, it will begin to
//! overwrite old data.
//!
//! Because the buffer will never resize, it's important to ensure the buffer is large enough to
//! hold all of the data you will need at once. This needs to account for any seeking that may occur
//! as well as the size of the initial prefetch phase. Attempting to read or seek outside the buffer
//! will return an error.
//!
//! If the reader falls too far behind the writer, the writer will pause so the reader can catch up.
//!
//! If your inputs may or may not have a known content length, consider using an
//! [`AdaptiveStorageProvider`](super::adaptive::AdaptiveStorageProvider) to automatically
//! determine whether or not the overhead of maintaining a bounded buffer is necessary.

use std::fmt::Debug;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::num::NonZeroUsize;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use educe::Educe;
use parking_lot::Mutex;
use tracing::{debug, instrument, trace, warn};

use super::{StorageProvider, StorageReader, StorageWriter};
use crate::WrapIoResult;

/// Creates a [`BoundedStorageReader`] with a fixed size.
#[derive(Clone, Debug)]
pub struct BoundedStorageProvider<T>
where
    T: StorageProvider,
{
    inner: T,
    buffer_size: usize,
}

impl<T> BoundedStorageProvider<T>
where
    T: StorageProvider,
{
    /// Creates a new [`BoundedStorageProvider`] with the specified fixed buffer size.
    ///
    /// **Note:** If the source has a known content length, the argument provided to `buffer_size`
    /// will be compared with the content length and the smaller of the two sizes will be used.
    /// This prevents excess allocations when the source is smaller than the buffer.
    pub fn new(inner: T, buffer_size: NonZeroUsize) -> Self {
        Self {
            inner,
            buffer_size: buffer_size.get(),
        }
    }
}

impl<T> StorageProvider for BoundedStorageProvider<T>
where
    T: StorageProvider,
{
    type Reader = BoundedStorageReader<T::Reader>;
    type Writer = BoundedStorageWriter<T::Writer>;

    fn max_capacity(&self) -> Option<usize> {
        Some(self.buffer_size)
    }

    fn into_reader_writer(
        self,
        content_length: Option<u64>,
    ) -> io::Result<(Self::Reader, Self::Writer)> {
        // We need to take the smaller of the two sizes here to prevent excess memory allocation
        let buffer_size = if let Some(content_length) = content_length {
            content_length.min(self.buffer_size as u64)
        } else {
            self.buffer_size as u64
        };

        let (reader, writer) = self.inner.into_reader_writer(Some(buffer_size))?;

        let buffer_size: usize = buffer_size
            .try_into()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
            .wrap_err(&format!(
                "Requested buffer size of {buffer_size} exceeds the maximum value"
            ))?;

        let shared_info = Arc::new(AssertUnwindSafe(Mutex::new(SharedInfo {
            read: 0,
            written: 0,
            read_position: 0,
            write_position: 0,
            size: buffer_size,
        })));
        let reader = BoundedStorageReader {
            inner: reader,
            shared_info: shared_info.clone(),
        };
        let writer = BoundedStorageWriter {
            inner: writer,
            shared_info,
        };
        Ok((reader, writer))
    }
}

#[derive(Debug)]
struct SharedInfo {
    read: usize,
    written: usize,
    read_position: usize,
    write_position: usize,
    size: usize,
}

impl SharedInfo {
    fn mapped_read_position(&self, val: usize) -> usize {
        (val + (self.read_position % self.size)) % self.size
    }

    fn mapped_write_position(&self, val: usize) -> usize {
        (val + (self.write_position % self.size)) % self.size
    }
}

/// Reader created by a [`BoundedStorageProvider`]. Reads from a fixed-size circular buffer.
#[derive(Educe)]
#[educe(Debug)]
pub struct BoundedStorageReader<T>
where
    T: StorageReader,
{
    #[educe(Debug = false)]
    inner: T,
    shared_info: Arc<AssertUnwindSafe<Mutex<SharedInfo>>>,
}

impl<T> Read for BoundedStorageReader<T>
where
    T: StorageReader,
{
    #[instrument(skip(buf))]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut shared_info = self.shared_info.lock();

        if buf.len() > shared_info.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "read size {} is greater than buffer size {}",
                    buf.len(),
                    shared_info.size
                ),
            ));
        }

        let available_len = shared_info
            .write_position
            .saturating_sub(shared_info.read_position);

        if available_len > shared_info.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "read position {} is too far behind write position {}, size {}",
                    shared_info.read_position, shared_info.write_position, shared_info.size
                ),
            ));
        }

        if shared_info.read >= shared_info.written {
            debug!("read bytes >= written bytes, ending read");
            return Ok(0);
        }

        let size = shared_info.size.min(shared_info.write_position);
        let available_buf_size = available_len.min(buf.len());

        trace!(buf_len = buf.len(), "original buf len");

        let buf = &mut buf[..available_buf_size];
        if buf.is_empty() {
            return Ok(0);
        }

        let start = shared_info.mapped_read_position(0);
        // is_empty check above prevents subtraction with overflow here
        let end = shared_info.mapped_read_position(available_buf_size - 1) + 1;
        trace!(
            start,
            end,
            size,
            buf_len = buf.len(),
            read = shared_info.read,
            "bounded read"
        );

        self.inner
            .seek(SeekFrom::Start(start as u64))
            .wrap_err("error seeking to mapped start")?;

        if start < end {
            self.inner
                .read_exact(buf)
                .wrap_err("error reading mapped positions")?;
        } else {
            // buffer is non-contiguous, need to read the first segment and then wrap around to the
            // start to read the rest
            let first_seg_len = size - start;
            self.inner
                .read_exact(&mut buf[..first_seg_len])
                .wrap_err("error reading first mapped segment")?;
            self.inner
                .seek(SeekFrom::Start(0))
                .wrap_err("error seeking second mapped segment")?;
            self.inner
                .read_exact(&mut buf[first_seg_len..])
                .wrap_err("error reading second mapped segment")?;
        };

        let buf_len = buf.len();
        shared_info.read_position += buf_len;
        shared_info.read += buf_len;

        Ok(buf_len)
    }
}

impl<T> Seek for BoundedStorageReader<T>
where
    T: StorageReader,
{
    #[instrument]
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        trace!("reader seek");
        let mut shared_info = self.shared_info.lock();
        let new_position = handle_seek(position, shared_info.read_position)?;
        // If we performed a backwards seek,we need to adjust the absolute read position.
        // Otherwise, if we've moved the read position back, but keep incrementing the
        // read counter, it may look like we've read more bytes than we've written.
        shared_info.read -= shared_info.read_position.saturating_sub(new_position);
        shared_info.read_position = new_position;
        Ok(new_position as u64)
    }
}

fn handle_seek(position: SeekFrom, current_position: usize) -> io::Result<usize> {
    match position {
        SeekFrom::Start(position) => Ok(position as usize),
        SeekFrom::Current(from_current) => Ok((current_position as i64 + from_current) as usize),
        SeekFrom::End(_) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "seek from end not supported",
        )),
    }
}

/// Write handle created by a [`BoundedStorageReader`]. Writes to a fixed-size circular buffer.
#[derive(Educe)]
#[educe(Debug)]
pub struct BoundedStorageWriter<T>
where
    T: StorageWriter,
{
    #[educe(Debug = false)]
    inner: T,
    shared_info: Arc<AssertUnwindSafe<Mutex<SharedInfo>>>,
}

impl<T> Write for BoundedStorageWriter<T>
where
    T: StorageWriter,
{
    #[instrument(skip(buf))]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut shared_info = self.shared_info.lock();
        let space_taken = shared_info
            .write_position
            .saturating_sub(shared_info.read_position);
        let available_size = shared_info.size.saturating_sub(space_taken).min(buf.len());
        trace!(available_size, buf_len = buf.len(), "calculated buf size");
        // If we don't have enough space to write the entire buffer, write as much as we can
        let buf = &buf[..available_size];
        if buf.is_empty() {
            return Ok(0);
        }

        let start = shared_info.mapped_write_position(0);
        // is_empty check above prevents subtraction with overflow here
        let end = shared_info.mapped_write_position(buf.len() - 1) + 1;
        trace!(
            start,
            end,
            buf_len = buf.len(),
            written = shared_info.written,
            "bounded write"
        );

        self.inner
            .seek(SeekFrom::Start(start as u64))
            .wrap_err("error seeking to mapped write start")?;
        if start < end {
            self.inner
                .write_all(buf)
                .wrap_err("error writing mapped segment")?;
        } else {
            let first_seg_len = shared_info.size - start;
            self.inner
                .write_all(&buf[..first_seg_len])
                .wrap_err("error writing first mapped_segment")?;
            self.inner
                .seek(SeekFrom::Start(0))
                .wrap_err("error seeking for second mapped segment")?;
            self.inner
                .write_all(&buf[first_seg_len..])
                .wrap_err("error writing second mapped segment")?;
        }

        let buf_len = buf.len();
        shared_info.write_position += buf_len;
        shared_info.written += buf_len;

        self.inner.flush().wrap_err("error flushing during write")?;

        Ok(buf_len)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<T> Seek for BoundedStorageWriter<T>
where
    T: StorageWriter,
{
    #[instrument]
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        trace!("writer seek");
        let mut shared_info = self.shared_info.lock();
        let new_position = handle_seek(position, shared_info.write_position)?;
        shared_info.written -= shared_info.write_position.saturating_sub(new_position);
        shared_info.write_position = new_position;
        Ok(new_position as u64)
    }
}
