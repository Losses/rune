use std::convert::Infallible;
use std::future::Future;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::pin::Pin;
use std::sync::Arc;
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
use crate::storage::bounded::BoundedStorageProvider;
use crate::storage::memory::{MemoryStorage, MemoryStorageProvider};
use crate::storage::temp::TempStorageProvider;
use crate::{Settings, StreamDownload};

enum TestStream {
    Pending,
    Ranges(Vec<(u64, Option<u64>)>),
    Panic(oneshot::Receiver<()>),
    Controlled(ControlledStream),
}

type SeekLog = Arc<std::sync::Mutex<Vec<(u64, Option<u64>)>>>;

struct ControlledStream {
    chunks: tokio::sync::mpsc::UnboundedReceiver<Option<Bytes>>,
    seeks: SeekLog,
    content_length: Option<u64>,
}

/// A stream the test drives by hand: `tx` feeds chunks, `None` ends the stream, and every
/// seek the downloader performs lands in `seeks`.
fn controlled_stream(
    content_length: Option<u64>,
) -> (
    TestStream,
    tokio::sync::mpsc::UnboundedSender<Option<Bytes>>,
    SeekLog,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let seeks = Arc::new(std::sync::Mutex::new(Vec::new()));
    let stream = TestStream::Controlled(ControlledStream {
        chunks: rx,
        seeks: seeks.clone(),
        content_length,
    });
    (stream, tx, seeks)
}

impl Stream for TestStream {
    type Item = io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.get_mut() {
            Self::Panic(trigger) => {
                if Pin::new(trigger).poll(cx).is_ready() {
                    panic!("injected downloader panic");
                }
                Poll::Pending
            }
            Self::Controlled(controlled) => controlled
                .chunks
                .poll_recv(cx)
                .map(|chunk| chunk.flatten().map(Ok)),
            _ => Poll::Pending,
        }
    }
}

impl SourceStream for TestStream {
    type Params = ();
    type StreamCreationError = Infallible;

    async fn create((): ()) -> Result<Self, Infallible> {
        Ok(Self::Pending)
    }

    fn content_length(&self) -> Option<u64> {
        match self {
            Self::Controlled(controlled) => controlled.content_length,
            _ => Some(8),
        }
    }

    async fn seek_range(&mut self, start: u64, end: Option<u64>) -> io::Result<()> {
        match self {
            Self::Ranges(ranges) => {
                ranges.push((start, end));
                Ok(())
            }
            Self::Controlled(controlled) => {
                controlled
                    .seeks
                    .lock()
                    .expect("test operation")
                    .push((start, end));
                Ok(())
            }
            _ => panic!("cached seek must not request remote data"),
        }
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
        while source.requested_position.get_range() != Some((position, position)) {
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
        while requested.get_range() != Some((0, 1)) {
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

/// The reader's part of #538: a read that spans an unfilled hole must not return when a
/// write to a disjoint range past its end arrives, and must return exactly the stream's
/// bytes once the hole is filled. On unpatched 0.24.5 the disjoint write wakes the read,
/// which returns the hole's zeros at once, so the first wait below does not time out.
#[tokio::test]
async fn read_across_gap_waits_for_the_hole_and_returns_stream_bytes() {
    let mut content = vec![0u8; 200];
    for (i, byte) in content.iter_mut().enumerate() {
        *byte = (i + 1) as u8;
    }
    let content = content;

    let (stream, tx, seeks) = controlled_stream(Some(200));
    let mut download = StreamDownload::from_stream(
        stream,
        MemoryStorageProvider,
        Settings::default().prefetch_bytes(0),
    )
    .await
    .expect("create test stream");
    let seeker_handle = download.handle.clone();

    // Bytes 0..50 arrive; a forward jump of another consumer takes the downloader to 150,
    // skipping 50..150. With prefetch off the downloader is waiting for chunks, so the
    // seek it picks up moves its next writes past the read's end.
    tx.send(Some(Bytes::copy_from_slice(&content[0..50])))
        .expect("send prefix chunk");
    timeout(Duration::from_secs(1), async {
        while download
            .handle
            .get_downloaded_at_position(0)
            .is_none_or(|range| range.end < 50)
        {
            yield_now().await;
        }
    })
    .await
    .expect("peer step timed out: prefix download");
    seeker_handle.seek_tx.try_send(150).expect("queue jump");

    // The reader asks for 0..100, across the hole at 50..150.
    let (finished, mut reader_finished) = oneshot::channel();
    let reader = thread::spawn(move || {
        let mut buf = vec![0u8; 100];
        let result = download.read(&mut buf);
        let _ = finished.send((result, buf, download));
    });
    // Let the reader thread reach its wait before the disjoint write lands.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 150..200 arrives, disjoint from the read's range and past its end. It must not
    // finish the read: on 0.24.5 it does, over the hole.
    tokio::time::sleep(Duration::from_millis(50)).await;
    tx.send(Some(Bytes::copy_from_slice(&content[150..200])))
        .expect("send tail chunk");
    tokio::time::sleep(Duration::from_millis(50)).await;
    let premature = timeout(Duration::from_millis(250), &mut reader_finished).await;
    assert!(
        premature.is_err(),
        "the read returned while the hole at 50..100 was still missing"
    );
    let seek_positions = seeks.lock().expect("test operation").clone();
    assert!(
        seek_positions.iter().any(|&(start, _)| start == 150),
        "the jump must restart the stream at 150, got {seek_positions:?}"
    );

    // The consumer at the tail jumps back over the hole, so the next chunk lands in it.
    // Filling the hole completes the read with exactly the stream's bytes, even though
    // the write ends at 100, before the read's end position reached the writer.
    seeker_handle.seek_tx.try_send(50).expect("queue jump back");
    tokio::time::sleep(Duration::from_millis(50)).await;
    tx.send(Some(Bytes::copy_from_slice(&content[50..100])))
        .expect("send hole bytes");
    let (result, buf, download) = timeout(Duration::from_secs(1), reader_finished)
        .await
        .expect("peer step timed out: read after the hole was filled")
        .expect("reader result");
    reader.join().expect("reader thread");
    let read = result.expect("read must succeed once the range is whole");
    assert_eq!(read, 100, "the read must fill the buffer");
    assert_eq!(
        buf,
        content[0..100],
        "the read must return the stream's bytes, not the hole's zeros"
    );
    drop(download);
}

/// The completion half of #538: a stream of unknown length can end with a hole that is
/// never re-downloaded, because the downloader looks for missing ranges only when the
/// length is known. The read must end with the uninterrupted run at its position; past
/// it, sparse storage reads back zeros that were never received. On unpatched 0.24.5 the
/// read below returns all 100 bytes with the hole and the tail in them.
#[tokio::test]
async fn read_at_eof_with_unfilled_hole_stops_at_the_hole() {
    let mut content = vec![0u8; 200];
    for (i, byte) in content.iter_mut().enumerate() {
        *byte = (i + 1) as u8;
    }
    let content = content;

    let (stream, tx, _seeks) = controlled_stream(None);
    let mut download = StreamDownload::from_stream(
        stream,
        TempStorageProvider::new(),
        Settings::default().prefetch_bytes(0),
    )
    .await
    .expect("create test stream");
    let seeker_handle = download.handle.clone();

    // 0..50 arrives; a forward jump takes the downloader to 150, and the stream ends
    // after the tail, so 50..150 is never downloaded.
    tx.send(Some(Bytes::copy_from_slice(&content[0..50])))
        .expect("send prefix chunk");
    timeout(Duration::from_secs(1), async {
        while download
            .handle
            .get_downloaded_at_position(0)
            .is_none_or(|range| range.end < 50)
        {
            yield_now().await;
        }
    })
    .await
    .expect("peer step timed out: prefix download");
    seeker_handle.seek_tx.try_send(150).expect("queue jump");
    tokio::time::sleep(Duration::from_millis(50)).await;
    tx.send(Some(Bytes::copy_from_slice(&content[150..200])))
        .expect("send tail chunk");
    tx.send(None).expect("end the stream");
    timeout(
        Duration::from_secs(1),
        download.handle().wait_for_completion(),
    )
    .await
    .expect("peer step timed out: stream completion");

    // The read over the hole must end at 50 with exactly the downloaded prefix, never the
    // hole's zeros or bytes from the disjoint tail.
    let mut buf = vec![0u8; 100];
    let read = timeout(Duration::from_secs(1), async {
        tokio::task::spawn_blocking(move || {
            let read = download.read(&mut buf);
            (read, buf)
        })
        .await
        .expect("reader task")
    })
    .await
    .expect("peer step timed out: read at eof with hole");
    let (result, buf) = read;
    let read = result.expect("read must succeed");
    assert_eq!(
        read, 50,
        "the read must stop at the hole, not return its zeros"
    );
    assert_eq!(
        buf[..50],
        content[0..50],
        "the read must return the downloaded prefix"
    );
}

/// #540: cancelling a download whose writer waits for ring-buffer space must finish the
/// task's cleanup without another read. On unpatched master the writer's inner
/// `wait_for_read` cannot observe the cancellation, so the download task never reaches
/// its cleanup: the stream never reports completion, and readers are never released by
/// it. The cancellation token the completion handle waits on fires on cancel even while
/// the task is still stuck, so the cleanup is observed through the stream completion it
/// signals.
///
/// Setup: a ring of 8 and a chunk of 24 bytes, which cannot fit in the ring at once.
/// Once the reader drains the first ringful the writer fills it again and stalls inside
/// its write loop with positions 16..24 of its chunk unwritten, because freeing a slot
/// needs a read, and the frozen consumer never reads again.
#[tokio::test]
async fn cancellation_finishes_a_writer_waiting_for_space() {
    use std::num::NonZeroUsize;

    let mut content = vec![0u8; 32];
    for (i, byte) in content.iter_mut().enumerate() {
        *byte = (i + 1) as u8;
    }
    let content = content;

    let (stream, tx, _seeks) = controlled_stream(Some(32));
    let mut download = StreamDownload::from_stream(
        stream,
        BoundedStorageProvider::new(MemoryStorageProvider, NonZeroUsize::new(8).expect("nonzero")),
        Settings::default().prefetch_bytes(0),
    )
    .await
    .expect("create test stream");
    let download_handle = download.handle.clone();
    let completion = download.handle();
    let cancel = download.cancellation_token();

    tx.send(Some(Bytes::copy_from_slice(&content[0..24])))
        .expect("send chunk larger than the ring");

    // The reader drains exactly the first ringful, then the consumer stops: no further
    // read can ever free a slot for the writer.
    let (finished, reader_finished) = oneshot::channel();
    let reader = thread::spawn(move || {
        let mut received = Vec::new();
        while received.len() < 8 {
            let mut buf = [0u8; 1];
            match download.read(&mut buf) {
                Ok(0) => break,
                Ok(read) => received.extend_from_slice(&buf[..read]),
                Err(e) => panic!("the first reads must succeed: {e}"),
            }
        }
        let _ = finished.send((received, download));
    });

    // Once the first ringful is drained the writer fills it again and stalls: positions
    // 16..24 of its chunk remain unwritten. The freeze check proves the writer is inside
    // its write loop waiting for a read, not waiting for the next chunk in the outer
    // select, which observes cancellation on unpatched master too.
    timeout(Duration::from_secs(2), async {
        loop {
            let end = download_handle
                .get_downloaded_at_position(0)
                .map_or(0, |range| range.end);
            if end >= 16 {
                break;
            }
            yield_now().await;
        }
    })
    .await
    .expect("peer step timed out: writer filling the ring ahead of the reader");
    // Let the writer settle into its wait for a read.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let end_at_cancel = download_handle
        .get_downloaded_at_position(0)
        .map_or(0, |range| range.end);
    assert!(
        end_at_cancel < 24,
        "the writer must still hold unwritten positions of its chunk at cancellation, \
         got end {end_at_cancel}"
    );

    cancel.cancel();

    timeout(Duration::from_secs(1), async {
        while !download_handle.is_stream_complete() {
            yield_now().await;
        }
    })
    .await
    .expect(
        "the download task must finish its cleanup after cancellation instead of waiting          for a read that can never happen",
    );
    timeout(Duration::from_secs(1), completion.wait_for_completion())
        .await
        .expect("peer step timed out: download completion");

    let (received, download) = timeout(Duration::from_secs(1), reader_finished)
        .await
        .expect("the draining reader must finish")
        .expect("reader result");
    reader.join().expect("reader thread");
    assert_eq!(received.len(), 8, "the reader drained the first ringful");
    drop(download);
}

/// Control for the same backpressure scenario without a cancellation: the second chunk
/// fills the ring exactly and the writer stalls, then reads that free space must resume
/// the writer, and the download must complete with the whole stream.
#[tokio::test]
async fn a_read_resumes_a_writer_waiting_for_space() {
    use std::num::NonZeroUsize;

    let mut first = vec![0u8; 32];
    for (i, byte) in first.iter_mut().enumerate() {
        *byte = (i + 1) as u8;
    }
    let mut tail = vec![0u8; 8];
    for (i, byte) in tail.iter_mut().enumerate() {
        *byte = (i + 33) as u8;
    }
    let mut content = first.clone();
    content.extend_from_slice(&tail);

    let (stream, tx, _seeks) = controlled_stream(Some(40));
    let mut download = StreamDownload::from_stream(
        stream,
        BoundedStorageProvider::new(MemoryStorageProvider, NonZeroUsize::new(8).expect("nonzero")),
        Settings::default().prefetch_bytes(0),
    )
    .await
    .expect("create test stream");
    let download_handle = download.handle.clone();
    let completion = download.handle();

    tx.send(Some(Bytes::copy_from_slice(&first)))
        .expect("send first chunk");

    // The reader consumes the whole stream one byte at a time.
    let content_len = content.len();
    let (finished, reader_finished) = oneshot::channel();
    let reader = thread::spawn(move || {
        let mut received = Vec::new();
        let mut buf = [0u8; 1];
        while received.len() < content_len {
            match download.read(&mut buf) {
                Ok(0) => break,
                Ok(read) => received.extend_from_slice(&buf[..read]),
                Err(e) => panic!("reads must succeed: {e}"),
            }
        }
        let _ = finished.send((received, download));
    });

    // Once the writer passes 16, the reader has drained the first ringful and the ring
    // holds exactly 16..24. The second chunk stalls the writer at 24: no read is possible
    // until 8 is written, and writing 8 needs a read. The reader's reads resume it.
    timeout(Duration::from_secs(2), async {
        loop {
            let end = download_handle
                .get_downloaded_at_position(0)
                .map_or(0, |range| range.end);
            if end >= 16 {
                break;
            }
            yield_now().await;
        }
    })
    .await
    .expect("peer step timed out: writer passing the first ringful");
    tx.send(Some(Bytes::copy_from_slice(&tail)))
        .expect("send second chunk");
    tx.send(None).expect("end the stream");

    let (received, download) = timeout(Duration::from_secs(2), reader_finished)
        .await
        .expect("peer step timed out: reads resuming the writer through the full ring")
        .expect("reader result");
    reader.join().expect("reader thread");
    assert_eq!(received, content, "the reads return the whole stream");
    timeout(Duration::from_secs(1), completion.wait_for_completion())
        .await
        .expect("peer step timed out: download completion");
    drop(download);
}
