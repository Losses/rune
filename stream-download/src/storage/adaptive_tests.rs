use std::convert::Infallible;
use std::io::{self, ErrorKind, Read, Seek, SeekFrom};
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread;
use std::time::Duration;

use bytes::Bytes;
use futures_util::Stream;
use tokio::sync::oneshot;
use tokio::time::timeout;

use super::adaptive::AdaptiveStorageProvider;
use super::memory::MemoryStorageProvider;
use crate::source::SourceStream;
use crate::{Settings, StreamDownload};

const CHUNK: u64 = 4;

/// Yields a counting byte pattern in small chunks, ending after `length` bytes when it is known.
struct TestStream {
    length: Option<u64>,
    sent: u64,
}

impl TestStream {
    fn new(length: Option<u64>) -> Self {
        Self { length, sent: 0 }
    }
}

impl Stream for TestStream {
    type Item = io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let remaining = this.length.map_or(CHUNK, |length| length - this.sent);
        if remaining == 0 {
            return Poll::Ready(None);
        }
        let chunk = remaining.min(CHUNK);
        let bytes = (this.sent..this.sent + chunk)
            .map(pattern)
            .collect::<Vec<_>>();
        this.sent += chunk;
        Poll::Ready(Some(Ok(Bytes::from(bytes))))
    }
}

impl SourceStream for TestStream {
    type Params = ();
    type StreamCreationError = Infallible;

    async fn create((): ()) -> Result<Self, Infallible> {
        Ok(Self::new(None))
    }

    fn content_length(&self) -> Option<u64> {
        self.length
    }

    async fn seek_range(&mut self, _: u64, _: Option<u64>) -> io::Result<()> {
        panic!("the test stream does not seek");
    }

    async fn reconnect(&mut self, _: u64) -> io::Result<()> {
        panic!("the test stream does not reconnect");
    }

    fn supports_seek(&self) -> bool {
        false
    }
}

fn pattern(position: u64) -> u8 {
    (position % 251) as u8
}

type Adaptive = AdaptiveStorageProvider<MemoryStorageProvider, MemoryStorageProvider>;

async fn adaptive_stream(length: Option<u64>, buffer_size: usize) -> StreamDownload<Adaptive> {
    let buffer_size = NonZeroUsize::new(buffer_size).expect("nonzero buffer size");
    StreamDownload::from_stream(
        TestStream::new(length),
        AdaptiveStorageProvider::new(MemoryStorageProvider, buffer_size),
        Settings::default().prefetch_bytes(0),
    )
    .await
    .expect("create test stream")
}

/// Runs `operation` against the stream on its own thread so a stalled read cannot hang the test.
/// A stalled thread is abandoned after the download is cancelled; it cannot be joined while a
/// bounded writer is waiting for the reader.
async fn run_with_deadline<T: Send + 'static>(
    stream: StreamDownload<Adaptive>,
    operation: impl FnOnce(&mut StreamDownload<Adaptive>) -> T + Send + 'static,
) -> T {
    let cancel = stream.cancellation_token();
    let (finished, result) = oneshot::channel();
    let thread = thread::spawn(move || {
        let mut stream = stream;
        let _ = finished.send(operation(&mut stream));
    });
    let Ok(result) = timeout(Duration::from_secs(2), result).await else {
        cancel.cancel();
        panic!("operation did not finish within the deadline");
    };
    thread.join().expect("operation thread");
    result.expect("operation result")
}

#[tokio::test]
async fn unknown_length_read_larger_than_the_buffer_is_rejected() {
    let stream = adaptive_stream(None, 4).await;
    let result = run_with_deadline(stream, |stream| stream.read(&mut [0; 8])).await;
    assert_eq!(
        result.expect_err("oversized read").kind(),
        ErrorKind::InvalidInput
    );
}

#[tokio::test]
async fn unknown_length_seek_beyond_the_buffer_is_rejected() {
    let stream = adaptive_stream(None, 4).await;
    let result = run_with_deadline(stream, |stream| stream.seek(SeekFrom::Start(5))).await;
    assert_eq!(
        result.expect_err("oversized seek").kind(),
        ErrorKind::InvalidInput
    );
}

#[tokio::test]
async fn unknown_length_read_of_the_buffer_size_returns_data() {
    let stream = adaptive_stream(None, 4).await;
    let (read, buf) = run_with_deadline(stream, |stream| {
        let mut buf = [0; 4];
        (stream.read(&mut buf).expect("read within capacity"), buf)
    })
    .await;
    assert!(read > 0);
    assert!(
        buf[..read]
            .iter()
            .zip(0..)
            .all(|(&byte, i)| byte == pattern(i))
    );
}

#[tokio::test]
async fn finite_stream_within_the_buffer_reads_more_than_the_buffer_size() {
    let stream = adaptive_stream(Some(3), 4).await;
    let read = run_with_deadline(stream, |stream| stream.read(&mut [0; 8])).await;
    assert_eq!(read.expect("read of a fixed-branch stream"), 3);
}

#[tokio::test]
async fn finite_stream_beyond_the_buffer_reads_more_than_the_buffer_size() {
    let stream = adaptive_stream(Some(10), 4).await;
    let read = run_with_deadline(stream, |stream| stream.read(&mut [0; 8])).await;
    assert_eq!(read.expect("read of a variable-branch stream"), 8);
}
