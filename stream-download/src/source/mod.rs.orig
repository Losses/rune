//! Provides the [`SourceStream`] trait which abstracts over the transport used to
//! stream remote content.

use std::convert::Infallible;
use std::error::Error;
use std::fmt::Debug;
use std::future;
use std::io::{self, SeekFrom};
use std::time::{Duration, Instant};

use bytes::{BufMut, Bytes, BytesMut};
use futures_util::{Future, Stream, StreamExt, TryStream};
use handle::{
    DownloadStatus, Downloaded, NotifyRead, PositionReached, RequestedPosition, SourceHandle,
};
use tokio::sync::mpsc;
use tokio::task::yield_now;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, instrument, trace, warn};

use crate::storage::StorageWriter;
use crate::{ProgressFn, ReconnectFn, Settings, StreamPhase, StreamState};

pub(crate) mod handle;

/// Enum representing the final outcome of the stream.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StreamOutcome {
    /// The stream completed naturally.
    Completed,
    /// The stream was cancelled by the user.
    CancelledByUser,
}

/// Represents a remote resource that can be streamed over the network. Streaming
/// over http is implemented via the [`HttpStream`](crate::http::HttpStream)
/// implementation if the `http` feature is enabled.
///
/// The implementation must also implement the
/// [Stream](https://docs.rs/futures/latest/futures/stream/trait.Stream.html) trait.
pub trait SourceStream:
    TryStream<Ok = Bytes>
    + Stream<Item = Result<Self::Ok, Self::Error>>
    + Unpin
    + Send
    + Sync
    + Sized
    + 'static
{
    /// Parameters used to create the remote resource.
    type Params: Send;

    /// Error type thrown when creating the stream.
    type StreamCreationError: DecodeError + Send;

    /// Creates an instance of the stream.
    fn create(
        params: Self::Params,
    ) -> impl Future<Output = Result<Self, Self::StreamCreationError>> + Send;

    /// Returns the size of the remote resource in bytes. The result should be `None`
    /// if the stream is infinite or doesn't have a known length.
    fn content_length(&self) -> Option<u64>;

    /// Seeks to a specific position in the stream. This method is only called if the
    /// requested range has not been downloaded, so this method should jump to the
    /// requested position in the stream as quickly as possible.
    ///
    /// The start value should be inclusive and the end value should be exclusive.
    fn seek_range(
        &mut self,
        start: u64,
        end: Option<u64>,
    ) -> impl Future<Output = io::Result<()>> + Send;

    /// Attempts to reconnect to the server when a failure occurs.
    fn reconnect(&mut self, current_position: u64) -> impl Future<Output = io::Result<()>> + Send;

    /// Returns whether seeking is supported in the stream.
    /// If this method returns `false`, [`SourceStream::seek_range`] will never be invoked.
    fn supports_seek(&self) -> bool;

    /// Called when the stream finishes downloading
    fn on_finish(
        &mut self,
        result: io::Result<()>,
        #[expect(unused)] outcome: StreamOutcome,
    ) -> impl Future<Output = io::Result<()>> + Send {
        future::ready(result)
    }
}

/// Trait for decoding extra error information asynchronously.
pub trait DecodeError: Error + Send + Sized {
    /// Decodes extra error information.
    fn decode_error(self) -> impl Future<Output = String> + Send {
        future::ready(self.to_string())
    }
}

impl DecodeError for Infallible {
    async fn decode_error(self) -> String {
        // This will never get called since it's infallible
        String::new()
    }
}

#[derive(PartialEq, Eq)]
enum DownloadAction {
    Continue,
    Complete,
}

pub(crate) struct Source<S: SourceStream, W: StorageWriter> {
    writer: W,
    downloaded: Downloaded,
    download_status: DownloadStatus,
    requested_position: RequestedPosition,
    position_reached: PositionReached,
    notify_read: NotifyRead,
    content_length: Option<u64>,
    seek_tx: mpsc::Sender<u64>,
    seek_rx: mpsc::Receiver<u64>,
    prefetch_bytes: u64,
    batch_write_size: usize,
    retry_timeout: Duration,
    on_progress: Option<ProgressFn<S>>,
    on_reconnect: Option<ReconnectFn<S>>,
    prefetch_complete: bool,
    prefetch_start_position: u64,
    remaining_bytes: Option<Bytes>,
    cancellation_token: CancellationToken,
}

impl<S, W> Source<S, W>
where
    S: SourceStream<Error: Debug>,
    W: StorageWriter,
{
    pub(crate) fn new(
        writer: W,
        content_length: Option<u64>,
        settings: Settings<S>,
        cancellation_token: CancellationToken,
    ) -> Self {
        // buffer size of 1 is fine here because we wait for the position to update after we send
        // each request
        let (seek_tx, seek_rx) = mpsc::channel(1);
        Self {
            writer,
            downloaded: Downloaded::default(),
            download_status: DownloadStatus::default(),
            requested_position: RequestedPosition::default(),
            position_reached: PositionReached::default(),
            notify_read: NotifyRead::default(),
            seek_tx,
            seek_rx,
            content_length,
            prefetch_complete: settings.prefetch_bytes == 0,
            prefetch_bytes: settings.prefetch_bytes,
            batch_write_size: settings.batch_write_size,
            retry_timeout: settings.retry_timeout,
            on_progress: settings.on_progress,
            on_reconnect: settings.on_reconnect,
            prefetch_start_position: 0,
            remaining_bytes: None,
            cancellation_token,
        }
    }

    #[instrument(skip_all)]
    pub(crate) async fn download(&mut self, mut stream: S) {
        let res = self.download_inner(&mut stream).await;
        let (res, stream_res) = match res {
            Ok(StreamOutcome::Completed) => (Ok(()), StreamOutcome::Completed),
            Ok(StreamOutcome::CancelledByUser) => (
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "stream cancelled by user",
                )),
                StreamOutcome::CancelledByUser,
            ),
            Err(e) => (Err(e), StreamOutcome::Completed),
        };
        let res = stream.on_finish(res, stream_res).await;
        if let Err(e) = res {
            if stream_res == StreamOutcome::Completed {
                error!("download failed: {e:?}");
            }
            self.download_status.set_failed();
        }
        self.signal_download_complete();
    }

    async fn download_inner(&mut self, stream: &mut S) -> io::Result<StreamOutcome> {
        debug!("starting file download");
        let download_start = std::time::Instant::now();

        loop {
            // Some streams may get stuck if the connection has a hiccup while waiting for the next
            // chunk. Forcing the client to abort and retry may help in these cases.
            let next_chunk = timeout(self.retry_timeout, stream.next());
            tokio::select! {
                position = self.seek_rx.recv() => {
                    // seek_tx can't be dropped here since we keep a reference in this struct
                    self.handle_seek(stream, position.expect("seek_tx dropped")).await?;
                },
                bytes = next_chunk => {
                    let Ok(bytes) = bytes else {
                        self.handle_reconnect(stream).await?;
                        continue;
                    };
                    if self
                        .handle_bytes(stream, bytes, download_start)
                        .await?
                        == DownloadAction::Complete
                    {
                        debug!(
                            download_duration = format!("{:?}", download_start.elapsed()),
                            "stream finished downloading"
                        );
                        break;
                    }
                }
                () = self.cancellation_token.cancelled() => {
                    debug!("received cancellation request, stopping download task");
                    return Ok(StreamOutcome::CancelledByUser);
                }
            };
        }
        self.report_download_complete(stream, download_start)?;
        Ok(StreamOutcome::Completed)
    }

    async fn handle_seek(&mut self, stream: &mut S, position: u64) -> io::Result<()> {
        if self.should_seek(stream, position)? {
            debug!("seek position not yet downloaded");
            let current_stream_position = self.writer.stream_position()?;
            if self.prefetch_complete {
                debug!("re-starting prefetch");
                self.prefetch_start_position = position;
                self.prefetch_complete = false;
            } else {
                debug!("seeking during prefetch, ending prefetch early");
                self.downloaded
                    .add(self.prefetch_start_position..current_stream_position);
                self.prefetch_complete = true;
            }
            if let Some(content_length) = self.content_length {
                // Get the minimum possible start position to ensure we capture the entire range
                let min_start_position = current_stream_position.min(position);
                debug!(
                    start = min_start_position,
                    end = content_length,
                    "checking for seek range",
                );
                if let Some(gap) = self.downloaded.next_gap(min_start_position..content_length) {
                    // Gap start may be too low if we're seeking forward, so check it against the
                    // position
                    let seek_start = gap.start.max(position);
                    debug!(seek_start, seek_end = gap.end, "requesting seek range");
                    self.seek(stream, seek_start, Some(gap.end)).await?;
                }
            } else {
                self.seek(stream, position, None).await?;
            }
        }
        Ok(())
    }

    async fn handle_reconnect(&mut self, stream: &mut S) -> io::Result<()> {
        warn!("timed out reading next chunk, retrying");
        let pos = self.writer.stream_position()?;
        // We already know there's a network issue if we're attempting a reconnect.
        // A retry policy on the client may cause an exponential backoff to be triggered here, so
        // we'll cap the reconnect time to prevent additional delays between reconnect attempts.
        let reconnect_pos = tokio::time::timeout(self.retry_timeout, stream.reconnect(pos)).await;
        if reconnect_pos
            .inspect_err(|e| warn!("error attempting to reconnect: {e:?}"))
            .is_ok()
            && let Some(on_reconnect) = &mut self.on_reconnect
        {
            on_reconnect(stream, &self.cancellation_token);
        }

        Ok(())
    }

    async fn handle_prefetch(
        &mut self,
        stream: &mut S,
        bytes: Option<Bytes>,
        start_position: u64,
        download_start: Instant,
    ) -> io::Result<DownloadAction> {
        let Some(bytes) = bytes else {
            self.prefetch_complete = true;
            debug!("file shorter than prefetch length, download finished");
            self.writer.flush()?;
            let position = self.writer.stream_position()?;
            self.downloaded.add(start_position..position);

            return self.finish_or_find_next_gap(stream).await;
        };
        let written = self.write_batched(&bytes).await?;
        self.writer.flush()?;

        let stream_position = self.writer.stream_position()?;
        let partial_write = written < bytes.len();

        // End prefetch early if we weren't able to write the entire contents
        if partial_write {
            debug!(
                written,
                bytes_len = bytes.len(),
                "failed to write all during prefetch"
            );
            self.remaining_bytes = Some(bytes.slice(written..));
        }
        if (stream_position >= start_position + self.prefetch_bytes) || partial_write {
            self.downloaded.add(start_position..stream_position);
            debug!("prefetch complete");
            self.prefetch_complete = true;
        }

        self.report_prefetch_progress(stream, stream_position, download_start, written);
        Ok(DownloadAction::Continue)
    }

    async fn finish_or_find_next_gap(&mut self, stream: &mut S) -> io::Result<DownloadAction> {
        if stream.supports_seek()
            && let Some(content_length) = self.content_length
        {
            let gap = self.downloaded.next_gap(0..content_length);
            if let Some(gap) = gap {
                debug!(
                    missing = format!("{gap:?}"),
                    "downloading missing stream chunk"
                );
                self.seek(stream, gap.start, Some(gap.end)).await?;
                return Ok(DownloadAction::Continue);
            }
        }
        self.writer.flush()?;
        self.signal_download_complete();
        Ok(DownloadAction::Complete)
    }

    async fn write_batched(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut written = 0;
        loop {
            let write_size = self.batch_write_size.min(bytes[written..].len());
            let batch_written = self.writer.write(&bytes[written..written + write_size])?;
            if batch_written == 0 {
                return Ok(written);
            }
            written += batch_written;
            // yield between writes to ensure we don't spend too long on writes
            // without an await point
            yield_now().await;
        }
    }

    async fn handle_bytes(
        &mut self,
        stream: &mut S,
        bytes: Option<Result<Bytes, S::Error>>,
        download_start: Instant,
    ) -> io::Result<DownloadAction> {
        let bytes = match bytes.transpose() {
            Ok(bytes) => bytes,
            Err(e) => {
                error!("Error fetching chunk from stream: {e:?}");
                return Ok(DownloadAction::Continue);
            }
        };

        if !self.prefetch_complete {
            return self
                .handle_prefetch(stream, bytes, self.prefetch_start_position, download_start)
                .await;
        }

        let bytes = match (self.remaining_bytes.take(), bytes) {
            (Some(remaining), Some(bytes)) => {
                let mut combined = BytesMut::new();
                combined.put(remaining);
                combined.put(bytes);
                combined.freeze()
            }
            (Some(remaining), None) => remaining,
            (None, Some(bytes)) => bytes,
            (None, None) => {
                return self.finish_or_find_next_gap(stream).await;
            }
        };
        let bytes_len = bytes.len();
        let new_position = self.write(bytes).await?;
        self.report_downloading_progress(stream, new_position, download_start, bytes_len)?;

        Ok(DownloadAction::Continue)
    }

    async fn write(&mut self, bytes: Bytes) -> io::Result<u64> {
        let mut written = 0;
        let position = self.writer.stream_position()?;
        let mut new_position = position;
        // Keep writing until we process the whole buffer.
        // If the reader is falling behind, this may take several attempts.
        while written < bytes.len() {
            self.notify_read.request();
            let new_written = self.write_batched(&bytes[written..]).await?;
            trace!(written, new_written, len = bytes.len(), "wrote data");

            if new_written > 0 {
                self.writer.flush()?;
                written += new_written;
            }
            new_position = self.writer.stream_position()?;
            if new_position > position {
                self.downloaded.add(position..new_position);
            }

            if let Some(requested) = self.requested_position.get() {
                debug!(
                    requested_position = requested,
                    current_position = new_position,
                    "received requested position"
                );

                if new_position >= requested {
                    debug!("notifying position reached");
                    self.requested_position.clear();
                    self.position_reached.notify_position_reached();
                }
            }
            if new_written == 0 {
                // We're not able to write any data, so we need to wait for space to be available
                debug!("waiting for next read");
                self.notify_read.wait_for_read().await;
                debug!("read finished");
            }

            trace!(
                previous_position = position,
                new_position,
                chunk_size = bytes.len(),
                "received response chunk"
            );
        }
        Ok(new_position)
    }

    fn should_seek(&mut self, stream: &S, position: u64) -> io::Result<bool> {
        if !stream.supports_seek() {
            warn!("Attempting to seek, but it's unsupported. Waiting for stream to catch up.");
            return Ok(false);
        }
        Ok(if let Some(range) = self.downloaded.get(position) {
            !range.contains(&self.writer.stream_position()?)
        } else {
            true
        })
    }

    async fn seek(&mut self, stream: &mut S, start: u64, end: Option<u64>) -> io::Result<()> {
        stream.seek_range(start, end).await?;
        self.writer.seek(SeekFrom::Start(start))?;
        Ok(())
    }

    fn signal_download_complete(&self) {
        self.position_reached.notify_stream_done();
    }

    fn report_progress(&mut self, stream: &S, info: StreamState) {
        if let Some(on_progress) = self.on_progress.as_mut() {
            on_progress(stream, info, &self.cancellation_token);
        }
    }

    fn report_prefetch_progress(
        &mut self,
        stream: &S,
        stream_position: u64,
        download_start: Instant,
        chunk_size: usize,
    ) {
        self.report_progress(
            stream,
            StreamState {
                current_position: stream_position,
                current_chunk: (0..stream_position),
                elapsed: download_start.elapsed(),
                phase: StreamPhase::Prefetching {
                    target: self.prefetch_bytes,
                    chunk_size,
                },
            },
        );
    }

    fn report_downloading_progress(
        &mut self,
        stream: &S,
        new_position: u64,
        download_start: Instant,
        chunk_size: usize,
    ) -> io::Result<()> {
        let pos = self.writer.stream_position()?;
        self.report_progress(
            stream,
            StreamState {
                current_position: pos,
                current_chunk: self
                    .downloaded
                    .get(new_position - 1)
                    .expect("position already downloaded"),
                elapsed: download_start.elapsed(),
                phase: StreamPhase::Downloading { chunk_size },
            },
        );
        Ok(())
    }

    fn report_download_complete(&mut self, stream: &S, download_start: Instant) -> io::Result<()> {
        let pos = self.writer.stream_position()?;
        self.report_progress(
            stream,
            StreamState {
                current_position: pos,
                elapsed: download_start.elapsed(),
                // ensure no subtraction overflow
                current_chunk: self.downloaded.get(pos.max(1) - 1).unwrap_or_default(),
                phase: StreamPhase::Complete,
            },
        );
        Ok(())
    }

    pub(crate) fn source_handle(&self) -> SourceHandle {
        SourceHandle {
            downloaded: self.downloaded.clone(),
            download_status: self.download_status.clone(),
            requested_position: self.requested_position.clone(),
            notify_read: self.notify_read.clone(),
            position_reached: self.position_reached.clone(),
            seek_tx: self.seek_tx.clone(),
            content_length: self.content_length,
        }
    }
}
