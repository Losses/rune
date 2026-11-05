//! Storage wrappers that adaptively choose between two storage providers based on content length.
//!
//! This module provides a way to use different storage strategies depending on the stream's
//! characteristics:
//! - Infinite streams use a fixed-length storage type wrapped in a [`BoundedStorageProvider`]
//! - Finite streams smaller than or equal to the buffer size use the fixed-length storage type
//!   directly
//! - Finite streams larger than the buffer size use the variable-length storage type
//!
//! A typical use case is optimizing memory usage and reducing writes to persistent storage:
//! ```
//! use std::num::NonZeroUsize;
//!
//! use stream_download::storage::adaptive::AdaptiveStorageProvider;
//! use stream_download::storage::memory::MemoryStorageProvider;
//! use stream_download::storage::temp::TempStorageProvider;
//!
//! // Use memory for small streams, temp files for large ones
//! let provider = AdaptiveStorageProvider::with_fixed_and_variable(
//!     MemoryStorageProvider,                       // fixed-length: memory storage
//!     TempStorageProvider::default(),              // variable-length: temp file storage
//!     NonZeroUsize::new(8 * 1024 * 1024).unwrap(), // 8MB buffer size
//! );
//! ```
//!
//! This approach helps reduce wear on storage devices (SSDs, SD cards, etc.) by keeping smaller
//! streams in memory while automatically switching to file storage for larger streams that would
//! consume too much RAM.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::num::NonZeroUsize;

use super::bounded::{BoundedStorageProvider, BoundedStorageReader, BoundedStorageWriter};
use super::{StorageProvider, StorageReader, StorageWriter};

/// Provides adaptive storage selection based on stream characteristics.
///
/// Takes two storage providers:
/// - `F`: Fixed-length storage provider, used for:
///   - Infinite streams (wrapped in [`BoundedStorageProvider`])
///   - Finite streams smaller than or equal to the buffer size
/// - `V`: Variable-length storage provider, used for:
///   - Finite streams larger than the buffer size
#[derive(Clone, Debug)]
pub struct AdaptiveStorageProvider<F, V>
where
    F: StorageProvider,
    V: StorageProvider,
{
    buffer_size: NonZeroUsize,
    fixed_storage: F,
    variable_storage: V,
}

impl<F, V> AdaptiveStorageProvider<F, V>
where
    F: StorageProvider,
    V: StorageProvider,
{
    /// Creates a new [`AdaptiveStorageProvider`] with separate providers for fixed and
    /// variable-length storage.
    ///
    /// # Arguments
    /// * `fixed_storage` - Provider for fixed-length storage, used for infinite streams (wrapped in
    ///   [`BoundedStorageProvider`]) and finite streams smaller than or equal to `buffer_size`
    /// * `variable_storage` - Provider for variable-length storage, used for finite streams larger
    ///   than `buffer_size`
    /// * `buffer_size` - Maximum size for using fixed-length storage
    pub fn with_fixed_and_variable(
        fixed_storage: F,
        variable_storage: V,
        buffer_size: NonZeroUsize,
    ) -> Self {
        Self {
            buffer_size,
            fixed_storage,
            variable_storage,
        }
    }
}

impl<T> AdaptiveStorageProvider<T, T>
where
    T: StorageProvider,
{
    /// Creates a new [`AdaptiveStorageProvider`] using the same provider type for both fixed
    /// and variable-length storage.
    ///
    /// This is a convenience constructor that clones the provider and calls
    /// [`with_fixed_and_variable`](Self::with_fixed_and_variable).
    pub fn new(provider: T, buffer_size: NonZeroUsize) -> Self
    where
        T: Clone,
    {
        Self::with_fixed_and_variable(provider.clone(), provider, buffer_size)
    }
}

impl<F, V> StorageProvider for AdaptiveStorageProvider<F, V>
where
    F: StorageProvider,
    V: StorageProvider,
{
    type Reader = AdaptiveStorageReader<F::Reader, V::Reader>;
    type Writer = AdaptiveStorageWriter<F::Writer, V::Writer>;

    fn into_reader_writer(
        self,
        content_length: Option<u64>,
    ) -> io::Result<(Self::Reader, Self::Writer)> {
        match content_length {
            None => {
                // For infinite streams, use bounded storage
                let provider = BoundedStorageProvider::new(self.fixed_storage, self.buffer_size);
                let (reader, writer) = provider.into_reader_writer(None)?;
                Ok((Self::Reader::Bounded(reader), Self::Writer::Bounded(writer)))
            }
            Some(length) => {
                if u64::try_from(self.buffer_size.get()).is_ok_and(|buffer| length <= buffer) {
                    // Small enough for fixed-length storage
                    let (reader, writer) = self.fixed_storage.into_reader_writer(Some(length))?;
                    Ok((Self::Reader::Fixed(reader), Self::Writer::Fixed(writer)))
                } else {
                    // Too large, use variable-length storage
                    let (reader, writer) =
                        self.variable_storage.into_reader_writer(Some(length))?;
                    Ok((
                        Self::Reader::Variable(reader),
                        Self::Writer::Variable(writer),
                    ))
                }
            }
        }
    }
}

/// Reader that adaptively uses either fixed-length or variable-length storage
///
/// The storage type used depends on the stream characteristics:
/// - `Bounded`: For infinite streams, using fixed-length storage with size limits
/// - `Fixed`: For finite streams smaller than or equal to the buffer size
/// - `Variable`: For finite streams larger than the buffer size
#[derive(Debug)]
pub enum AdaptiveStorageReader<F, V>
where
    F: StorageReader,
    V: StorageReader,
{
    /// Used for infinite streams, wrapping the fixed-length storage in a bounded buffer
    Bounded(BoundedStorageReader<F>),
    /// Used for finite streams smaller than or equal to the buffer size
    Fixed(F),
    /// Used for finite streams larger than the buffer size
    Variable(V),
}

impl<F, V> Read for AdaptiveStorageReader<F, V>
where
    F: StorageReader,
    V: StorageReader,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Bounded(r) => r.read(buf),
            Self::Fixed(r) => r.read(buf),
            Self::Variable(r) => r.read(buf),
        }
    }
}

impl<F, V> Seek for AdaptiveStorageReader<F, V>
where
    F: StorageReader,
    V: StorageReader,
{
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match self {
            Self::Bounded(r) => r.seek(pos),
            Self::Fixed(r) => r.seek(pos),
            Self::Variable(r) => r.seek(pos),
        }
    }
}

/// Writer that adaptively uses either fixed-length or variable-length storage
///
/// The storage type used depends on the stream characteristics:
/// - `Bounded`: For infinite streams, using fixed-length storage with size limits
/// - `Fixed`: For finite streams smaller than or equal to the buffer size
/// - `Variable`: For finite streams larger than the buffer size
#[derive(Debug)]
pub enum AdaptiveStorageWriter<F, V>
where
    F: StorageWriter,
    V: StorageWriter,
{
    /// Used for infinite streams, wrapping the fixed-length storage in a bounded buffer
    Bounded(BoundedStorageWriter<F>),
    /// Used for finite streams smaller than or equal to the buffer size
    Fixed(F),
    /// Used for finite streams larger than the buffer size
    Variable(V),
}

impl<F, V> Write for AdaptiveStorageWriter<F, V>
where
    F: StorageWriter,
    V: StorageWriter,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Bounded(w) => w.write(buf),
            Self::Fixed(w) => w.write(buf),
            Self::Variable(w) => w.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Bounded(w) => w.flush(),
            Self::Fixed(w) => w.flush(),
            Self::Variable(w) => w.flush(),
        }
    }
}

impl<F, V> Seek for AdaptiveStorageWriter<F, V>
where
    F: StorageWriter,
    V: StorageWriter,
{
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match self {
            Self::Bounded(w) => w.seek(pos),
            Self::Fixed(w) => w.seek(pos),
            Self::Variable(w) => w.seek(pos),
        }
    }
}
