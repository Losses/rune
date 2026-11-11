use crate::{messages::LibraryContentChanged, utils::Broadcaster};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use anyhow::Result;
use subsonic::{SubsonicAlbum, SubsonicArtist, SubsonicClient, SubsonicSong};

use crate::request_error::ExpectedRequestError;

/// Opaque catalog IDs are scoped to the lifetime of the open Subsonic binding.
#[derive(Default)]
pub(crate) struct SubsonicIdMap {
    forward: HashMap<SubsonicIdKind, HashMap<String, i32>>,
    reverse: HashMap<SubsonicIdKind, HashMap<i32, String>>,
    next: HashMap<SubsonicIdKind, i32>,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum SubsonicIdKind {
    Artist,
    Album,
    Track,
    CoverArt,
}

impl SubsonicIdMap {
    pub(crate) fn intern(&mut self, kind: SubsonicIdKind, remote_id: &str) -> Result<i32> {
        if let Some(id) = self.forward.get(&kind).and_then(|ids| ids.get(remote_id)) {
            return Ok(*id);
        }
        let next = self.next.entry(kind).or_default();
        *next = next
            .checked_add(1)
            .ok_or_else(|| ExpectedRequestError::invalid_input("Subsonic ID map is full"))?;
        let id = *next;
        self.forward
            .entry(kind)
            .or_default()
            .insert(remote_id.to_owned(), id);
        self.reverse
            .entry(kind)
            .or_default()
            .insert(id, remote_id.to_owned());
        Ok(id)
    }

    pub(crate) fn resolve(&self, kind: SubsonicIdKind, id: i32) -> Result<&str> {
        self.reverse
            .get(&kind)
            .and_then(|ids| ids.get(&id))
            .map(String::as_str)
            .ok_or_else(|| {
                ExpectedRequestError::invalid_input("Unknown Subsonic catalog ID").into()
            })
    }
}

pub(crate) struct SubsonicBinding {
    pub(crate) client: Arc<SubsonicClient>,
    pub(crate) probe: subsonic::SubsonicProbe,
    pub(crate) catalog: Mutex<SubsonicCatalog>,
    pub(crate) covers: Arc<SubsonicCoverCache>,
}

pub(crate) struct SubsonicCoverCache {
    pub(crate) root: PathBuf,
    pub(crate) pending: Mutex<HashSet<String>>,
    pub(crate) concurrency: Semaphore,
    pub(crate) cancellation: CancellationToken,
    pub(crate) broadcaster: Arc<dyn Broadcaster>,
}

pub(crate) async fn delete_subsonic_cover_cache(
    config_dir: &Path,
    library_id: Uuid,
) -> std::io::Result<()> {
    let path = config_dir
        .join("subsonic-covers")
        .join(library_id.to_string());
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

impl SubsonicCoverCache {
    pub(crate) fn path_for(&self, cover_id: &str) -> PathBuf {
        let key = blake3::hash(cover_id.as_bytes()).to_hex();
        self.root.join(format!("{key}.img"))
    }

    pub(crate) fn cached_path(&self, cover_id: &str) -> Option<String> {
        let path = self.path_for(cover_id);
        let metadata = fs::metadata(&path).ok()?;
        if !metadata.is_file() || metadata.len() == 0 {
            return None;
        }
        if let Ok(file) = fs::OpenOptions::new().write(true).open(&path) {
            let _ = file.set_times(fs::FileTimes::new().set_modified(SystemTime::now()));
        }
        path.to_str().map(str::to_owned)
    }

    pub(crate) fn schedule(self: &Arc<Self>, client: Arc<SubsonicClient>, cover_id: String) {
        if self.path_for(&cover_id).is_file() {
            return;
        }
        let Ok(mut pending) = self.pending.lock() else {
            return;
        };
        if !pending.insert(cover_id.clone()) {
            return;
        }
        drop(pending);
        let cache = Arc::clone(self);
        tokio::spawn(async move {
            let permit = cache.concurrency.acquire().await;
            if permit.is_err() {
                if let Ok(mut pending) = cache.pending.lock() {
                    pending.remove(&cover_id);
                }
                return;
            }
            let path = cache.path_for(&cover_id);
            let fetched = tokio::select! {
                _ = cache.cancellation.cancelled() => None,
                result = client.cover_art(&cover_id, None) => Some(result),
            };
            let Some(fetched) = fetched else {
                if let Ok(mut pending) = cache.pending.lock() {
                    pending.remove(&cover_id);
                }
                drop(permit);
                return;
            };
            if let Ok(bytes) = fetched
                && !bytes.is_empty()
                && bytes.len() <= 8 * 1024 * 1024
            {
                let result = (|| -> std::io::Result<()> {
                    fs::create_dir_all(&cache.root)?;
                    let temporary = path.with_extension("tmp");
                    fs::write(&temporary, bytes)?;
                    fs::rename(&temporary, &path)?;
                    evict_cover_cache(&cache.root, &path, COVER_CACHE_LIMITS)
                })();
                if result.is_ok() {
                    let notification = LibraryContentChanged {
                        path: String::new(),
                    };
                    cache.broadcaster.broadcast(&notification);
                }
            }
            if let Ok(mut pending) = cache.pending.lock() {
                pending.remove(&cover_id);
            }
            drop(permit);
        });
    }
}

/// Bounds of one Subsonic library's cover cache. A cover wall or album list shows
/// hundreds of covers, so the cache must hold several pages of them.
#[derive(Clone, Copy)]
struct CoverCacheLimits {
    max_files: usize,
    max_bytes: u64,
}

const COVER_CACHE_LIMITS: CoverCacheLimits = CoverCacheLimits {
    max_files: 4096,
    max_bytes: 256 * 1024 * 1024,
};

fn evict_cover_cache(root: &Path, newest: &Path, limits: CoverCacheLimits) -> std::io::Result<()> {
    let mut entries = fs::read_dir(root)?
        .filter_map(Result::ok)
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

#[derive(Default)]
pub(crate) struct SubsonicCatalog {
    pub(crate) ids: SubsonicIdMap,
    pub(crate) artists: HashMap<i32, SubsonicArtist>,
    pub(crate) albums: HashMap<i32, SubsonicAlbum>,
    pub(crate) songs: HashMap<i32, SubsonicSong>,
    pub(crate) artists_loaded: bool,
    pub(crate) albums_loaded: bool,
}

impl SubsonicBinding {
    pub(crate) fn new(
        client: SubsonicClient,
        probe: subsonic::SubsonicProbe,
        config_dir: PathBuf,
        library_id: Uuid,
        broadcaster: Arc<dyn Broadcaster>,
    ) -> Self {
        Self {
            client: Arc::new(client),
            probe,
            catalog: Mutex::new(SubsonicCatalog::default()),
            covers: Arc::new(SubsonicCoverCache {
                root: config_dir
                    .join("subsonic-covers")
                    .join(library_id.to_string()),
                pending: Mutex::new(HashSet::new()),
                concurrency: Semaphore::new(4),
                cancellation: CancellationToken::new(),
                broadcaster,
            }),
        }
    }
}

impl Drop for SubsonicBinding {
    fn drop(&mut self) {
        self.covers.cancellation.cancel();
    }
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
        CoverCacheLimits, SubsonicIdKind, SubsonicIdMap, delete_subsonic_cover_cache,
        evict_cover_cache,
    };
    use crate::request_error::ExpectedRequestError;

    #[test]
    fn ids_are_stable_per_kind_and_unknown_ids_are_typed() {
        let mut ids = SubsonicIdMap::default();
        let artist = ids.intern(SubsonicIdKind::Artist, "artist-opaque").unwrap();
        assert_eq!(artist, 1);
        assert_eq!(
            ids.intern(SubsonicIdKind::Artist, "artist-opaque").unwrap(),
            artist
        );
        assert_eq!(
            ids.intern(SubsonicIdKind::Album, "album-opaque").unwrap(),
            1
        );
        assert_eq!(
            ids.resolve(SubsonicIdKind::Artist, artist).unwrap(),
            "artist-opaque"
        );
        let error = ids.resolve(SubsonicIdKind::Track, 999).unwrap_err();
        assert!(error.downcast_ref::<ExpectedRequestError>().is_some());
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
    async fn removing_subsonic_record_deletes_its_cover_cache_directory() {
        let root = TempDir::new().unwrap();
        let library_id = Uuid::new_v4();
        let cache = root
            .path()
            .join("subsonic-covers")
            .join(library_id.to_string());
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("cover.img"), b"cover").unwrap();
        delete_subsonic_cover_cache(root.path(), library_id)
            .await
            .unwrap();
        assert!(!cache.exists());
    }
}
