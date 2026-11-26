//! The covers of a streaming service's library, downloaded by the ids the service gives them and
//! kept on disk.

use std::collections::{HashMap, HashSet};
use std::error;
use std::fmt;
use std::fs;
use std::future;
use std::io;
use std::path::{Path, PathBuf};
use std::result;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use anyhow::{Error, Result};
use futures::future::BoxFuture;
use futures::{StreamExt, stream};
use tokio::fs as tokio_fs;
use tokio::sync::{Notify, oneshot, watch};
use tokio::task;
use tokio::time as tokio_time;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    messages::LibraryContentChanged, service::folder::service_library_folder, utils::Broadcaster,
};

/// Downloads cover images by the ids a service gives them.
pub(crate) trait CoverSource: Send + Sync + 'static {
    fn cover_bytes<'a>(&'a self, cover_id: &'a str) -> BoxFuture<'a, Result<Vec<u8>>>;
}

/// How many covers download at once.
const COVER_DOWNLOADS: usize = 4;

/// How many scheduled downloads may wait for their turn. Past it the oldest are dropped: a page
/// that still shows them schedules them again.
const MAX_WAITING_SCHEDULED: usize = 512;

/// How long finished covers are gathered into one announcement.
const CHANGE_WINDOW: Duration = Duration::from_millis(500);

/// The largest cover a service may send.
pub(crate) const MAX_COVER_BYTES: usize = 8 * 1024 * 1024;

/// Where the cover cache of one service library lives: in the library's own folder.
pub(crate) fn cover_cache_root(config_dir: &Path, library_id: Uuid) -> PathBuf {
    service_library_folder(config_dir, library_id).join("covers")
}

/// The covers of one service library, downloaded once and kept on disk.
pub(crate) struct ServiceCoverCache {
    pub(crate) root: PathBuf,
    pending: Mutex<HashMap<String, Pending>>,
    downloads: Arc<DownloadQueue>,
    pub(crate) cancellation: CancellationToken,
    pub(crate) broadcaster: Arc<dyn Broadcaster>,
    /// Whether a change announcement is waiting out its window.
    change_pending: Arc<AtomicBool>,
    /// The request that waits for the cover of what plays now, with that cover's id.
    playing: Mutex<Option<(String, task::AbortHandle)>>,
}

/// A download in flight or waiting for its turn, which requests for the same cover join.
struct Pending {
    receiver: watch::Receiver<Option<Fetched>>,
    /// How many requests that wait for the cover joined it. While any do, it goes ahead of the
    /// downloads that nobody waits for.
    waiters: Arc<AtomicUsize>,
}

/// How a download ended, as the requests that joined it are told.
#[derive(Clone)]
enum Fetched {
    /// It ran, or found the cover cached, or the cache was shut down.
    Done(result::Result<Option<String>, Arc<Error>>),
    /// It was dropped from the queue before its turn came.
    Dropped,
}

/// How a download that asked for a turn ended.
enum Finished {
    /// With the path of the cover, or none when there is nothing to keep.
    Done(Option<String>),
    /// It was dropped from the queue before its turn came.
    Dropped,
}

/// A download waiting in the queue.
struct Waiting {
    /// Orders the waiting downloads by when they were requested, the newest having the highest.
    order: u64,
    cover_id: String,
    waiters: Arc<AtomicUsize>,
    grant: oneshot::Sender<Slot>,
}

impl Waiting {
    /// Whether a request waits for the download.
    fn awaited(&self) -> bool {
        self.waiters.load(Ordering::SeqCst) > 0
    }
}

#[derive(Default)]
struct QueueState {
    running: usize,
    closed: bool,
    next_order: u64,
    waiting: Vec<Waiting>,
}

impl QueueState {
    /// Takes the waiting download that is to start next: one that is awaited before one that is
    /// not. The awaited ones start in the order they were asked for, so that newer ones never
    /// keep an older one waiting, and of the others the newest starts first, as it is what the
    /// user looks at.
    fn take_next(&mut self) -> Option<Waiting> {
        let index = (0..self.waiting.len()).max_by_key(|&index| {
            let waiting = &self.waiting[index];
            let awaited = waiting.awaited();
            let rank = if awaited {
                u64::MAX - waiting.order
            } else {
                waiting.order
            };
            (awaited, rank)
        })?;
        Some(self.waiting.swap_remove(index))
    }

    /// Drops the oldest waiting download that nobody awaits, while more than
    /// `MAX_WAITING_SCHEDULED` of them wait.
    fn drop_oldest_scheduled(&mut self) {
        loop {
            let scheduled = self
                .waiting
                .iter()
                .enumerate()
                .filter(|(_, waiting)| !waiting.awaited());
            if scheduled.clone().count() <= MAX_WAITING_SCHEDULED {
                return;
            }
            let Some((index, _)) = scheduled.min_by_key(|(_, waiting)| waiting.order) else {
                return;
            };
            // Its sender goes with it, which tells the download it was dropped.
            self.waiting.swap_remove(index);
        }
    }
}

/// Holds the downloads that wait for one of `COVER_DOWNLOADS` turns, and starts the next as a
/// turn ends.
struct DownloadQueue {
    state: Mutex<QueueState>,
    /// Notifies when all downloads have completed (running count reaches 0).
    all_done: Arc<Notify>,
}

/// The turn of one download, given back when it is dropped.
struct Slot {
    queue: Arc<DownloadQueue>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.queue.release();
    }
}

/// Where a download stands as it asks for a turn.
enum Turn {
    Ready(Slot),
    Waiting(oneshot::Receiver<Slot>),
    /// The queue is closed, so no download starts any more.
    Closed,
}

/// A request that waits for a cover, counted among the waiters of its download until it is
/// dropped. Once the last one stops waiting, as one does that gives up, the download waits in
/// line as a scheduled one again, within their bound.
struct Waiter {
    waiters: Arc<AtomicUsize>,
    queue: Arc<DownloadQueue>,
}

impl Waiter {
    /// Counts a request as waiting for a download, and says whether it is the only one.
    fn join(waiters: &Arc<AtomicUsize>, queue: &Arc<DownloadQueue>) -> (Self, bool) {
        let first = waiters.fetch_add(1, Ordering::SeqCst) == 0;
        let waiter = Self {
            waiters: Arc::clone(waiters),
            queue: Arc::clone(queue),
        };
        (waiter, first)
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        if self.waiters.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.queue.lock().drop_oldest_scheduled();
        }
    }
}

impl DownloadQueue {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(QueueState::default()),
            all_done: Arc::new(Notify::new()),
        })
    }

    fn lock(&self) -> MutexGuard<'_, QueueState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Asks for a turn: at once when one is free, else by waiting in line.
    fn enter(self: &Arc<Self>, cover_id: &str, waiters: &Arc<AtomicUsize>) -> Turn {
        let mut state = self.lock();
        if state.closed {
            return Turn::Closed;
        }
        if state.running < COVER_DOWNLOADS {
            state.running += 1;
            return Turn::Ready(Slot {
                queue: Arc::clone(self),
            });
        }
        let (grant, turn) = oneshot::channel();
        let order = state.next_order;
        state.next_order += 1;
        state.waiting.push(Waiting {
            order,
            cover_id: cover_id.to_owned(),
            waiters: Arc::clone(waiters),
            grant,
        });
        state.drop_oldest_scheduled();
        Turn::Waiting(turn)
    }

    /// A request for a cover whose download was asked for before, which waits in line again,
    /// counted among the download's waiters when it waits for the cover. A cover nobody awaits
    /// becomes the newest, and so does one that this request is the first to await, which joins
    /// the end of the line of awaited covers. One that was awaited already keeps its place in that
    /// line. The count and the place change under one lock, so no turn sees the one without the
    /// other.
    fn rejoin(
        self: &Arc<Self>,
        cover_id: &str,
        waiters: &Arc<AtomicUsize>,
        awaited: bool,
    ) -> Option<Waiter> {
        let mut state = self.lock();
        let (waiter, first_awaited) = if awaited {
            let (waiter, first) = Waiter::join(waiters, self);
            (Some(waiter), first)
        } else {
            (None, false)
        };
        let order = state.next_order;
        if let Some(waiting) = state
            .waiting
            .iter_mut()
            .find(|waiting| waiting.cover_id == cover_id)
            && (first_awaited || !waiting.awaited())
        {
            waiting.order = order;
            state.next_order += 1;
        }
        waiter
    }

    /// Takes the download of `cover_id` out of the line, as it was cancelled. It is found by
    /// its cover, as a request that waits in line again changes its order.
    fn withdraw(&self, cover_id: &str) {
        self.lock()
            .waiting
            .retain(|waiting| waiting.cover_id != cover_id);
    }

    /// Ends a turn and gives the free ones to the waiting downloads.
    fn release(self: &Arc<Self>) {
        // Turns whose download is gone already are given back after the lock is let go.
        let mut refused = Vec::new();
        let running_after = {
            let mut state = self.lock();
            state.running = state.running.saturating_sub(1);
            while !state.closed && state.running < COVER_DOWNLOADS {
                let Some(next) = state.take_next() else {
                    break;
                };
                state.running += 1;
                let turn = Slot {
                    queue: Arc::clone(self),
                };
                if let Err(turn) = next.grant.send(turn) {
                    refused.push(turn);
                }
            }
            state.running
        };
        drop(refused);
        // Notify when all downloads have completed
        if running_after == 0 {
            self.all_done.notify_waiters();
        }
    }

    fn is_closed(&self) -> bool {
        self.lock().closed
    }

    /// Closes the queue and waits for all running downloads to complete.
    async fn close_and_wait(&self) {
        // Create the notified future first, before checking state
        let notified = self.all_done.notified();
        {
            let mut state = self.lock();
            state.closed = true;
            // If there are no running downloads, notify immediately
            if state.running == 0 {
                self.all_done.notify_waiters();
                return;
            }
        }
        // Wait for all running downloads to complete
        notified.await;
    }
}

struct SharedCoverFetchError(Arc<Error>);

impl fmt::Display for SharedCoverFetchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "shared cover fetch failed: {}", self.0)
    }
}

impl fmt::Debug for SharedCoverFetchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("SharedCoverFetchError")
            .field(&self.0)
            .finish()
    }
}

impl error::Error for SharedCoverFetchError {}

/// Takes a cover's download out of the cache's pending ones when dropped.
struct PendingGuard {
    cache: Arc<ServiceCoverCache>,
    cover_id: String,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.cache
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.cover_id);
    }
}

impl ServiceCoverCache {
    pub(crate) fn new(root: PathBuf, broadcaster: Arc<dyn Broadcaster>) -> Self {
        Self {
            root,
            pending: Mutex::new(HashMap::new()),
            downloads: DownloadQueue::new(),
            cancellation: CancellationToken::new(),
            broadcaster,
            change_pending: Arc::new(AtomicBool::new(false)),
            playing: Mutex::new(None),
        }
    }

    /// Closes the download queue and waits for all active downloads to complete.
    pub(crate) async fn close_and_wait_downloads(&self) {
        self.cancellation.cancel();
        self.downloads.close_and_wait().await;
    }

    /// Announces that covers are ready. A burst of finished covers gives one announcement, sent
    /// when the window that the first one opened ends, so a page showing covers does not reload
    /// once per cover.
    fn announce_change(&self) {
        if self.change_pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let pending = Arc::clone(&self.change_pending);
        let broadcaster = Arc::clone(&self.broadcaster);
        let cancellation = self.cancellation.clone();
        tokio::spawn(async move {
            tokio_time::sleep(CHANGE_WINDOW).await;
            pending.store(false, Ordering::SeqCst);
            if !cancellation.is_cancelled() {
                broadcaster.broadcast(&LibraryContentChanged {
                    path: String::new(),
                });
            }
        });
    }

    pub(crate) fn path_for(&self, cover_id: &str) -> PathBuf {
        let key = blake3::hash(cover_id.as_bytes()).to_hex();
        self.root.join(format!("{key}.img"))
    }

    pub(crate) async fn cached_path(&self, cover_id: &str) -> Result<Option<String>> {
        let path = self.path_for(cover_id);
        task::spawn_blocking(move || -> Result<Option<String>> {
            let Ok(metadata) = fs::metadata(&path) else {
                return Ok(None);
            };
            if !metadata.is_file() || metadata.len() == 0 {
                return Ok(None);
            }
            if let Ok(file) = fs::OpenOptions::new().write(true).open(&path) {
                let _ = file.set_times(fs::FileTimes::new().set_modified(SystemTime::now()));
            }
            Ok(path.to_str().map(str::to_owned))
        })
        .await?
    }

    /// Starts downloading a cover that nobody waits for. It waits for its turn behind the
    /// covers a request waits for, and ahead of the scheduled ones requested before it.
    pub(crate) fn schedule<S: CoverSource>(self: &Arc<Self>, source: Arc<S>, cover_id: String) {
        let cache = Arc::clone(self);
        tokio::spawn(async move {
            let _ = cache.ensure(source, &cover_id, false).await;
        });
    }

    /// Starts downloading the cover of what plays now, which its status shows once the cover is
    /// cached and which nothing asks for again. It waits for the cover until the cover is cached,
    /// so like a cover a request waits for, it goes ahead of the scheduled covers and is never
    /// dropped from the queue, and the status does get it. `cover_id` is None when what plays
    /// now has no cover to wait for. The cover of what played before is no longer waited for, so
    /// covers of tracks skipped over go back among the scheduled ones, while any request that
    /// waits for one of them still gets it.
    pub(crate) fn schedule_playing<S: CoverSource>(
        self: &Arc<Self>,
        source: Arc<S>,
        cover_id: Option<String>,
    ) {
        let mut playing = self.playing.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((waited, request)) = &*playing
            && cover_id.as_ref() == Some(waited)
            && !request.is_finished()
        {
            return;
        }
        if let Some((_, request)) = playing.take() {
            request.abort();
        }
        let Some(cover_id) = cover_id else {
            return;
        };
        let cache = Arc::clone(self);
        let id = cover_id.clone();
        let request = tokio::spawn(async move {
            let _ = cache.ensure(source, &id, true).await;
        });
        *playing = Some((cover_id, request.abort_handle()));
    }

    /// The path of a cover once it is cached, downloaded now when it is not. The download goes
    /// ahead of the scheduled ones.
    pub(crate) async fn ensure_cached<S: CoverSource>(
        self: &Arc<Self>,
        source: Arc<S>,
        cover_id: &str,
    ) -> Result<Option<String>> {
        self.ensure(source, cover_id, true).await
    }

    /// Waits until every cover of `cover_ids` is in the cache, fetching them as `ensure_cached`
    /// does, and returns the ids of the covers that are now cached. A cover that cannot be
    /// fetched is left out rather than failing the others.
    pub(crate) async fn cache_covers<S: CoverSource>(
        self: &Arc<Self>,
        source: &Arc<S>,
        cover_ids: HashSet<String>,
    ) -> HashSet<String> {
        stream::iter(cover_ids)
            .map(|cover_id| async move {
                match self.ensure_cached(Arc::clone(source), &cover_id).await {
                    Ok(Some(_)) => Some(cover_id),
                    Ok(None) => {
                        log::debug!("Cover {cover_id} is not available");
                        None
                    }
                    Err(error) => {
                        log::debug!("Cover {cover_id} could not be cached: {error:#}");
                        None
                    }
                }
            })
            // The cache downloads only so many covers at a time, so waiting on more only
            // queues them.
            .buffer_unordered(COVER_DOWNLOADS)
            .filter_map(future::ready)
            .collect()
            .await
    }

    async fn ensure<S: CoverSource>(
        self: &Arc<Self>,
        source: Arc<S>,
        cover_id: &str,
        awaited: bool,
    ) -> Result<Option<String>> {
        loop {
            if let Some(path) = self.cached_path(cover_id).await? {
                return Ok(Some(path));
            }
            // A request that waits stays counted among the download's waiters while `_waiter`
            // lives.
            let (mut receiver, _waiter) = {
                let mut pending = self
                    .pending
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Service cover cache lock poisoned"))?;
                if let Some(joined) = pending.get(cover_id) {
                    let waiter = self.downloads.rejoin(cover_id, &joined.waiters, awaited);
                    (joined.receiver.clone(), waiter)
                } else {
                    let (sender, receiver) = watch::channel(None);
                    let waiters = Arc::new(AtomicUsize::new(0));
                    let waiter = awaited.then(|| Waiter::join(&waiters, &self.downloads).0);
                    pending.insert(
                        cover_id.to_owned(),
                        Pending {
                            receiver: receiver.clone(),
                            waiters: Arc::clone(&waiters),
                        },
                    );
                    let cache = Arc::clone(self);
                    let fetch_source = Arc::clone(&source);
                    let id = cover_id.to_owned();
                    tokio::spawn(async move {
                        // Removed also when the download panics, so a later request starts
                        // afresh.
                        let pending = PendingGuard {
                            cache: Arc::clone(&cache),
                            cover_id: id.clone(),
                        };
                        let result = cache.fetch_uncached(fetch_source, &id, &waiters).await;
                        drop(pending);
                        let _ = sender.send(Some(match result {
                            Ok(Finished::Done(path)) => Fetched::Done(Ok(path)),
                            Ok(Finished::Dropped) => Fetched::Dropped,
                            Err(error) => Fetched::Done(Err(Arc::new(error))),
                        }));
                    });
                    (receiver, waiter)
                }
            };
            let fetched = loop {
                if let Some(fetched) = receiver.borrow().clone() {
                    break fetched;
                }
                receiver.changed().await.map_err(|_| {
                    anyhow::anyhow!("Service cover fetch task ended without a result")
                })?;
            };
            match fetched {
                Fetched::Done(result) => {
                    return result.map_err(|error| Error::new(SharedCoverFetchError(error)));
                }
                // A request that waits does not take a download dropped from the queue for an
                // answer: it asks again, and the download starts afresh.
                Fetched::Dropped if awaited => {}
                Fetched::Dropped => return Ok(None),
            }
        }
    }

    /// Downloads a cover once it has a turn. Answers `Dropped` when it was dropped from the
    /// queue before that, which clears its place for a later request.
    async fn fetch_uncached<S: CoverSource>(
        &self,
        source: Arc<S>,
        cover_id: &str,
        waiters: &Arc<AtomicUsize>,
    ) -> Result<Finished> {
        // A request that missed the cache just before an earlier fetch for this cover stored
        // its file, and found no fetch in flight just after that fetch finished, starts this
        // one: the file is there now, so it must not be fetched twice.
        if let Some(path) = self.cached_path(cover_id).await? {
            return Ok(Finished::Done(Some(path)));
        }
        let slot = match self.downloads.enter(cover_id, waiters) {
            Turn::Ready(slot) => slot,
            Turn::Closed => return Ok(Finished::Done(None)),
            Turn::Waiting(mut turn) => {
                tokio::select! {
                    _ = self.cancellation.cancelled() => {
                        self.downloads.withdraw(cover_id);
                        return Ok(Finished::Done(None));
                    }
                    granted = &mut turn => match granted {
                        Ok(slot) => slot,
                        Err(_) if self.downloads.is_closed() => return Ok(Finished::Done(None)),
                        Err(_) => return Ok(Finished::Dropped),
                    },
                }
            }
        };
        let bytes = tokio::select! {
            _ = self.cancellation.cancelled() => return Ok(Finished::Done(None)),
            result = source.cover_bytes(cover_id) => result?,
        };
        if bytes.is_empty() || bytes.len() > MAX_COVER_BYTES {
            return Ok(Finished::Done(None));
        }
        let path = self.path_for(cover_id);
        tokio_fs::create_dir_all(&self.root).await?;
        // Each download writes a file of its own, as another cache of the same library may
        // fetch the same cover at once.
        let temporary = path.with_extension(format!("{}.tmp", Uuid::new_v4().simple()));
        let written = async {
            tokio_fs::write(&temporary, bytes).await?;
            tokio_fs::rename(&temporary, &path).await
        };
        if let Err(error) = written.await {
            let _ = tokio_fs::remove_file(&temporary).await;
            return Err(error.into());
        }
        let root = self.root.clone();
        let newest = path.clone();
        task::spawn_blocking(move || evict_cover_cache(&root, &newest, COVER_CACHE_LIMITS))
            .await??;
        self.announce_change();
        drop(slot);
        Ok(Finished::Done(self.cached_path(cover_id).await?))
    }
}

/// Where the primary color of the cached cover at `cover` is kept: beside it, under the same
/// name with the extension `color`.
fn color_path(cover: &Path) -> PathBuf {
    cover.with_extension("color")
}

/// The primary color of the cached cover at `cover`, as `metadata::cover_art::get_primary_color`
/// gives it for the cover's bytes. It is kept beside the cover with the digest of the bytes it
/// was worked out from, and read from there while the cover holds those bytes: a cover evicted
/// and fetched again may hold other artwork by the time a color worked out before is written.
/// None when the cover cannot be read or has no color. Reading and decoding run off the async
/// runtime.
pub(crate) async fn cover_primary_color(cover: &str) -> Option<i32> {
    let cover = PathBuf::from(cover);
    task::spawn_blocking(move || {
        let bytes = fs::read(&cover).ok()?;
        let digest = blake3::hash(&bytes).to_hex();
        let color_file = color_path(&cover);
        if let Ok(text) = fs::read_to_string(&color_file)
            && let Some((kept_for, color)) = text.trim().split_once(' ')
            && kept_for == digest.as_str()
            && let Ok(color) = color.parse::<i32>()
        {
            return Some(color);
        }
        let color = metadata::cover_art::get_primary_color(&bytes)?;
        keep_primary_color(&cover, digest.as_str(), color);
        Some(color)
    })
    .await
    .ok()
    .flatten()
}

/// Keeps `color`, worked out from the bytes with `digest`, beside the cached cover at `cover`.
///
/// Eviction may remove the cover meanwhile, and it counts only covers, so a color left without
/// its cover would stay for good. Eviction removes a cover before its color, and the color is
/// put in place before the cover is looked for: either eviction finds the color and removes it,
/// or the cover is found gone and the color is removed here.
fn keep_primary_color(cover: &Path, digest: &str, color: i32) {
    let color_file = color_path(cover);
    // A file of its own, as another request may work out the same color at once.
    let temporary = color_file.with_extension(format!("{}.color.tmp", Uuid::new_v4().simple()));
    if fs::write(&temporary, format!("{digest} {color}"))
        .and_then(|()| fs::rename(&temporary, &color_file))
        .is_err()
    {
        let _ = fs::remove_file(&temporary);
        return;
    }
    if !cover.exists() {
        let _ = fs::remove_file(&color_file);
    }
}

/// Bounds of one service library's cover cache. A cover wall or album list shows hundreds of
/// covers, so the cache must hold several pages of them.
#[derive(Clone, Copy)]
struct CoverCacheLimits {
    max_files: usize,
    max_bytes: u64,
}

const COVER_CACHE_LIMITS: CoverCacheLimits = CoverCacheLimits {
    max_files: 4096,
    max_bytes: 256 * 1024 * 1024,
};

fn evict_cover_cache(root: &Path, newest: &Path, limits: CoverCacheLimits) -> io::Result<()> {
    let mut entries = fs::read_dir(root)?
        .filter_map(result::Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            (metadata.is_file() && entry.path().extension().is_some_and(|ext| ext == "img")).then(
                || {
                    (
                        entry.path(),
                        metadata.len(),
                        metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    )
                },
            )
        })
        .collect::<Vec<_>>();
    let mut bytes = entries.iter().map(|(_, size, _)| *size).sum::<u64>();
    while entries.len() > limits.max_files || bytes > limits.max_bytes {
        let Some((oldest_index, _)) = entries
            .iter()
            .enumerate()
            .filter(|(_, (path, _, _))| path != newest)
            .min_by_key(|(_, (_, _, modified))| *modified)
        else {
            break;
        };
        let (path, size, _) = entries.swap_remove(oldest_index);
        fs::remove_file(&path)?;
        // The color kept beside the cover goes with it, so a cover fetched again is read afresh.
        let _ = fs::remove_file(color_path(&path));
        bytes = bytes.saturating_sub(size);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, OpenOptions},
        thread,
        time::{Duration, SystemTime},
    };

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use futures::future::BoxFuture;
    use tempfile::TempDir;
    use tokio::sync::Semaphore;
    use tokio::time as tokio_time;

    use super::{
        CoverCacheLimits, CoverSource, MAX_WAITING_SCHEDULED, ServiceCoverCache, Turn,
        evict_cover_cache, keep_primary_color,
    };
    use crate::test_wait::wait_until;
    use crate::utils::{Broadcaster, RinfRustSignal};

    /// A source that notes which covers it was asked for, in order, and finishes one only when
    /// the test lets it.
    struct Gated {
        started: Mutex<Vec<String>>,
        gate: Semaphore,
    }

    impl Gated {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                started: Mutex::new(Vec::new()),
                gate: Semaphore::new(0),
            })
        }

        fn started(&self) -> Vec<String> {
            self.started.lock().unwrap().clone()
        }

        /// Lets `count` downloads finish.
        fn finish(&self, count: usize) {
            self.gate.add_permits(count);
        }
    }

    impl CoverSource for Gated {
        fn cover_bytes<'a>(&'a self, cover_id: &'a str) -> BoxFuture<'a, anyhow::Result<Vec<u8>>> {
            Box::pin(async move {
                self.started.lock().unwrap().push(cover_id.to_owned());
                self.gate.acquire().await?.forget();
                Ok(vec![1])
            })
        }
    }

    struct Quiet;

    impl Broadcaster for Quiet {
        fn broadcast(&self, _: &dyn RinfRustSignal) {}
    }

    struct Counting(AtomicUsize);

    impl Broadcaster for Counting {
        fn broadcast(&self, _: &dyn RinfRustSignal) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Waits until the cache's tasks make `condition` true.
    async fn until(what: &str, mut condition: impl FnMut() -> bool) {
        wait_until(what, || {
            let holds = condition();
            async move { Ok(holds.then_some(())) }
        })
        .await
        .unwrap();
    }

    /// Whether `cover_id` waits in the download queue, and whether a request waits for it.
    fn in_line(cache: &ServiceCoverCache, cover_id: &str) -> Option<bool> {
        cache
            .downloads
            .lock()
            .waiting
            .iter()
            .find(|waiting| waiting.cover_id == cover_id)
            .map(|waiting| waiting.awaited())
    }

    /// How many times a download took a place in the queue's line.
    fn places_taken(cache: &ServiceCoverCache) -> u64 {
        cache.downloads.lock().next_order
    }

    /// How many scheduled covers, which no request waits for, wait in the download queue.
    fn scheduled_in_line(cache: &ServiceCoverCache) -> usize {
        cache
            .downloads
            .lock()
            .waiting
            .iter()
            .filter(|waiting| !waiting.awaited())
            .count()
    }

    fn pending(cache: &ServiceCoverCache) -> usize {
        cache.pending.lock().unwrap().len()
    }

    fn cache_in(root: &TempDir) -> Arc<ServiceCoverCache> {
        Arc::new(ServiceCoverCache::new(
            root.path().join("covers"),
            Arc::new(Quiet),
        ))
    }

    /// Fills the four turns with covers that stay downloading until the source is let go.
    async fn fill_turns(cache: &Arc<ServiceCoverCache>, source: &Arc<Gated>) {
        for index in 0..4 {
            cache.schedule(Arc::clone(source), format!("running-{index}"));
        }
        until("the four turns to be taken", || source.started().len() == 4).await;
    }

    /// Schedules `cover_id` and waits until it is in line.
    async fn schedule_in_line(cache: &Arc<ServiceCoverCache>, source: &Arc<Gated>, cover_id: &str) {
        cache.schedule(Arc::clone(source), cover_id.to_owned());
        until(cover_id, || in_line(cache, cover_id) == Some(false)).await;
    }

    #[tokio::test]
    async fn waiting_downloads_start_newest_first() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        let source = Gated::new();
        fill_turns(&cache, &source).await;
        for index in 0..5 {
            schedule_in_line(&cache, &source, &format!("page-{index}")).await;
        }
        for (done, expected) in ["page-4", "page-3", "page-2", "page-1", "page-0"]
            .into_iter()
            .enumerate()
        {
            source.finish(1);
            until(expected, || source.started().len() == 5 + done).await;
            assert_eq!(source.started().last().map(String::as_str), Some(expected));
        }
    }

    #[tokio::test]
    async fn covers_that_requests_wait_for_start_in_the_order_they_were_asked_for() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        let source = Gated::new();
        fill_turns(&cache, &source).await;
        let ask = |cover_id: &'static str| {
            let (cache, source) = (cache.clone(), source.clone());
            tokio::spawn(async move { cache.ensure_cached(source, cover_id).await })
        };
        // The playing cover waits first, and requests for other covers keep coming after it, as
        // from a busy cover wall: none of them may keep it waiting.
        cache.schedule_playing(Arc::clone(&source), Some("playing".to_owned()));
        until("playing", || in_line(&cache, "playing") == Some(true)).await;
        schedule_in_line(&cache, &source, "scheduled").await;
        let asked = ask("asked");
        until("asked", || in_line(&cache, "asked") == Some(true)).await;
        let later = ask("later");
        until("later", || in_line(&cache, "later") == Some(true)).await;
        for (done, expected) in ["playing", "asked", "later", "scheduled"]
            .into_iter()
            .enumerate()
        {
            source.finish(1);
            until(expected, || source.started().len() == 5 + done).await;
            assert_eq!(source.started().last().map(String::as_str), Some(expected));
        }
        source.finish(100);
        assert!(asked.await.unwrap().unwrap().is_some());
        assert!(later.await.unwrap().unwrap().is_some());
    }

    #[tokio::test]
    async fn an_awaited_cover_goes_ahead_of_scheduled_ones() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        let source = Gated::new();
        fill_turns(&cache, &source).await;
        schedule_in_line(&cache, &source, "scheduled-old").await;
        let playing = tokio::spawn({
            let (cache, source) = (cache.clone(), source.clone());
            async move { cache.ensure_cached(source, "playing").await }
        });
        until("playing", || in_line(&cache, "playing") == Some(true)).await;
        schedule_in_line(&cache, &source, "scheduled-new").await;
        source.finish(1);
        until("the fifth download", || source.started().len() == 5).await;
        assert_eq!(source.started()[4], "playing");
        // A scheduled cover that a request then waits for is awaited from then on.
        let late = tokio::spawn({
            let (cache, source) = (cache.clone(), source.clone());
            async move { cache.ensure_cached(source, "scheduled-old").await }
        });
        until("scheduled-old awaited", || {
            in_line(&cache, "scheduled-old") == Some(true)
        })
        .await;
        source.finish(1);
        until("the sixth download", || source.started().len() == 6).await;
        assert_eq!(source.started()[5], "scheduled-old");
        source.finish(100);
        assert!(playing.await.unwrap().unwrap().is_some());
        assert!(late.await.unwrap().unwrap().is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cover_a_request_starts_waiting_for_is_never_seen_awaited_at_its_old_place() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        let source = Gated::new();
        fill_turns(&cache, &source).await;
        schedule_in_line(&cache, &source, "scheduled").await;
        let ask = |cover_id: &'static str| {
            let (cache, source) = (cache.clone(), source.clone());
            tokio::spawn(async move { cache.ensure_cached(source, cover_id).await })
        };
        let asked = ask("asked");
        until("asked", || in_line(&cache, "asked") == Some(true)).await;
        // A request starts waiting for the scheduled cover just as a turn ends. Awaited at the
        // place it was scheduled at, it would go ahead of the cover asked for before it.
        let late = {
            let mut state = cache.downloads.lock();
            let late = ask("scheduled");
            // The request gets as far as it can while the turn holds the queue.
            thread::sleep(Duration::from_millis(200));
            let next = state.take_next().unwrap();
            assert_eq!(next.cover_id, "asked");
            state.waiting.push(next);
            late
        };
        source.finish(100);
        assert!(asked.await.unwrap().unwrap().is_some());
        assert!(late.await.unwrap().unwrap().is_some());
    }

    #[tokio::test]
    async fn the_oldest_scheduled_downloads_are_dropped_beyond_the_bound_and_start_afresh_later() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        let source = Gated::new();
        fill_turns(&cache, &source).await;
        let pages = MAX_WAITING_SCHEDULED + 3;
        for index in 0..pages {
            cache.schedule(Arc::clone(&source), format!("page-{index}"));
        }
        // Every page took a place in line, so every page was pending, and the three that came
        // first gave their places up and are pending no more. Which they are depends on the
        // order the downloads reached the queue in, so only their number is known.
        until("every page in line and three dropped", || {
            places_taken(&cache) == pages as u64 && pending(&cache) == 4 + MAX_WAITING_SCHEDULED
        })
        .await;
        let dropped: Vec<String> = (0..pages)
            .map(|index| format!("page-{index}"))
            .filter(|id| !cache.pending.lock().unwrap().contains_key(id))
            .collect();
        assert_eq!(dropped.len(), 3);
        assert!(dropped.iter().all(|id| !source.started().contains(id)));
        // Asked for again, a dropped one is downloaded afresh.
        let again = tokio::spawn({
            let (cache, source, id) = (cache.clone(), source.clone(), dropped[0].clone());
            async move { cache.ensure_cached(source, &id).await }
        });
        source.finish(10_000);
        assert!(again.await.unwrap().unwrap().is_some());
        let started = source.started();
        assert_eq!(started.iter().filter(|id| **id == dropped[0]).count(), 1);
    }

    #[tokio::test]
    async fn a_cover_a_request_waits_for_is_not_dropped_beyond_the_bound() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        let source = Gated::new();
        fill_turns(&cache, &source).await;
        schedule_in_line(&cache, &source, "first").await;
        let asked = tokio::spawn({
            let (cache, source) = (cache.clone(), source.clone());
            async move { cache.ensure_cached(source, "first").await }
        });
        until("first awaited", || in_line(&cache, "first") == Some(true)).await;
        let taken = places_taken(&cache);
        // One more scheduled cover than the bound allows: the oldest scheduled one gives its
        // place up, not the cover that was scheduled first but is waited for now.
        let pages = MAX_WAITING_SCHEDULED + 1;
        for index in 0..pages {
            cache.schedule(Arc::clone(&source), format!("page-{index}"));
        }
        until("every page in line and one dropped", || {
            places_taken(&cache) == taken + pages as u64
                && pending(&cache) == 4 + 1 + MAX_WAITING_SCHEDULED
        })
        .await;
        assert_eq!(in_line(&cache, "first"), Some(true));
        source.finish(10_000);
        assert!(asked.await.unwrap().unwrap().is_some());
    }

    #[tokio::test]
    async fn a_cover_whose_request_stopped_waiting_waits_as_a_scheduled_one() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        let source = Gated::new();
        fill_turns(&cache, &source).await;
        let asked = tokio::spawn({
            let (cache, source) = (cache.clone(), source.clone());
            async move { cache.ensure_cached(source, "asked").await }
        });
        until("asked awaited", || in_line(&cache, "asked") == Some(true)).await;
        // The request gives up, as a color request does once its wait is over. The download
        // keeps its place, but nothing waits for it any more.
        asked.abort();
        assert!(asked.await.unwrap_err().is_cancelled());
        until("asked scheduled", || {
            in_line(&cache, "asked") == Some(false)
        })
        .await;
        // So it counts within the bound of the scheduled covers, as the oldest of them.
        let taken = places_taken(&cache);
        for index in 0..MAX_WAITING_SCHEDULED {
            cache.schedule(Arc::clone(&source), format!("page-{index}"));
        }
        until("every page in line and the bound kept", || {
            places_taken(&cache) == taken + MAX_WAITING_SCHEDULED as u64
                && scheduled_in_line(&cache) == MAX_WAITING_SCHEDULED
        })
        .await;
        assert_eq!(in_line(&cache, "asked"), None);
        source.finish(10_000);
    }

    #[tokio::test]
    async fn the_playing_cover_is_not_dropped_beyond_the_bound() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        let source = Gated::new();
        fill_turns(&cache, &source).await;
        cache.schedule_playing(Arc::clone(&source), Some("playing".to_owned()));
        until("playing", || in_line(&cache, "playing").is_some()).await;
        let taken = places_taken(&cache);
        // More scheduled covers than the bound allows arrive after it. Nothing asks for the
        // playing cover again, so it must keep its place.
        let pages = MAX_WAITING_SCHEDULED + 1;
        for index in 0..pages {
            cache.schedule(Arc::clone(&source), format!("page-{index}"));
        }
        until("every page in line and the bound kept", || {
            places_taken(&cache) == taken + pages as u64
                && scheduled_in_line(&cache) == MAX_WAITING_SCHEDULED
        })
        .await;
        assert!(in_line(&cache, "playing").is_some());
        source.finish(1);
        until("the fifth download", || source.started().len() == 5).await;
        assert_eq!(source.started()[4], "playing");
        source.finish(10_000);
    }

    #[tokio::test]
    async fn the_cover_of_what_played_before_waits_as_a_scheduled_one() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        let source = Gated::new();
        fill_turns(&cache, &source).await;
        let plays = |cover_id: Option<&str>| {
            cache.schedule_playing(Arc::clone(&source), cover_id.map(str::to_owned));
        };
        plays(Some("first"));
        until("first", || in_line(&cache, "first") == Some(true)).await;
        // The next track starts before the first one's cover came. Only the next cover is waited
        // for now, so a skipped cover cannot keep it waiting.
        plays(Some("next"));
        until("first scheduled", || {
            in_line(&cache, "first") == Some(false)
        })
        .await;
        until("next", || in_line(&cache, "next") == Some(true)).await;
        // The same track again keeps its cover's place.
        let taken = places_taken(&cache);
        plays(Some("next"));
        assert_eq!(in_line(&cache, "next"), Some(true));
        assert_eq!(places_taken(&cache), taken);
        // A track whose cover is cached leaves no cover to wait for.
        plays(None);
        until("next scheduled", || in_line(&cache, "next") == Some(false)).await;
        source.finish(100);
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_of_finished_covers_gives_one_announcement() {
        let root = TempDir::new().unwrap();
        let counting = Arc::new(Counting(AtomicUsize::new(0)));
        let cache = ServiceCoverCache::new(root.path().join("covers"), counting.clone());
        let count = || counting.0.load(Ordering::SeqCst);
        for _ in 0..50 {
            cache.announce_change();
            tokio_time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(count(), 0, "nothing is sent while the window is open");
        tokio_time::sleep(Duration::from_secs(1)).await;
        assert_eq!(count(), 1);
        // A later cover opens a new window.
        cache.announce_change();
        tokio_time::sleep(Duration::from_secs(1)).await;
        assert_eq!(count(), 2);
    }

    /// A source that gives every cover at once.
    struct Ready;

    impl CoverSource for Ready {
        fn cover_bytes<'a>(&'a self, _: &'a str) -> BoxFuture<'a, anyhow::Result<Vec<u8>>> {
            Box::pin(async { Ok(vec![1; 4096]) })
        }
    }

    /// A library without a binding held for it gets a binding, and so a cache, for each use, and
    /// those caches share its folder.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_caches_of_one_library_fetch_the_same_cover_at_once() {
        let root = TempDir::new().unwrap();
        let (first, second) = (cache_in(&root), cache_in(&root));
        let source = Arc::new(Ready);
        for index in 0..64 {
            let cover_id = format!("cover-{index}");
            let (from_first, from_second) = tokio::join!(
                first.ensure_cached(Arc::clone(&source), &cover_id),
                second.ensure_cached(Arc::clone(&source), &cover_id),
            );
            for path in [from_first.unwrap(), from_second.unwrap()] {
                assert_eq!(fs::read(path.unwrap()).unwrap(), vec![1; 4096]);
            }
        }
    }

    #[tokio::test]
    async fn closing_the_downloads_waits_for_a_cover_being_written_and_ends_them_all() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        // A download holds its turn until its file is written.
        let Turn::Ready(writing) = cache
            .downloads
            .enter("writing", &Arc::new(AtomicUsize::new(1)))
        else {
            panic!("a free turn was not given");
        };
        // Downloads waiting behind it are let go by the close, not left to wait for ever.
        let source = Gated::new();
        let waiting = cache.clone();
        let asked = tokio::spawn({
            let source = source.clone();
            async move {
                for index in 0..4 {
                    waiting.schedule(source.clone(), format!("held-{index}"));
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
                waiting.ensure_cached(source, "waiting").await
            }
        });
        tokio::time::sleep(Duration::from_millis(150)).await;
        let closed = tokio::spawn({
            let cache = cache.clone();
            async move { cache.close_and_wait_downloads().await }
        });
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            !closed.is_finished(),
            "the close did not wait for the write"
        );
        drop(writing);
        closed.await.unwrap();
        assert_eq!(asked.await.unwrap().unwrap(), None);
        // Nobody gets a turn again, and no download reaches the source.
        assert!(matches!(
            cache
                .downloads
                .enter("late", &Arc::new(AtomicUsize::new(1))),
            Turn::Closed
        ));
        assert_eq!(
            cache.ensure_cached(source.clone(), "late").await.unwrap(),
            None
        );
        assert!(!source.started().contains(&"late".to_owned()));
    }

    #[test]
    fn cover_cache_evicts_oldest_files_to_its_entry_limit() {
        let root = TempDir::new().unwrap();
        let newest = root.path().join("cover-128.img");
        for index in 0..129 {
            let path = root.path().join(format!("cover-{index}.img"));
            fs::write(&path, [index as u8]).unwrap();
            let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(index + 1);
            OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(modified))
                .unwrap();
        }
        evict_cover_cache(
            root.path(),
            &newest,
            CoverCacheLimits {
                max_files: 128,
                max_bytes: u64::MAX,
            },
        )
        .unwrap();
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 128);
        assert!(!root.path().join("cover-0.img").exists());
        assert!(newest.exists());
    }

    #[test]
    fn evicting_a_cover_removes_the_color_kept_beside_it() {
        let root = TempDir::new().unwrap();
        let old = root.path().join("old.img");
        let newest = root.path().join("new.img");
        for path in [&old, &newest] {
            fs::write(path, b"cover").unwrap();
            fs::write(path.with_extension("color"), b"7").unwrap();
        }
        OpenOptions::new()
            .write(true)
            .open(&old)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();
        evict_cover_cache(
            root.path(),
            &newest,
            CoverCacheLimits {
                max_files: 1,
                max_bytes: u64::MAX,
            },
        )
        .unwrap();
        assert!(!old.exists());
        assert!(!old.with_extension("color").exists());
        assert!(newest.exists());
        assert!(newest.with_extension("color").exists());
    }

    #[test]
    fn a_color_worked_out_for_a_cover_evicted_meanwhile_is_not_kept() {
        let root = TempDir::new().unwrap();
        // Eviction removed the cover after its bytes were read for the color, and found no
        // color beside it to remove: the color must not stay behind on its own, where no
        // eviction counts it.
        let gone = root.path().join("gone.img");
        keep_primary_color(&gone, "digest", 7);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        // Beside a cover still cached, the color is kept.
        let cached = root.path().join("cached.img");
        fs::write(&cached, b"cover").unwrap();
        keep_primary_color(&cached, "digest", 7);
        assert_eq!(
            fs::read_to_string(cached.with_extension("color")).unwrap(),
            "digest 7"
        );
    }
}
