use crate::{
    messages::{
        LibraryContentChanged, PlaybackStatus, PlayingItemRef, PlaylistItem, PlaylistUpdate,
    },
    utils::Broadcaster,
};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use anyhow::Result;
use futures::future::BoxFuture;
use playback::player::{
    AudioPlaybackNote, AudioRequest, AudioSourceMode, RemoteAudioError, RemoteAudioSource,
    SubsonicItem, UnavailableReason,
};
use subsonic::{SubsonicAlbum, SubsonicArtist, SubsonicClient, SubsonicJukebox, SubsonicSong};

use crate::request_error::ExpectedRequestError;
use crate::subsonic_translate::jukebox_error_for_refresh;

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
    pub(crate) library_id: Uuid,
    pub(crate) play_on_this_device: bool,
    pub(crate) client: Arc<SubsonicClient>,
    pub(crate) probe: subsonic::SubsonicProbe,
    pub(crate) catalog: Mutex<SubsonicCatalog>,
    pub(crate) covers: Arc<SubsonicCoverCache>,
    jukebox_poll: Mutex<Option<CancellationToken>>,
    jukebox_state: Arc<Mutex<Option<JukeboxPollState>>>,
}

#[derive(Clone, Debug, PartialEq)]
struct JukeboxPollState {
    jukebox: SubsonicJukebox,
    // The server exposes no queue revision, so this local revision is best effort.
    revision: u64,
}

fn broadcast_jukebox(
    broadcaster: &dyn Broadcaster,
    library_id: Uuid,
    jukebox: &SubsonicJukebox,
    revision: u64,
) {
    let session_id = library_id.to_string();
    let items = jukebox
        .playlist
        .songs
        .iter()
        .map(|song| PlaylistItem {
            unavailable: None,
            item: PlayingItemRef::LibraryTrack {
                library_id: session_id.clone(),
                track: song.id.clone(),
            },
            artist: song.artist.clone().unwrap_or_default(),
            album: song.album.clone().unwrap_or_default(),
            title: song.title.clone(),
            duration: f64::from(song.duration.unwrap_or_default()),
        })
        .collect();
    broadcaster.broadcast(&PlaylistUpdate {
        items,
        session_id: session_id.clone(),
        revision,
    });

    let current = jukebox
        .status
        .current_index
        .and_then(|index| jukebox.playlist.songs.get(index as usize));
    let position = jukebox.status.position_secs.unwrap_or_default() as f32;
    let duration = current
        .and_then(|song| song.duration)
        .map(f64::from)
        .unwrap_or_default();
    broadcaster.broadcast(&PlaybackStatus {
        session_id,
        state: if jukebox.status.playing {
            "playing".into()
        } else {
            "paused".into()
        },
        progress_seconds: position,
        progress_percentage: if duration > 0.0 {
            position / duration as f32
        } else {
            0.0
        },
        artist: current.and_then(|song| song.artist.clone()),
        album: current.and_then(|song| song.album.clone()),
        title: current.map(|song| song.title.clone()),
        duration,
        index: jukebox
            .status
            .current_index
            .and_then(|index| i32::try_from(index).ok()),
        item: current.map(|song| song.id.clone()),
        item_request: current.map(|song| PlayingItemRef::LibraryTrack {
            library_id: library_id.to_string(),
            track: song.id.clone(),
        }),
        playback_mode: 0,
        ready: current.is_some(),
        cover_art_path: None,
        lib_path: String::new(),
    });
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
        play_on_this_device: bool,
        broadcaster: Arc<dyn Broadcaster>,
    ) -> Self {
        Self {
            library_id,
            play_on_this_device,
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
            jukebox_poll: Mutex::new(None),
            jukebox_state: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn start_jukebox_polling(self: &Arc<Self>) {
        if self.play_on_this_device || !self.probe.roles.jukebox_role {
            return;
        }
        let Ok(mut poll) = self.jukebox_poll.lock() else {
            return;
        };
        if let Some(token) = poll.take() {
            token.cancel();
        }
        let cancellation = self.covers.cancellation.child_token();
        *poll = Some(cancellation.clone());
        let client = Arc::clone(&self.client);
        let library_id = self.library_id;
        let broadcaster = Arc::clone(&self.covers.broadcaster);
        let binding_state = Arc::clone(&self.jukebox_state);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3));
            loop {
                tokio::select! {
                    _ = cancellation.cancelled() => break,
                    _ = interval.tick() => {
                        let result = tokio::select! {
                            _ = cancellation.cancelled() => break,
                            result = async {
                                let status = client.jukebox_status().await?;
                                let mut jukebox = client.jukebox_get().await?;
                                jukebox.status = status;
                                Ok::<_, subsonic::SubsonicError>(jukebox)
                            } => result,
                        };
                        match result {
                            Ok(jukebox) => {
                                let changed = {
                                    let Ok(mut previous) = binding_state.lock() else { continue; };
                                    let revision = match previous.as_ref() {
                                        Some(current) if current.jukebox == jukebox => continue,
                                        Some(current) => current.revision.saturating_add(1),
                                        None => 1,
                                    };
                                    *previous = Some(JukeboxPollState {
                                        jukebox: jukebox.clone(),
                                        revision,
                                    });
                                    revision
                                };
                                broadcast_jukebox(&*broadcaster, library_id, &jukebox, changed);
                            }
                            Err(error) => log::debug!("Subsonic jukebox status poll failed: {error}"),
                        }
                    }
                }
            }
        });
    }

    pub(crate) fn stop_jukebox_polling(&self) {
        if let Ok(mut poll) = self.jukebox_poll.lock()
            && let Some(token) = poll.take()
        {
            token.cancel();
        }
    }

    pub(crate) async fn publish_jukebox_snapshot(&self) -> Result<()> {
        let jukebox = self
            .client
            .jukebox_get()
            .await
            .map_err(jukebox_error_for_refresh)?;
        let revision = {
            let mut state = self
                .jukebox_state
                .lock()
                .map_err(|_| anyhow::anyhow!("Subsonic jukebox state lock poisoned"))?;
            let revision = match state.as_ref() {
                Some(current) if current.jukebox == jukebox => return Ok(()),
                Some(current) => current.revision.saturating_add(1),
                None => 1,
            };
            *state = Some(JukeboxPollState {
                jukebox: jukebox.clone(),
                revision,
            });
            revision
        };
        broadcast_jukebox(
            &*self.covers.broadcaster,
            self.library_id,
            &jukebox,
            revision,
        );
        Ok(())
    }

    pub(crate) async fn playback_source(
        self: &Arc<Self>,
        item: SubsonicItem,
    ) -> Result<RemoteAudioSource> {
        let song = self
            .client
            .song(&item.track_id)
            .await
            .map_err(subsonic_playback_error)?;
        let extension = song
            .suffix
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "audio".to_owned());
        let cancellation = CancellationToken::new();
        let failure_reason = Arc::new(Mutex::new(None));
        Ok(RemoteAudioSource {
            request: Arc::new(SubsonicAudioRequest {
                client: Arc::clone(&self.client),
                track_id: item.track_id,
                failure_reason: failure_reason.clone(),
            }),
            // The request capability owns the authenticated URL. This parse-only URL has no
            // query, credentials, or server identity and is never requested directly.
            url: "https://subsonic.invalid/rest/stream".to_owned(),
            extension,
            size: song.size.unwrap_or_default(),
            cancellation,
            failure_reason,
        })
    }

    pub(crate) async fn song(&self, track_id: &str) -> Result<SubsonicSong> {
        self.client
            .song(track_id)
            .await
            .map_err(subsonic_playback_error)
    }

    pub(crate) fn cover_path(&self, cover_id: Option<&str>) -> Option<String> {
        let cover_id = cover_id?;
        self.covers
            .schedule(Arc::clone(&self.client), cover_id.to_owned());
        self.covers.cached_path(cover_id)
    }
}

struct SubsonicAudioRequest {
    client: Arc<SubsonicClient>,
    track_id: String,
    failure_reason: Arc<std::sync::Mutex<Option<UnavailableReason>>>,
}

impl AudioRequest for SubsonicAudioRequest {
    fn prepare(&self) -> BoxFuture<'static, Result<AudioSourceMode>> {
        let client = Arc::clone(&self.client);
        let track_id = self.track_id.clone();
        let failure_reason = self.failure_reason.clone();
        Box::pin(async move {
            let response = client
                .stream(&track_id, None)
                .await
                .map_err(|error| stream_playback_error(error, &failure_reason))?;
            let supports_range = response
                .headers()
                .get(reqwest::header::ACCEPT_RANGES)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.eq_ignore_ascii_case("bytes"));
            if supports_range {
                return Ok(AudioSourceMode::Ranged);
            }
            let bytes = response.bytes().await.map_err(|_| {
                if let Ok(mut reason) = failure_reason.lock() {
                    *reason = Some(UnavailableReason::SourceUnreachable);
                }
                anyhow::Error::new(RemoteAudioError::unreachable("Subsonic server unreachable"))
            })?;
            Ok(AudioSourceMode::Buffered {
                bytes: bytes.to_vec(),
                note: AudioPlaybackNote::RangeUnsupportedBuffered,
            })
        })
    }

    fn get(
        &self,
        range: Option<(u64, Option<u64>)>,
    ) -> BoxFuture<'static, Result<reqwest::Response>> {
        let client = Arc::clone(&self.client);
        let track_id = self.track_id.clone();
        let failure_reason = self.failure_reason.clone();
        Box::pin(async move {
            client
                .stream(&track_id, range)
                .await
                .map_err(|error| stream_playback_error(error, &failure_reason))
        })
    }
}

fn stream_playback_error(
    error: subsonic::StreamError,
    failure_reason: &std::sync::Mutex<Option<UnavailableReason>>,
) -> anyhow::Error {
    let (reason, failure) = match error {
        subsonic::StreamError::Unreachable => (
            UnavailableReason::SourceUnreachable,
            RemoteAudioError::unreachable("Subsonic server unreachable"),
        ),
        subsonic::StreamError::CredentialsRejected => (
            UnavailableReason::CredentialsRejected,
            RemoteAudioError::CredentialsRejected,
        ),
        subsonic::StreamError::TrackGone => {
            (UnavailableReason::TrackGone, RemoteAudioError::TrackGone)
        }
        subsonic::StreamError::Rejected => (
            UnavailableReason::SourceUnreachable,
            RemoteAudioError::unreachable("Subsonic stream rejected"),
        ),
    };
    if let Ok(mut stored) = failure_reason.lock() {
        *stored = Some(reason);
    }
    anyhow::Error::new(failure)
}

fn subsonic_playback_error(error: subsonic::SubsonicError) -> anyhow::Error {
    match error {
        subsonic::SubsonicError::Transport(_) | subsonic::SubsonicError::HttpStatus(_) => {
            anyhow::Error::new(RemoteAudioError::unreachable("Subsonic server unreachable"))
        }
        subsonic::SubsonicError::Server {
            kind:
                subsonic::SubsonicServerError::WrongCredentials
                | subsonic::SubsonicServerError::NotAuthorized
                | subsonic::SubsonicServerError::InvalidApiKey,
            ..
        } => anyhow::Error::new(RemoteAudioError::CredentialsRejected),
        subsonic::SubsonicError::Server {
            kind: subsonic::SubsonicServerError::NotFound,
            ..
        } => anyhow::Error::new(RemoteAudioError::TrackGone),
        _ => anyhow::Error::new(RemoteAudioError::TrackGone),
    }
}

impl Drop for SubsonicBinding {
    fn drop(&mut self) {
        self.stop_jukebox_polling();
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
