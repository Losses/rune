#![deny(missing_docs)]
#![forbid(clippy::unwrap_used)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

use std::fmt::Debug;
use std::future::{self, Future};
use std::io::{self, Read, Seek, SeekFrom};

use educe::Educe;
pub use settings::*;
use source::handle::SourceHandle;
use source::{DecodeError, Source, SourceStream};
use storage::StorageProvider;
use tokio_util::sync::CancellationToken;
use tracing::{debug, instrument, trace};

#[cfg(feature = "async-read")]
pub mod async_read;
#[cfg(feature = "http")]
pub mod http;
#[cfg(feature = "process")]
pub mod process;
#[cfg(feature = "registry")]
pub mod registry;
mod settings;
pub mod source;
pub mod storage;

/// A handle that can be usd to interact with the stream remotely.
#[derive(Debug, Clone)]
pub struct StreamHandle {
    finished: CancellationToken,
}

impl StreamHandle {
    /// Wait for the stream download task to complete.
    ///
    /// This method can be useful when using a [`ProcessStream`][process::ProcessStream] if you want
    /// to ensure the subprocess has exited cleanly before continuing.
    pub async fn wait_for_completion(self) {
        self.finished.cancelled().await;
    }
}

/// Represents content streamed from a remote source.
/// This struct implements [read](https://doc.rust-lang.org/stable/std/io/trait.Read.html)
/// and [seek](https://doc.rust-lang.org/stable/std/io/trait.Seek.html)
/// so it can be used as a generic source for libraries and applications that operate on these
/// traits. On creation, an async task is spawned that will immediately start to download the remote
/// content.
///
/// Any read attempts that request part of the stream that hasn't been downloaded yet will block
/// until the requested portion is reached. Any seek attempts that meet the same criteria will
/// result in additional request to restart the stream download from the seek point.
///
/// If the stream download hasn't completed when this struct is dropped, the task will be cancelled.
///
/// If the stream stalls for any reason, the download task will attempt to automatically reconnect.
/// This reconnect interval can be controlled via [`Settings::retry_timeout`].
/// Server-side failures are not automatically handled and should be retried by the supplied
/// [`SourceStream`] implementation if desired.
#[derive(Debug)]
pub struct StreamDownload<P: StorageProvider> {
    output_reader: P::Reader,
    handle: SourceHandle,
    download_task_cancellation_token: CancellationToken,
    cancel_on_drop: bool,
    content_length: Option<u64>,
    storage_capacity: Option<usize>,
}

impl<P: StorageProvider> StreamDownload<P> {
    #[cfg(feature = "reqwest")]
    /// Creates a new [`StreamDownload`] that accesses an HTTP resource at the given URL.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use std::error::Error;
    /// use std::io::{self, Read};
    /// use std::result::Result;
    ///
    /// use stream_download::source::DecodeError;
    /// use stream_download::storage::temp::TempStorageProvider;
    /// use stream_download::{Settings, StreamDownload};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn Error>> {
    ///     let mut reader = match StreamDownload::new_http(
    ///         "https://some-cool-url.com/some-file.mp3".parse()?,
    ///         TempStorageProvider::default(),
    ///         Settings::default(),
    ///     )
    ///     .await
    ///     {
    ///         Ok(reader) => reader,
    ///         Err(e) => return Err(e.decode_error().await)?,
    ///     };
    ///
    ///     tokio::task::spawn_blocking(move || {
    ///         let mut buf = Vec::new();
    ///         reader.read_to_end(&mut buf)?;
    ///         Ok::<_, io::Error>(())
    ///     })
    ///     .await??;
    ///     Ok(())
    /// }
    /// ```
    pub async fn new_http(
        url: ::reqwest::Url,
        storage_provider: P,
        settings: Settings<http::HttpStream<::reqwest::Client>>,
    ) -> Result<Self, StreamInitializationError<http::HttpStream<::reqwest::Client>>> {
        Self::new(url, storage_provider, settings).await
    }

    /// Creates a new [`StreamDownload`] that accesses an HTTP resource at the given URL.
    /// It uses the [`reqwest_middleware::ClientWithMiddleware`] client instead of the default
    /// [`reqwest`] client. Any global middleware set by [`Settings::add_default_middleware`] will
    /// be automatically applied.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use std::error::Error;
    /// use std::io::{self, Read};
    /// use std::result::Result;
    ///
    /// use reqwest_retry::RetryTransientMiddleware;
    /// use reqwest_retry::policies::ExponentialBackoff;
    /// use stream_download::source::DecodeError;
    /// use stream_download::storage::temp::TempStorageProvider;
    /// use stream_download::{Settings, StreamDownload};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn Error>> {
    ///     let retry_policy = ExponentialBackoff::builder().build_with_max_retries(3);
    ///     Settings::add_default_middleware(RetryTransientMiddleware::new_with_policy(retry_policy));
    ///
    ///     let mut reader = match StreamDownload::new_http_with_middleware(
    ///         "https://some-cool-url.com/some-file.mp3".parse()?,
    ///         TempStorageProvider::default(),
    ///         Settings::default(),
    ///     )
    ///     .await
    ///     {
    ///         Ok(reader) => reader,
    ///         Err(e) => return Err(e.decode_error().await)?,
    ///     };
    ///
    ///     tokio::task::spawn_blocking(move || {
    ///         let mut buf = Vec::new();
    ///         reader.read_to_end(&mut buf)?;
    ///         Ok::<_, io::Error>(())
    ///     })
    ///     .await??;
    ///     Ok(())
    /// }
    /// ```
    #[cfg(feature = "reqwest-middleware")]
    pub async fn new_http_with_middleware(
        url: ::reqwest::Url,
        storage_provider: P,
        settings: Settings<http::HttpStream<::reqwest_middleware::ClientWithMiddleware>>,
    ) -> Result<
        Self,
        StreamInitializationError<http::HttpStream<::reqwest_middleware::ClientWithMiddleware>>,
    > {
        Self::new(url, storage_provider, settings).await
    }

    /// Creates a new [`StreamDownload`] that uses an [`AsyncRead`][tokio::io::AsyncRead] resource.
    ///
    /// # Example reading from `stdin`
    ///
    /// ```no_run
    /// use std::error::Error;
    /// use std::io::{self, Read};
    /// use std::result::Result;
    ///
    /// use stream_download::async_read::AsyncReadStreamParams;
    /// use stream_download::storage::temp::TempStorageProvider;
    /// use stream_download::{Settings, StreamDownload};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn Error>> {
    ///     let mut reader = StreamDownload::new_async_read(
    ///         AsyncReadStreamParams::new(tokio::io::stdin()),
    ///         TempStorageProvider::new(),
    ///         Settings::default(),
    ///     )
    ///     .await?;
    ///
    ///     tokio::task::spawn_blocking(move || {
    ///         let mut buf = Vec::new();
    ///         reader.read_to_end(&mut buf)?;
    ///         Ok::<_, io::Error>(())
    ///     })
    ///     .await??;
    ///     Ok(())
    /// }
    /// ```
    #[cfg(feature = "async-read")]
    pub async fn new_async_read<T>(
        params: async_read::AsyncReadStreamParams<T>,
        storage_provider: P,
        settings: Settings<async_read::AsyncReadStream<T>>,
    ) -> Result<Self, StreamInitializationError<async_read::AsyncReadStream<T>>>
    where
        T: tokio::io::AsyncRead + Send + Sync + Unpin + 'static,
    {
        Self::new(params, storage_provider, settings).await
    }

    /// Creates a new [`StreamDownload`] that uses a [`Command`][process::Command] as input.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use std::error::Error;
    /// use std::io::{self, Read};
    /// use std::result::Result;
    ///
    /// use stream_download::process::{Command, ProcessStreamParams};
    /// use stream_download::storage::temp::TempStorageProvider;
    /// use stream_download::{Settings, StreamDownload};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn Error>> {
    ///     let mut reader = StreamDownload::new_process(
    ///         ProcessStreamParams::new(Command::new("cat").args(["./assets/music.mp3"]))?,
    ///         TempStorageProvider::new(),
    ///         Settings::default(),
    ///     )
    ///     .await?;
    ///
    ///     tokio::task::spawn_blocking(move || {
    ///         let mut buf = Vec::new();
    ///         reader.read_to_end(&mut buf)?;
    ///         Ok::<_, io::Error>(())
    ///     })
    ///     .await??;
    ///     Ok(())
    /// }
    /// ```
    #[cfg(feature = "process")]
    pub async fn new_process(
        params: process::ProcessStreamParams,
        storage_provider: P,
        settings: Settings<process::ProcessStream>,
    ) -> Result<Self, StreamInitializationError<process::ProcessStream>> {
        Self::new(params, storage_provider, settings).await
    }

    /// Creates a new [`StreamDownload`] that accesses a remote resource at the given URL.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use std::error::Error;
    /// use std::io::{self, Read};
    /// use std::result::Result;
    ///
    /// use reqwest::Client;
    /// use stream_download::http::HttpStream;
    /// use stream_download::storage::temp::TempStorageProvider;
    /// use stream_download::{Settings, StreamDownload};
    ///
    /// use crate::stream_download::source::DecodeError;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn Error>> {
    ///     let mut reader = match StreamDownload::new::<HttpStream<Client>>(
    ///         "https://some-cool-url.com/some-file.mp3".parse()?,
    ///         TempStorageProvider::default(),
    ///         Settings::default(),
    ///     )
    ///     .await
    ///     {
    ///         Ok(reader) => reader,
    ///         Err(e) => return Err(e.decode_error().await)?,
    ///     };
    ///
    ///     tokio::task::spawn_blocking(move || {
    ///         let mut buf = Vec::new();
    ///         reader.read_to_end(&mut buf)?;
    ///         Ok::<_, io::Error>(())
    ///     })
    ///     .await??;
    ///     Ok(())
    /// }
    /// ```
    pub async fn new<S>(
        params: S::Params,
        storage_provider: P,
        settings: Settings<S>,
    ) -> Result<Self, StreamInitializationError<S>>
    where
        S: SourceStream,
        S::Error: Debug + Send,
    {
        Self::from_create_stream(move || S::create(params), storage_provider, settings).await
    }

    /// Creates a new [`StreamDownload`] from a [`SourceStream`].
    ///
    /// # Example
    ///
    /// ```no_run
    /// use std::error::Error;
    /// use std::io::Read;
    /// use std::result::Result;
    ///
    /// use reqwest::Client;
    /// use stream_download::http::HttpStream;
    /// use stream_download::storage::temp::TempStorageProvider;
    /// use stream_download::{Settings, StreamDownload};
    ///
    /// use crate::stream_download::source::DecodeError;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn Error>> {
    ///     let stream = HttpStream::new(
    ///         Client::new(),
    ///         "https://some-cool-url.com/some-file.mp3".parse()?,
    ///     )
    ///     .await?;
    ///
    ///     let mut reader = match StreamDownload::from_stream(
    ///         stream,
    ///         TempStorageProvider::default(),
    ///         Settings::default(),
    ///     )
    ///     .await
    ///     {
    ///         Ok(reader) => reader,
    ///         Err(e) => Err(e.decode_error().await)?,
    ///     };
    ///     Ok(())
    /// }
    /// ```
    pub async fn from_stream<S>(
        stream: S,
        storage_provider: P,
        settings: Settings<S>,
    ) -> Result<Self, StreamInitializationError<S>>
    where
        S: SourceStream,
        S::Error: Debug + Send,
    {
        Self::from_create_stream(
            move || future::ready(Ok(stream)),
            storage_provider,
            settings,
        )
        .await
    }

    /// Cancels the background task that's downloading the stream content.
    /// This has no effect if the download is already completed.
    pub fn cancel_download(&self) {
        self.download_task_cancellation_token.cancel();
    }

    /// Returns the [`CancellationToken`] for the download task.
    /// This can be used to cancel the download task before it completes.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.download_task_cancellation_token.clone()
    }

    /// Returns a [`StreamHandle`] that can be used to interact with
    /// the stream remotely.
    pub fn handle(&self) -> StreamHandle {
        StreamHandle {
            finished: self.download_task_cancellation_token.clone(),
        }
    }

    /// Returns the content length of the stream, if available.
    pub fn content_length(&self) -> Option<u64> {
        self.content_length
    }

    async fn from_create_stream<S, F, Fut>(
        create_stream: F,
        storage_provider: P,
        settings: Settings<S>,
    ) -> Result<Self, StreamInitializationError<S>>
    where
        S: SourceStream<Error: Debug + Send>,
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<S, S::StreamCreationError>> + Send,
    {
        let stream = create_stream()
            .await
            .map_err(StreamInitializationError::StreamCreationFailure)?;
        let content_length = stream.content_length();
        let storage_capacity = storage_provider.max_capacity();
        let (reader, writer) = storage_provider
            .into_reader_writer(content_length)
            .map_err(StreamInitializationError::StorageCreationFailure)?;
        let cancellation_token = CancellationToken::new();
        let cancel_on_drop = settings.cancel_on_drop;
        let mut source = Source::new(writer, content_length, settings, cancellation_token.clone());
        let handle = source.source_handle();

        tokio::spawn({
            let cancellation_token = cancellation_token.clone();
            async move {
                source.download(stream).await;
                cancellation_token.cancel();
                debug!("download task finished");
            }
        });

        Ok(Self {
            output_reader: reader,
            handle,
            download_task_cancellation_token: cancellation_token,
            cancel_on_drop,
            content_length,
            storage_capacity,
        })
    }

    fn get_absolute_seek_position(&mut self, relative_position: SeekFrom) -> io::Result<u64> {
        Ok(match relative_position {
            SeekFrom::Start(position) => {
                debug!(seek_position = position, "seeking from start");
                position
            }
            SeekFrom::Current(position) => {
                debug!(seek_position = position, "seeking from current position");
                (self.output_reader.stream_position()? as i64 + position) as u64
            }
            SeekFrom::End(position) => {
                debug!(seek_position = position, "seeking from end");
                if let Some(length) = self.handle.content_length() {
                    (length as i64 + position) as u64
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "cannot seek from end when content length is unknown",
                    ));
                }
            }
        })
    }

    fn handle_read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let res = self.output_reader.read(buf).inspect(|l| {
            trace!(read_length = format!("{l:?}"), "returning read");
        });
        self.handle.notify_read();
        res
    }

    fn normalize_requested_position(&self, requested_position: u64) -> u64 {
        if let Some(content_length) = self.content_length {
            // ensure we don't request a position beyond the end of the stream
            requested_position.min(content_length)
        } else {
            requested_position
        }
    }

    fn check_for_failure(&self) -> io::Result<()> {
        if self.handle.is_failed() {
            Err(io::Error::other("stream failed to download"))
        } else {
            Ok(())
        }
    }

    fn check_for_excessive_read(&self, buf_len: usize) -> io::Result<()> {
        // Ensure the buffer fits within the storage capacity.
        // We could get around this from erroring by breaking this into multiple smaller reads, but
        // if you're using a bounded storage type, that's probably not what you want.
        let capacity = self.storage_capacity.unwrap_or(usize::MAX);
        if buf_len > capacity {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("buffer size {buf_len} exceeds the max capacity of {capacity}",),
            ))
        } else {
            Ok(())
        }
    }

    fn check_for_excessive_seek(&mut self, absolute_seek_position: u64) -> io::Result<()> {
        // Ensure the seek position is within the available storage capacity.
        // We could get around this by issuing a few read requests until the seek position is within
        // bounds, but if you're using a bounded storage type, that's probably not what you want.
        if let Some(max_capacity) = self.storage_capacity {
            let max_possible_seek_position = self
                .output_reader
                .stream_position()?
                .saturating_add(max_capacity as u64);
            if absolute_seek_position
                > self
                    .output_reader
                    .stream_position()?
                    .saturating_add(max_capacity as u64)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "seek position {absolute_seek_position} exceeds maximum of \
                         {max_possible_seek_position}"
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// Error returned when initializing a stream.
#[derive(thiserror::Error, Educe)]
#[educe(Debug)]
pub enum StreamInitializationError<S: SourceStream> {
    /// Storage creation failure.
    #[error("Storage creation failure: {0}")]
    StorageCreationFailure(io::Error),
    /// Stream creation failure.
    #[error("Stream creation failure: {0}")]
    StreamCreationFailure(<S as SourceStream>::StreamCreationError),
}

impl<S: SourceStream> DecodeError for StreamInitializationError<S> {
    async fn decode_error(self) -> String {
        match self {
            this @ Self::StorageCreationFailure(_) => this.to_string(),
            Self::StreamCreationFailure(e) => e.decode_error().await,
        }
    }
}

impl<P: StorageProvider> Drop for StreamDownload<P> {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.cancel_download();
        }
    }
}

impl<P: StorageProvider> Read for StreamDownload<P> {
    #[instrument(skip_all, fields(len=buf.len()))]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.check_for_failure()?;
        self.check_for_excessive_read(buf.len())?;

        trace!(buffer_length = buf.len(), "read requested");
        let stream_position = self.output_reader.stream_position()?;
        let requested_position =
            self.normalize_requested_position(stream_position + buf.len() as u64);
        trace!(
            current_position = stream_position,
            requested_position = requested_position
        );

        if let Some(closest_set) = self.handle.get_downloaded_at_position(stream_position) {
            trace!(
                downloaded_range = format!("{closest_set:?}"),
                "current position already downloaded"
            );
            if closest_set.end >= requested_position {
                trace!("requested position already downloaded");
                return self.handle_read(buf);
            }
            debug!("requested position not yet downloaded");
        } else {
            debug!("stream position not yet downloaded");
        }

        self.handle.wait_for_position(requested_position);
        self.check_for_failure()?;
        debug!(
            current_position = stream_position,
            requested_position = requested_position,
            output_stream_position = self.output_reader.stream_position()?,
            "reached requested position"
        );

        self.handle_read(buf)
    }
}

impl<P: StorageProvider> Seek for StreamDownload<P> {
    #[instrument(skip(self))]
    fn seek(&mut self, relative_position: SeekFrom) -> io::Result<u64> {
        self.check_for_failure()?;

        let absolute_seek_position = self.get_absolute_seek_position(relative_position)?;
        let absolute_seek_position = self.normalize_requested_position(absolute_seek_position);
        self.check_for_excessive_seek(absolute_seek_position)?;

        debug!(absolute_seek_position, "absolute seek position");
        if let Some(closest_set) = self
            .handle
            .get_downloaded_at_position(absolute_seek_position)
        {
            debug!(
                downloaded_range = format!("{closest_set:?}"),
                "seek position already downloaded"
            );
            return self
                .output_reader
                .seek(SeekFrom::Start(absolute_seek_position))
                .inspect_err(|p| debug!(position = format!("{p:?}"), "returning seek position"));
        }

        self.handle.seek(absolute_seek_position);
        self.check_for_failure()?;
        debug!("reached seek position");

        self.output_reader
            .seek(SeekFrom::Start(absolute_seek_position))
            .inspect_err(|p| debug!(position = format!("{p:?}"), "returning seek position"))
    }
}

pub(crate) trait WrapIoResult {
    fn wrap_err(self, msg: &str) -> Self;
}

impl<T> WrapIoResult for io::Result<T> {
    fn wrap_err(self, msg: &str) -> Self {
        if let Err(e) = self {
            Err(io::Error::new(e.kind(), format!("{msg}: {e}")))
        } else {
            self
        }
    }
}
