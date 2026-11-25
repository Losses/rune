use std::fmt::Debug;
use std::ops::Range;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use parking_lot::{Condvar, Mutex, RwLock};
use rangemap::RangeSet;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{Notify, mpsc};
use tracing::{debug, error};

#[derive(Debug, Clone)]
pub(crate) struct SourceHandle {
    pub(super) downloaded: Downloaded,
    pub(super) download_status: DownloadStatus,
    pub(super) requested_position: RequestedPosition,
    pub(super) position_reached: PositionReached,
    pub(super) content_length: Option<u64>,
    pub(super) seek_tx: mpsc::Sender<u64>,
    pub(super) notify_read: NotifyRead,
}

impl SourceHandle {
    pub(crate) fn get_downloaded_at_position(&self, position: u64) -> Option<Range<u64>> {
        self.downloaded.get(position)
    }

    pub(crate) fn contiguous_end(&self, position: u64) -> u64 {
        self.downloaded.contiguous_end(position)
    }

    pub(crate) fn is_stream_complete(&self) -> bool {
        self.download_status.is_failed() || self.position_reached.is_stream_done()
    }

    pub(crate) fn wait_for_position(&self, start: u64, requested_position: u64) {
        self.request_position(start, requested_position);
        debug!(
            requested_position = requested_position,
            "waiting for requested position"
        );
        self.notify_read.notify_waiting();
        self.position_reached.wait_for_position_reached();
    }

    pub(crate) fn seek(&self, seek_position: u64) {
        self.request_position(seek_position, seek_position);
        self.request_seek(seek_position);
        debug!(
            requested_position = seek_position,
            "waiting for requested position"
        );
        self.position_reached.wait_for_position_reached();
    }

    pub(crate) fn notify_read(&self) {
        self.notify_read.notify_read();
    }

    pub(crate) fn content_length(&self) -> Option<u64> {
        self.content_length
    }

    pub(crate) fn is_failed(&self) -> bool {
        self.download_status.is_failed()
    }

    fn request_position(&self, start: u64, end: u64) {
        self.requested_position.set(start, end);
    }

    fn request_seek(&self, position: u64) {
        self.seek_tx
            .try_send(position)
            .inspect_err(|e| {
                if let TrySendError::Full(_) = e {
                    error!("Sent multiple seek requests without waiting");
                }
            })
            .ok();
    }
}

/// A position the reader is waiting for, as the inclusive start and exclusive end of the
/// range it is about to read. A seek stores the position twice. Writes check the whole
/// range for gaps, not only whether the stream passed its end: a disjoint write past the
/// end must not wake the reader over an unfilled hole, and a write that fills the hole
/// must wake it even when the write itself stops before the end.
///
/// `start` is the presence marker: clearing moves the end out first, so a writer that
/// sees a request at all sees either its whole range or an end that fails the check.
#[derive(Debug, Clone)]
pub(super) struct RequestedPosition {
    start: Arc<AtomicU64>,
    end: Arc<AtomicU64>,
}

const NO_REQUESTED_POSITION: u64 = u64::MAX;

impl Default for RequestedPosition {
    fn default() -> Self {
        Self {
            start: Arc::new(AtomicU64::new(NO_REQUESTED_POSITION)),
            end: Arc::new(AtomicU64::new(NO_REQUESTED_POSITION)),
        }
    }
}

impl RequestedPosition {
    /// Clears the request if it still waits for `start` and `end`, so a newer request is
    /// kept.
    pub(super) fn clear(&self, start: u64, end: u64) {
        if end != self.end.load(Ordering::SeqCst) {
            return;
        }
        if self
            .start
            .compare_exchange(
                start,
                NO_REQUESTED_POSITION,
                Ordering::SeqCst,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            self.end.store(NO_REQUESTED_POSITION, Ordering::SeqCst);
        }
    }

    /// The start of the reader's range and the position one past its last byte.
    pub(super) fn get_range(&self) -> Option<(u64, u64)> {
        let start = self.start.load(Ordering::SeqCst);
        if start == NO_REQUESTED_POSITION {
            return None;
        }
        let end = self.end.load(Ordering::SeqCst);
        if end == NO_REQUESTED_POSITION {
            return None;
        }
        Some((start, end))
    }

    fn set(&self, start: u64, end: u64) {
        self.end.store(end, Ordering::SeqCst);
        self.start.store(start, Ordering::SeqCst);
    }
}

#[derive(Default, Debug)]
struct Waiter {
    position_reached: bool,
    stream_done: bool,
}

// parking_lot's synchronization primitive's aren't unwind safe: https://github.com/Amanieu/parking_lot/issues/32
// Libraries that use FFI sometimes require these traits, which can cause compilation failures for
// anything that tries to hold a reference to this struct.
// Here we can use AssertUnwindSafe as long as we ensure we don't panic while holding a lock.
#[derive(Default, Clone, Debug)]
pub(super) struct PositionReached(Arc<(AssertUnwindSafe<Mutex<Waiter>>, Condvar)>);

impl PositionReached {
    pub(super) fn notify_position_reached(&self) {
        let (mutex, cvar) = self.0.as_ref();
        mutex.lock().position_reached = true;
        cvar.notify_all();
    }

    pub(super) fn notify_stream_done(&self) {
        let (mutex, cvar) = self.0.as_ref();
        mutex.lock().stream_done = true;
        cvar.notify_all();
    }

    pub(super) fn is_stream_done(&self) -> bool {
        let (mutex, _) = self.0.as_ref();
        mutex.lock().stream_done
    }

    fn wait_for_position_reached(&self) {
        let (mutex, cvar) = self.0.as_ref();
        let mut waiter = mutex.lock();
        if waiter.stream_done {
            return;
        }

        let wait_start = Instant::now();

        cvar.wait_while(&mut waiter, |waiter| {
            !waiter.stream_done && !waiter.position_reached
        });
        debug!(
            elapsed = format!("{:?}", wait_start.elapsed()),
            "position reached"
        );
        if !waiter.stream_done {
            waiter.position_reached = false;
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct Downloaded(Arc<AssertUnwindSafe<RwLock<RangeSet<u64>>>>);

impl Downloaded {
    pub(super) fn add(&self, range: Range<u64>) {
        // explicit check here prevents panic
        if range.end > range.start {
            self.0.write().insert(range);
        }
    }

    pub(super) fn get(&self, position: u64) -> Option<Range<u64>> {
        self.0.read().get(&position).cloned()
    }

    pub(super) fn next_gap(&self, range: Range<u64>) -> Option<Range<u64>> {
        self.0.read().gaps(&range).next()
    }

    /// The end of the uninterrupted run of downloaded bytes at `position`, or `position`
    /// itself when nothing is downloaded there.
    pub(super) fn contiguous_end(&self, position: u64) -> u64 {
        match self.get(position) {
            Some(range) => range.end,
            None => position,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct NotifyRead {
    notify: Arc<Notify>,
    write_requested: Arc<AtomicBool>,
}

impl NotifyRead {
    pub(super) fn request(&self) {
        self.write_requested.store(true, Ordering::SeqCst);
    }

    fn notify_read(&self) {
        if self.write_requested.swap(false, Ordering::SeqCst) {
            self.notify.notify_one();
        }
    }

    fn notify_waiting(&self) {
        if self.write_requested.load(Ordering::SeqCst) {
            self.notify.notify_one();
        }
    }

    pub(super) async fn wait_for_read(&self) {
        self.notify.notified().await;
    }
}

#[derive(Default, Clone, Debug)]
pub(super) struct DownloadStatus(Arc<AtomicBool>);

impl DownloadStatus {
    pub(super) fn set_failed(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn is_failed(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}
