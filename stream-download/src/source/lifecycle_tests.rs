use std::convert::Infallible;
use std::future::Future;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::Stream;
use tokio::sync::oneshot;
use tokio::task::yield_now;
use tokio::time::{error::Elapsed, timeout};
use tokio_util::sync::CancellationToken;

use super::{Source, SourceStream};
use crate::storage::StorageProvider;
use crate::storage::memory::{MemoryStorage, MemoryStorageProvider};
use crate::{Settings, StreamDownload};

enum TestStream {
    Pending,
    Ranges(Vec<(u64, Option<u64>)>),
    Panic(oneshot::Receiver<()>),
}

impl Stream for TestStream {
    type Item = io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Self::Panic(trigger) = self.get_mut() {
            if Pin::new(trigger).poll(cx).is_ready() {
                panic!("injected downloader panic");
            }
        }
        Poll::Pending
    }
}

impl SourceStream for TestStream {
    type Params = ();
    type StreamCreationError = Infallible;

    async fn create(_: ()) -> Result<Self, Infallible> {
        Ok(Self::Pending)
    }

    fn content_length(&self) -> Option<u64> {
        Some(8)
    }

    async fn seek_range(&mut self, start: u64, end: Option<u64>) -> io::Result<()> {
        if let Self::Ranges(ranges) = self {
            ranges.push((start, end));
            return Ok(());
        }
        panic!("cached seek must not request remote data");
    }

    async fn reconnect(&mut self, _: u64) -> io::Result<()> {
        panic!("cached seek must not reconnect");
    }

    fn supports_seek(&self) -> bool {
        true
    }
}

fn source_with_downloaded(
    writer_position: u64,
    downloaded: Range<u64>,
) -> Source<TestStream, MemoryStorage> {
    let (_, mut writer) = MemoryStorageProvider
        .into_reader_writer(Some(8))
        .expect("test operation");
    writer.write_all(b"abcdefgh").expect("test operation");
    writer
        .seek(SeekFrom::Start(writer_position))
        .expect("test operation");
    let source = Source::new(
        writer,
        Some(8),
        Settings::default().prefetch_bytes(0),
        CancellationToken::new(),
    );
    source.downloaded.add(downloaded);
    source
}

fn start_seek(
    source: &Source<TestStream, MemoryStorage>,
    position: u64,
) -> (JoinHandle<()>, oneshot::Receiver<()>) {
    let handle = source.source_handle();
    let (finished, reader_finished) = oneshot::channel();
    let reader = thread::spawn(move || {
        handle.seek(position);
        let _ = finished.send(());
    });
    (reader, reader_finished)
}

async fn wait_for_seek(
    source: &Source<TestStream, MemoryStorage>,
    position: u64,
) -> Result<(), Elapsed> {
    timeout(Duration::from_secs(1), async {
        while source.requested_position.get() != Some(position) {
            yield_now().await;
        }
    })
    .await
}

#[tokio::test]
async fn queued_seek_after_last_write_has_no_missing_gap() {
    let mut source = source_with_downloaded(8, 0..8);
    source
        .handle_seek(&mut TestStream::Pending, 3)
        .await
        .expect("test operation");
}

#[tokio::test]
async fn seek_to_eof_has_no_missing_gap() {
    let mut source = source_with_downloaded(8, 0..8);
    source
        .handle_seek(&mut TestStream::Pending, 8)
        .await
        .expect("test operation");
}

#[tokio::test]
async fn cached_seek_wakes_reader_without_another_chunk() {
    let mut source = source_with_downloaded(3, 0..8);
    let (reader, mut reader_finished) = start_seek(&source, 3);
    let queued = wait_for_seek(&source, 3).await;
    if queued.is_ok() {
        source
            .handle_seek(&mut TestStream::Pending, 3)
            .await
            .expect("test operation");
    }
    let result = timeout(Duration::from_millis(250), &mut reader_finished).await;
    source.signal_download_complete();
    reader.join().expect("test operation");
    queued.expect("peer step timed out: queue cached seek");
    result
        .expect("peer step timed out: cached seek wakeup")
        .expect("test operation");
}

#[tokio::test]
async fn downloader_panic_wakes_reader_and_finishes_handle() {
    let (trigger, panic_when_triggered) = oneshot::channel();
    let mut stream = StreamDownload::from_stream(
        TestStream::Panic(panic_when_triggered),
        MemoryStorageProvider,
        Settings::default().prefetch_bytes(0),
    )
    .await
    .expect("create test stream");
    let requested = stream.handle.requested_position.clone();
    let handle = stream.handle();
    let (finished, reader_finished) = oneshot::channel();
    let reader = thread::spawn(move || {
        let result = stream.read(&mut [0; 1]);
        let _ = finished.send((result, stream));
    });
    timeout(Duration::from_secs(1), async {
        while requested.get() != Some(1) {
            yield_now().await;
        }
    })
    .await
    .expect("peer step timed out: queue read before downloader panic");
    trigger.send(()).expect("trigger downloader panic");
    let (result, _stream) = timeout(Duration::from_secs(1), reader_finished)
        .await
        .expect("peer step timed out: reader after downloader panic")
        .expect("reader result");
    reader.join().expect("reader thread");
    assert!(result.is_err(), "downloader panic must fail the read");
    timeout(Duration::from_secs(1), handle.wait_for_completion())
        .await
        .expect("peer step timed out: download handle after panic");
}

#[tokio::test]
async fn zero_prefetch_seek_wakes_when_tail_arrives_before_missing_prefix() {
    let mut source = source_with_downloaded(0, 0..0);
    let mut stream = TestStream::Ranges(Vec::new());
    let (reader, mut reader_finished) = start_seek(&source, 6);
    let queued = wait_for_seek(&source, 6).await;
    if queued.is_ok() {
        source.handle_seek(&mut stream, 6).await.expect("seek tail");
        source
            .handle_bytes(
                &mut stream,
                Some(Ok(Bytes::from_static(b"gh"))),
                Instant::now(),
            )
            .await
            .expect("publish tail bytes");
    }
    // The next range response can stall; no prefix bytes or EOF have arrived.
    let result = timeout(Duration::from_millis(250), &mut reader_finished).await;
    source.signal_download_complete();
    reader.join().expect("tail seeker thread");
    queued.expect("peer step timed out: queue tail seek");
    result
        .expect("peer step timed out: zero-prefetch tail seek")
        .expect("tail seeker result");
}
