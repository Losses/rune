use std::num::NonZeroUsize;
use std::ops::Range;
use std::time::Duration;

use educe::Educe;
use tokio_util::sync::CancellationToken;

pub(crate) type ProgressFn<S> = Box<dyn FnMut(&S, StreamState, &CancellationToken) + Send + Sync>;

pub(crate) type ReconnectFn<S> = Box<dyn FnMut(&S, &CancellationToken) + Send + Sync>;

/// Current phase of the download for use during a progress callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamPhase {
    /// Stream is currently in a prefetch state.
    #[non_exhaustive]
    Prefetching {
        /// Current prefetch target.
        target: u64,
        /// Size of the most recently downloaded chunk.
        chunk_size: usize,
    },
    /// Stream is currently in a downloading state.
    #[non_exhaustive]
    Downloading {
        /// Size of the most recently downloaded chunk.
        chunk_size: usize,
    },
    /// Stream has finished downloading.
    Complete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
/// State of the stream for use during a progress callback.
pub struct StreamState {
    /// Current position of the stream.
    pub current_position: u64,
    /// Time elapsed since download start.
    pub elapsed: Duration,
    /// Current phase of the download.
    pub phase: StreamPhase,
    /// Current chunk of the stream being downloaded.
    pub current_chunk: Range<u64>,
}

/// Settings to configure the stream behavior.
#[derive(Educe)]
#[educe(Debug, PartialEq, Eq)]
pub struct Settings<S> {
    pub(crate) prefetch_bytes: u64,
    pub(crate) batch_write_size: usize,
    pub(crate) retry_timeout: Duration,
    pub(crate) cancel_on_drop: bool,
    #[educe(Debug = false, PartialEq = false)]
    pub(crate) on_progress: Option<ProgressFn<S>>,
    #[educe(Debug = false, PartialEq = false)]
    pub(crate) on_reconnect: Option<ReconnectFn<S>>,
}

impl<S> Default for Settings<S> {
    fn default() -> Self {
        Self {
            prefetch_bytes: 256 * 1024,
            batch_write_size: 4096,
            retry_timeout: Duration::from_secs(5),
            cancel_on_drop: true,
            on_progress: None,
            on_reconnect: None,
        }
    }
}

impl<S> Settings<S> {
    /// How many bytes to download from the stream before allowing read requests.
    /// This is used to create a buffer between the read position and the stream position
    /// and prevent stuttering.
    ///
    /// The default value is 256 kilobytes.
    #[must_use]
    pub fn prefetch_bytes(self, prefetch_bytes: u64) -> Self {
        Self {
            prefetch_bytes,
            ..self
        }
    }

    /// The maximum number of bytes written to the underlying
    /// [`StorageWriter`](crate::storage::StorageWriter) before yielding to the async runtime. This
    /// prevents large writes from performing long blocking operations without giving the scheduler
    /// a chance to schedule other tasks.
    ///
    /// The default value is 4096.
    pub fn batch_write_size(self, batch_write_size: NonZeroUsize) -> Self {
        Self {
            batch_write_size: batch_write_size.get(),
            ..self
        }
    }

    /// If there is no new data for a duration greater than this timeout, we will attempt to
    /// reconnect to the server.
    ///  
    /// This timeout is designed to help streams recover during temporary network failures,
    /// but you may need to increase this if you're seeing warnings about timeouts in the logs
    /// under normal network conditions.
    ///
    /// The default value is 5 seconds.
    #[must_use]
    pub fn retry_timeout(self, retry_timeout: Duration) -> Self {
        Self {
            retry_timeout,
            ..self
        }
    }

    /// If set to `true`, this will cause the stream download task to automatically cancel when the
    /// [`StreamDownload`][crate::StreamDownload] instance is dropped. It's useful to disable this
    /// if you want to ensure the stream shuts down cleanly.
    ///
    /// The default value is `true`.
    #[must_use]
    pub fn cancel_on_drop(self, cancel_on_drop: bool) -> Self {
        Self {
            cancel_on_drop,
            ..self
        }
    }

    /// Attach a callback function that will be called when a new chunk of the stream is processed.
    /// The provided [`CancellationToken`] can be used to immediately cancel the stream.
    ///
    /// # Example
    ///
    /// ```
    /// use reqwest::Client;
    /// use stream_download::Settings;
    /// use stream_download::http::HttpStream;
    /// use stream_download::source::SourceStream;
    ///
    /// let settings = Settings::default();
    /// settings.on_progress(|stream: &HttpStream<Client>, state, _| {
    ///     let progress = state.current_position as f32 / stream.content_length().unwrap() as f32;
    ///     println!("progress: {}%", progress * 100.0);
    /// });
    /// ```
    #[must_use]
    pub fn on_progress<F>(mut self, f: F) -> Self
    where
        F: FnMut(&S, StreamState, &CancellationToken) + Send + Sync + 'static,
    {
        self.on_progress = Some(Box::new(f));
        self
    }

    /// Attach a callback function that will be called when the stream reconnects after a failure.
    /// The provided [`CancellationToken`] can be used to immediately cancel the stream.
    #[must_use]
    pub fn on_reconnect<F>(mut self, f: F) -> Self
    where
        F: FnMut(&S, &CancellationToken) + Send + Sync + 'static,
    {
        self.on_reconnect = Some(Box::new(f));
        self
    }

    /// Retrieves the configured prefetch bytes
    pub const fn get_prefetch_bytes(&self) -> u64 {
        self.prefetch_bytes
    }

    /// Retrieves the configured batch write size
    pub const fn get_write_batch_size(&self) -> usize {
        self.batch_write_size
    }
}

#[cfg(feature = "reqwest-middleware")]
impl Settings<crate::http::HttpStream<::reqwest_middleware::ClientWithMiddleware>> {
    /// Adds a new [`reqwest_middleware::Middleware`]
    pub fn add_default_middleware<M>(middleware: M)
    where
        M: reqwest_middleware::Middleware,
    {
        crate::http::reqwest_middleware_client::add_default_middleware(middleware);
    }
}
