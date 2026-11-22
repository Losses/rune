//! A [`SourceStream`] adapter for any source that implements [`AsyncRead`].

use std::convert::Infallible;
use std::io;
use std::pin::Pin;

use bytes::Bytes;
use futures_util::Stream;
use tokio::io::AsyncRead;
use tokio_util::io::ReaderStream;

use crate::source::SourceStream;

/// Parameters for creating an [`AsyncReadStream`].
#[derive(Debug)]
pub struct AsyncReadStreamParams<T> {
    stream: T,
    content_length: Option<u64>,
}

impl<T> AsyncReadStreamParams<T> {
    /// Creates a new [`AsyncReadStreamParams`] instance.
    pub fn new(stream: T) -> Self {
        Self {
            stream,
            content_length: None,
        }
    }

    /// Sets the content length of the stream.
    /// A generic [`AsyncRead`] source has no way of knowing the content length automatically, so it
    /// must be set explicitly or it will default to [`None`].
    #[must_use]
    pub fn content_length<L>(self, content_length: L) -> Self
    where
        L: Into<Option<u64>>,
    {
        Self {
            content_length: content_length.into(),
            ..self
        }
    }
}

/// An implementation of the [`SourceStream`] trait for any stream implementing [`AsyncRead`].
#[derive(Debug)]
pub struct AsyncReadStream<T> {
    stream: ReaderStream<T>,
    content_length: Option<u64>,
}

impl<T> AsyncReadStream<T>
where
    T: AsyncRead + Send + Sync + Unpin + 'static,
{
    /// Creates a new [`AsyncReadStream`].
    pub fn new<L>(stream: T, content_length: L) -> Self
    where
        L: Into<Option<u64>>,
    {
        Self {
            stream: ReaderStream::new(stream),
            content_length: content_length.into(),
        }
    }
}

impl<T> SourceStream for AsyncReadStream<T>
where
    T: AsyncRead + Send + Sync + Unpin + 'static,
{
    type Params = AsyncReadStreamParams<T>;

    type StreamCreationError = Infallible;

    async fn create(params: Self::Params) -> Result<Self, Self::StreamCreationError> {
        Ok(Self::new(params.stream, params.content_length))
    }

    fn content_length(&self) -> Option<u64> {
        self.content_length
    }

    fn supports_seek(&self) -> bool {
        false
    }

    async fn seek_range(&mut self, _start: u64, _end: Option<u64>) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "seek unsupported",
        ))
    }

    async fn reconnect(&mut self, _current_position: u64) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "reconnect unsupported",
        ))
    }
}

impl<T> Stream for AsyncReadStream<T>
where
    T: AsyncRead + Unpin,
{
    type Item = io::Result<Bytes>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        Pin::new(&mut self.stream).poll_next(cx)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Read};
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use tokio::io::{AsyncRead, ReadBuf};
    use tokio::task::spawn_blocking;
    use tokio::time::timeout;

    use super::AsyncReadStreamParams;
    use crate::storage::memory::MemoryStorageProvider;
    use crate::{Settings, StreamDownload};

    /// Yields its bytes on the first read, then either an I/O error or a clean end.
    struct Reader {
        sent: bool,
        fail_after_data: bool,
    }

    impl AsyncRead for Reader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            if !self.sent {
                self.sent = true;
                buf.put_slice(b"data");
                Poll::Ready(Ok(()))
            } else if self.fail_after_data {
                Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "injected reader failure",
                )))
            } else {
                Poll::Ready(Ok(()))
            }
        }
    }

    async fn stream(fail_after_data: bool) -> StreamDownload<MemoryStorageProvider> {
        StreamDownload::new_async_read(
            AsyncReadStreamParams::new(Reader {
                sent: false,
                fail_after_data,
            }),
            MemoryStorageProvider,
            Settings::default(),
        )
        .await
        .expect("create async read stream")
    }

    /// Reads to the end, then once more past the delivered bytes, on a reader thread.
    async fn read_twice(
        mut stream: StreamDownload<MemoryStorageProvider>,
    ) -> (io::Result<Vec<u8>>, io::Result<usize>) {
        let handle = stream.handle();
        // The reader hands the stream back: dropping it would cancel the download and end the wait
        // for its completion at once.
        let read = spawn_blocking(move || {
            let mut output = Vec::new();
            let first = stream.read_to_end(&mut output).map(|_| output);
            let second = stream.read(&mut [0; 1]);
            (first, second, stream)
        });
        let (first, second, _stream) = timeout(Duration::from_secs(5), read)
            .await
            .expect("peer step timed out: read async reader output")
            .expect("reader thread");
        timeout(Duration::from_secs(5), handle.wait_for_completion())
            .await
            .expect("peer step timed out: async reader stream completion");
        (first, second)
    }

    #[tokio::test]
    async fn reader_error_after_data_fails_the_stream() {
        let (first, second) = read_twice(stream(true).await).await;
        assert!(
            first.is_err(),
            "a reader error must fail the stream, got {first:?}"
        );
        assert!(
            second.is_err(),
            "a read past the delivered bytes must fail, got {second:?}"
        );
    }

    #[tokio::test]
    async fn reader_clean_end_completes_the_stream() {
        let (first, second) = read_twice(stream(false).await).await;
        assert_eq!(
            first.expect("a clean end must complete the stream"),
            b"data"
        );
        assert_eq!(second.expect("a read at the end must succeed"), 0);
    }
}
