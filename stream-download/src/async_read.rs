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
