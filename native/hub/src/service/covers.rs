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
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::{Error, Result};
use futures::future::BoxFuture;
use futures::{StreamExt, stream};
use tokio::fs as tokio_fs;
use tokio::sync::{Semaphore, watch};
use tokio::task;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{messages::LibraryContentChanged, utils::Broadcaster};

/// Downloads cover images by the ids a service gives them.
pub(crate) trait CoverSource: Send + Sync + 'static {
    fn cover_bytes<'a>(&'a self, cover_id: &'a str) -> BoxFuture<'a, Result<Vec<u8>>>;
}

/// How many covers download at once.
const COVER_DOWNLOADS: usize = 4;

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
    pending: Mutex<HashMap<String, watch::Receiver<Option<CoverFetchResult>>>>,
    concurrency: Arc<Semaphore>,
    pub(crate) cancellation: CancellationToken,
    pub(crate) broadcaster: Arc<dyn Broadcaster>,
}

#[derive(Clone)]
struct CoverFetchResult(result::Result<Option<String>, Arc<Error>>);

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

impl ServiceCoverCache {
    pub(crate) fn new(root: PathBuf, broadcaster: Arc<dyn Broadcaster>) -> Self {
        Self {
            root,
            pending: Mutex::new(HashMap::new()),
            concurrency: Arc::new(Semaphore::new(COVER_DOWNLOADS)),
            cancellation: CancellationToken::new(),
            broadcaster,
        }
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

    pub(crate) fn schedule<S: CoverSource>(self: &Arc<Self>, source: Arc<S>, cover_id: String) {
        let cache = Arc::clone(self);
        tokio::spawn(async move {
            let _ = cache.ensure_cached(source, &cover_id).await;
        });
    }

    pub(crate) async fn ensure_cached<S: CoverSource>(
        self: &Arc<Self>,
        source: Arc<S>,
        cover_id: &str,
    ) -> Result<Option<String>> {
        if let Some(path) = self.cached_path(cover_id).await? {
            return Ok(Some(path));
        }
        let mut receiver = {
            let mut pending = self
                .pending
                .lock()
                .map_err(|_| anyhow::anyhow!("Service cover cache lock poisoned"))?;
            if let Some(receiver) = pending.get(cover_id) {
                receiver.clone()
            } else {
                let (sender, receiver) = watch::channel(None);
                pending.insert(cover_id.to_owned(), receiver.clone());
                let cache = Arc::clone(self);
                let fetch_source = Arc::clone(&source);
                let id = cover_id.to_owned();
                tokio::spawn(async move {
                    let result = cache.fetch_uncached(fetch_source, &id).await;
                    if let Ok(mut pending) = cache.pending.lock() {
                        pending.remove(&id);
                    }
                    let _ = sender.send(Some(CoverFetchResult(result.map_err(Arc::new))));
                });
                receiver
            }
        };
        loop {
            if let Some(CoverFetchResult(result)) = receiver.borrow().clone() {
                return result.map_err(|error| Error::new(SharedCoverFetchError(error)));
            }
            receiver
                .changed()
                .await
                .map_err(|_| anyhow::anyhow!("Service cover fetch task ended without a result"))?;
        }
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

    async fn fetch_uncached<S: CoverSource>(
        &self,
        source: Arc<S>,
        cover_id: &str,
    ) -> Result<Option<String>> {
        // A request that missed the cache just before an earlier fetch for this cover stored
        // its file, and found no fetch in flight just after that fetch finished, starts this
        // one: the file is there now, so it must not be fetched twice.
        if let Some(path) = self.cached_path(cover_id).await? {
            return Ok(Some(path));
        }
        let permit = tokio::select! {
            _ = self.cancellation.cancelled() => return Ok(None),
            permit = self.concurrency.acquire() => permit?,
        };
        let bytes = tokio::select! {
            _ = self.cancellation.cancelled() => return Ok(None),
            result = source.cover_bytes(cover_id) => result?,
        };
        if bytes.is_empty() || bytes.len() > MAX_COVER_BYTES {
            return Ok(None);
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
        self.broadcaster.broadcast(&LibraryContentChanged {
            path: String::new(),
        });
        drop(permit);
        self.cached_path(cover_id).await
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

    use tempfile::TempDir;
    use uuid::Uuid;

    use super::{
        CoverCacheLimits, cover_cache_root, delete_service_cover_cache, evict_cover_cache,
    };

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
