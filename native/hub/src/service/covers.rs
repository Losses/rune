//! The covers of a streaming service's library, downloaded by the ids the service gives them and
//! kept on disk.

use std::collections::{HashMap, HashSet};
use std::error;
use std::fmt;
use std::fs;
use std::future;
use std::io;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use anyhow::{Error, Result};
use futures::future::BoxFuture;
use futures::{StreamExt, stream};
use tokio::fs as tokio_fs;
use tokio::sync::{oneshot, watch};
use tokio::task;
use tokio::time as tokio_time;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{messages::LibraryContentChanged, utils::Broadcaster};

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
const MAX_COVER_BYTES: usize = 8 * 1024 * 1024;

/// Where the cover cache of one service library lives: its own folder, named by its library
/// id. The parent keeps the name it had when only Subsonic libraries cached covers, so the
/// caches already on disk stay where they are.
pub(crate) fn cover_cache_root(config_dir: &Path, library_id: Uuid) -> PathBuf {
    config_dir
        .join("subsonic-covers")
        .join(library_id.to_string())
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
}

/// A download in flight or waiting for its turn, which requests for the same cover join.
struct Pending {
    receiver: watch::Receiver<Option<Fetched>>,
    /// Whether a request that waits for the cover joined it, which puts it ahead of the
    /// downloads that nobody waits for.
    awaited: Arc<AtomicBool>,
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
    awaited: Arc<AtomicBool>,
    grant: oneshot::Sender<Slot>,
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
    /// not, and the newest of those first, as it is what the user looks at.
    fn take_next(&mut self) -> Option<Waiting> {
        let index = (0..self.waiting.len()).max_by_key(|&index| {
            let waiting = &self.waiting[index];
            (waiting.awaited.load(Ordering::SeqCst), waiting.order)
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
                .filter(|(_, waiting)| !waiting.awaited.load(Ordering::SeqCst));
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

impl DownloadQueue {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(QueueState::default()),
        })
    }

    fn lock(&self) -> MutexGuard<'_, QueueState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Asks for a turn: at once when one is free, else by waiting in line.
    fn enter(self: &Arc<Self>, cover_id: &str, awaited: &Arc<AtomicBool>) -> Turn {
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
            awaited: Arc::clone(awaited),
            grant,
        });
        state.drop_oldest_scheduled();
        Turn::Waiting(turn)
    }

    /// A request for a cover that waits in line again: it is the newest, and it is awaited when
    /// the request is.
    fn bump(&self, cover_id: &str, awaited: bool) {
        let mut state = self.lock();
        let order = state.next_order;
        if let Some(waiting) = state
            .waiting
            .iter_mut()
            .find(|waiting| waiting.cover_id == cover_id)
        {
            waiting.order = order;
            if awaited {
                waiting.awaited.store(true, Ordering::SeqCst);
            }
            state.next_order += 1;
        }
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
        {
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
        }
        drop(refused);
    }

    fn is_closed(&self) -> bool {
        self.lock().closed
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

pub(crate) async fn delete_service_cover_cache(
    config_dir: &Path,
    library_id: Uuid,
) -> io::Result<()> {
    match tokio_fs::remove_dir_all(cover_cache_root(config_dir, library_id)).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

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
        }
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
            let (mut receiver, joined) = {
                let mut pending = self
                    .pending
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Service cover cache lock poisoned"))?;
                if let Some(joined) = pending.get(cover_id) {
                    if awaited {
                        joined.awaited.store(true, Ordering::SeqCst);
                    }
                    (joined.receiver.clone(), true)
                } else {
                    let (sender, receiver) = watch::channel(None);
                    let flag = Arc::new(AtomicBool::new(awaited));
                    pending.insert(
                        cover_id.to_owned(),
                        Pending {
                            receiver: receiver.clone(),
                            awaited: Arc::clone(&flag),
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
                        let result = cache.fetch_uncached(fetch_source, &id, &flag).await;
                        drop(pending);
                        let _ = sender.send(Some(match result {
                            Ok(Finished::Done(path)) => Fetched::Done(Ok(path)),
                            Ok(Finished::Dropped) => Fetched::Dropped,
                            Err(error) => Fetched::Done(Err(Arc::new(error))),
                        }));
                    });
                    (receiver, false)
                }
            };
            if joined {
                self.downloads.bump(cover_id, awaited);
            }
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
        awaited: &Arc<AtomicBool>,
    ) -> Result<Finished> {
        // A request that missed the cache just before an earlier fetch for this cover stored
        // its file, and found no fetch in flight just after that fetch finished, starts this
        // one: the file is there now, so it must not be fetched twice.
        if let Some(path) = self.cached_path(cover_id).await? {
            return Ok(Finished::Done(Some(path)));
        }
        let slot = match self.downloads.enter(cover_id, awaited) {
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
        let temporary = path.with_extension("tmp");
        tokio_fs::write(&temporary, bytes).await?;
        tokio_fs::rename(&temporary, &path).await?;
        let root = self.root.clone();
        let newest = path.clone();
        task::spawn_blocking(move || evict_cover_cache(&root, &newest, COVER_CACHE_LIMITS))
            .await??;
        self.announce_change();
        drop(slot);
        Ok(Finished::Done(self.cached_path(cover_id).await?))
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
        fs::remove_file(path)?;
        bytes = bytes.saturating_sub(size);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, OpenOptions},
        time::{Duration, SystemTime},
    };

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use futures::future::BoxFuture;
    use tempfile::TempDir;
    use tokio::sync::Semaphore;
    use tokio::time as tokio_time;
    use uuid::Uuid;

    use super::{
        CoverCacheLimits, CoverSource, MAX_WAITING_SCHEDULED, ServiceCoverCache, cover_cache_root,
        delete_service_cover_cache, evict_cover_cache,
    };
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

    /// Waits for the condition, which the cache's tasks make true in time.
    async fn until(mut condition: impl FnMut() -> bool) {
        for _ in 0..500 {
            if condition() {
                return;
            }
            tokio_time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the condition never held");
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
        until(|| source.started().len() == 4).await;
    }

    #[tokio::test]
    async fn waiting_downloads_start_newest_first() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        let source = Gated::new();
        fill_turns(&cache, &source).await;
        for index in 0..5 {
            cache.schedule(Arc::clone(&source), format!("page-{index}"));
            // Each is in line before the next is requested.
            tokio_time::sleep(Duration::from_millis(20)).await;
        }
        for (done, expected) in ["page-4", "page-3", "page-2", "page-1", "page-0"]
            .into_iter()
            .enumerate()
        {
            source.finish(1);
            until(|| source.started().len() == 5 + done).await;
            assert_eq!(source.started().last().map(String::as_str), Some(expected));
        }
    }

    #[tokio::test]
    async fn an_awaited_cover_goes_ahead_of_scheduled_ones() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        let source = Gated::new();
        fill_turns(&cache, &source).await;
        cache.schedule(Arc::clone(&source), "scheduled-old".to_owned());
        tokio_time::sleep(Duration::from_millis(20)).await;
        let playing = tokio::spawn({
            let (cache, source) = (cache.clone(), source.clone());
            async move { cache.ensure_cached(source, "playing").await }
        });
        tokio_time::sleep(Duration::from_millis(20)).await;
        cache.schedule(Arc::clone(&source), "scheduled-new".to_owned());
        tokio_time::sleep(Duration::from_millis(20)).await;
        source.finish(1);
        until(|| source.started().len() == 5).await;
        assert_eq!(source.started()[4], "playing");
        // A scheduled cover that a request then waits for is awaited from then on.
        let late = tokio::spawn({
            let (cache, source) = (cache.clone(), source.clone());
            async move { cache.ensure_cached(source, "scheduled-old").await }
        });
        tokio_time::sleep(Duration::from_millis(20)).await;
        source.finish(1);
        until(|| source.started().len() == 6).await;
        assert_eq!(source.started()[5], "scheduled-old");
        source.finish(100);
        assert!(playing.await.unwrap().unwrap().is_some());
        assert!(late.await.unwrap().unwrap().is_some());
    }

    #[tokio::test]
    async fn the_oldest_scheduled_downloads_are_dropped_beyond_the_bound_and_start_afresh_later() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        let source = Gated::new();
        fill_turns(&cache, &source).await;
        let pending = || cache.pending.lock().unwrap().len();
        for index in 0..MAX_WAITING_SCHEDULED + 3 {
            cache.schedule(Arc::clone(&source), format!("page-{index}"));
        }
        // Three give up their places and are no longer pending. Which they are depends on the
        // order the downloads reached the queue in, so only their number is known.
        until(|| pending() == 4 + MAX_WAITING_SCHEDULED).await;
        tokio_time::sleep(Duration::from_millis(100)).await;
        assert_eq!(pending(), 4 + MAX_WAITING_SCHEDULED);
        let dropped: Vec<String> = (0..MAX_WAITING_SCHEDULED + 3)
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
    async fn an_awaited_cover_whose_wait_was_dropped_is_asked_again() {
        let root = TempDir::new().unwrap();
        let cache = cache_in(&root);
        let source = Gated::new();
        fill_turns(&cache, &source).await;
        cache.schedule(Arc::clone(&source), "first".to_owned());
        tokio_time::sleep(Duration::from_millis(20)).await;
        for index in 0..MAX_WAITING_SCHEDULED {
            cache.schedule(Arc::clone(&source), format!("page-{index}"));
        }
        // "first" was the oldest, so it was dropped. Asking for it waits for a download anyway.
        until(|| !cache.pending.lock().unwrap().contains_key("first")).await;
        let asked = tokio::spawn({
            let (cache, source) = (cache.clone(), source.clone());
            async move { cache.ensure_cached(source, "first").await }
        });
        source.finish(10_000);
        assert!(asked.await.unwrap().unwrap().is_some());
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

    #[tokio::test]
    async fn removing_service_record_deletes_its_cover_cache_directory() {
        let root = TempDir::new().unwrap();
        let library_id = Uuid::new_v4();
        let cache = root
            .path()
            .join("subsonic-covers")
            .join(library_id.to_string());
        assert_eq!(cover_cache_root(root.path(), library_id), cache);
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("cover.img"), b"cover").unwrap();
        delete_service_cover_cache(root.path(), library_id)
            .await
            .unwrap();
        assert!(!cache.exists());
    }
}
