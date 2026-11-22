//! Configurable implementations for the buffer's storage layer.
//! Pre-configured implementations are available for memory and temporary file-based storage.

use std::io::{self, Read, Seek, Write};

pub mod adaptive;
#[cfg(test)]
mod adaptive_tests;
pub mod bounded;
#[cfg(test)]
mod bounded_tests;
pub mod memory;
#[cfg(test)]
mod memory_tests;
#[cfg(feature = "temp-storage")]
pub mod temp;

/// Creates a [`StorageReader`] and [`StorageWriter`] based on the content
/// length returned from the [`SourceStream`](crate::source::SourceStream).
/// The reader and writer must track their position in the stream independently.
pub trait StorageProvider: Send {
    /// Source used to read from the underlying storage.
    type Reader: StorageReader;
    /// Handle that can write to the underlying storage.
    type Writer: StorageWriter;

    /// Turn the provider into a reader and writer.
    fn into_reader_writer(
        self,
        content_length: Option<u64>,
    ) -> io::Result<(Self::Reader, Self::Writer)>;

    /// Returns the maximum number of bytes this provider can hold at a time.
    fn max_capacity(&self) -> Option<usize> {
        None
    }

    /// Returns the maximum number of bytes this provider can hold at a time for a stream with the
    /// given content length.
    ///
    /// Providers whose capacity depends on the content length, such as
    /// [`AdaptiveStorageProvider`](adaptive::AdaptiveStorageProvider), override this. The default
    /// returns [`max_capacity`](Self::max_capacity).
    fn max_capacity_for(&self, _content_length: Option<u64>) -> Option<usize> {
        self.max_capacity()
    }
}

/// Trait used to read from a storage layer
pub trait StorageReader: Read + Seek + Send {}

impl<T> StorageReader for T where T: Read + Seek + Send {}

/// Handle for writing to the underlying storage layer.
pub trait StorageWriter: Write + Seek + Send + 'static {}

impl<T> StorageWriter for T where T: Write + Seek + Send + 'static {}
