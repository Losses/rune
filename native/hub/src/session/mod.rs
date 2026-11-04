//! Node-owned render state. Catalog resources are leased only while resolving an item.
mod store;

use std::{
    future::pending,
    path::{Path, PathBuf},
    slice::from_ref,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use database::{
    actions::{
        logging::{LogLevel, insert_log},
        playback_queue::{SessionQueue, list_legacy_playback_queue},
        stats::increase_played_through,
    },
    connection::MainDbConnection,
    playing_item::{PlayingItemMetadataSummary, dispatcher::PlayingItemActionDispatcher},
};
use fsio::FsIo;
use playback::{
    controller::{MediaControlManager, handle_media_control_event},
    player::{Playable, PlaybackState, Player, PlayerStatus, PlayingItem, PlaylistStatus},
};
use scrobbling::manager::{ScrobblingManager, ScrobblingServiceManager};
use sea_orm::TransactionTrait;
use tokio::{
    sync::Mutex,
    task::JoinHandle,
    time::{Instant, interval},
};
use tokio_util::sync::CancellationToken;

use crate::{
    library_pool::LibraryHandle,
    messages::{
        CrashResponse, PlaybackStatus, PlaylistItem, PlaylistUpdate, RealtimeFFT,
        ScrobbleServiceStatus, ScrobbleServiceStatusUpdated,
    },
    node::NodeContext,
    server::PairedAudioResolver,
    utils::{
        Broadcaster,
        player::{
            metadata_summary_to_scrobbling_track, update_media_controls_metadata,
            update_media_controls_progress,
        },
    },
    visualizer::{VISUALIZER, playing_cover_changed},
};
use store::SessionState;

/// One renderer and its durable queue, with node-scoped event listeners.
pub struct PlayerSession {
    pub player: Arc<Mutex<dyn Playable>>,
    pub scrobbler: Arc<Mutex<dyn ScrobblingServiceManager>>,
    controls: Mutex<Option<Arc<Mutex<MediaControlManager>>>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    cancel: CancellationToken,
    state: Arc<Mutex<SessionState>>,
    path: PathBuf,
    node: Weak<NodeContext>,
    legacy_restore: AtomicBool,
}

impl PlayerSession {
    pub(crate) async fn new(
        node: &Arc<NodeContext>,
        name: &str,
        os_media_controls: bool,
    ) -> Result<Arc<Self>> {
        let path = Path::new(&*node.config_path)
            .join("sessions")
            .join(format!("{name}.json"));
        let state = SessionState::read(&path)?;
        let cancel = node.token.child_token();
        #[cfg(not(target_os = "android"))]
        let fsio = FsIo::new();
        #[cfg(target_os = "android")]
        let fsio = FsIo::new(Path::new(".rune/.android-fs.db"), "")?;
        let player: Arc<Mutex<dyn Playable>> = Arc::new(Mutex::new(Player::new(
            Arc::new(fsio),
            Some(cancel.child_token()),
        )));
        player
            .lock()
            .await
            .set_source_resolver(Arc::new(PairedAudioResolver(
                Arc::downgrade(node),
                String::new(),
            )));
        let controls = if os_media_controls {
            match MediaControlManager::with_name(&format!(
                "rune_player_{}_{}",
                node.node_id.replace('-', "_"),
                name
            ))
            .and_then(|mut manager| {
                manager.initialize()?;
                Ok(manager)
            }) {
                Ok(manager) => Some(Arc::new(Mutex::new(manager))),
                Err(error) => {
                    log::warn!("OS media controls unavailable: {error:#}");
                    None
                }
            }
        } else {
            None
        };
        let session = Arc::new(Self {
            player,
            scrobbler: Arc::new(Mutex::new(ScrobblingManager::new(
                10,
                Duration::from_secs(5),
            ))),
            controls: Mutex::new(controls),
            tasks: Mutex::new(Vec::new()),
            cancel,
            state: Arc::new(Mutex::new(state.clone())),
            path,
            node: Arc::downgrade(node),
            legacy_restore: AtomicBool::new(false),
        });
        let queue = Arc::new(SessionQueue::new(
            state.tracks().into_iter().map(|(item, _)| item).collect(),
        ));
        node.library_pool.set_session_queue(queue.clone()).await;
        session.start_events(node, queue).await;
        session.player.lock().await.restore(
            state.tracks(),
            state.index,
            state.position,
            state.playback_mode(),
        );
        Ok(session)
    }

    /// Import the first opened catalog queue only when this session has no store.
    pub async fn migrate_legacy(&self, library: &LibraryHandle) -> Result<()> {
        let mut state = self.state.lock().await;
        if state.migrated {
            return Ok(());
        }
        let ids = list_legacy_playback_queue(&library.main_db).await?;
        let items = ids
            .into_iter()
            .map(|id| PlayingItem::InLibrary(id, library.descriptor.library_id.to_string()))
            .collect::<Vec<_>>();
        let mut status = self.player.lock().await.get_status();
        // A controller may have populated the session before the first library opens.
        if status.playlist.is_empty() {
            status.playlist = items.clone();
            state.update(&status);
            state.migrated = true;
            state.write(&self.path)?;
            self.legacy_restore
                .store(!items.is_empty(), Ordering::Release);
            self.player.lock().await.restore(
                state.tracks(),
                None,
                Duration::ZERO,
                state.playback_mode(),
            );
        } else {
            state.update(&status);
            state.migrated = true;
            state.write(&self.path)?;
        }
        Ok(())
    }

    /// Consume the one-time request for the app to restore its legacy cursor.
    pub fn take_legacy_restore(&self) -> bool {
        self.legacy_restore.swap(false, Ordering::AcqRel)
    }

    /// Flush durable state and await the event listeners before terminating playback.
    pub async fn shutdown(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            return Ok(());
        }
        let status = self.player.lock().await.get_status();
        self.cancel.cancel();
        for task in self.tasks.lock().await.drain(..) {
            task.await.context("Player session listener failed")?;
        }
        if let Some(manager) = self.controls.lock().await.take()
            && let Err(error) = manager.lock().await.controls.detach()
        {
            log::warn!("Could not detach OS media controls: {error:?}");
        }
        let mut state = self.state.lock().await;
        state.update(&status);
        let result = state.write(&self.path);
        self.player.lock().await.terminate();
        result
    }

    async fn start_events(&self, node: &Arc<NodeContext>, queue: Arc<SessionQueue>) {
        let player = self.player.lock().await;
        let status = player.subscribe_status();
        let playlist = player.subscribe_playlist();
        let played = player.subscribe_played_through();
        let fft = player.subscribe_realtime_fft();
        let crash = player.subscribe_crash();
        let logs = player.subscribe_log();
        drop(player);
        let scrobbler = self.scrobbler.lock().await;
        let errors = scrobbler.subscribe_error();
        let login = scrobbler.subscribe_login_status();
        drop(scrobbler);
        let controls = self.controls.lock().await.clone();
        let controller = if let Some(manager) = &controls {
            Some(manager.lock().await.subscribe_controller_events())
        } else {
            None
        };
        let weak = self.node.clone();
        let broadcaster = node.broadcaster.clone();
        let player = self.player.clone();
        let scrobbler = self.scrobbler.clone();
        let cancel = self.cancel.clone();
        let event_task = tokio::spawn(async move {
            let mut last_item = None;
            let mut metadata = PlayingItemMetadataSummary::default();
            let mut cover = None;
            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        if let Some(manager) = &controls { let _ = manager.lock().await.controls.detach(); }
                        return;
                    },
                    Ok(value) = status.recv() => {
                        let Some(node) = weak.upgrade() else { return };
                        if value.item != last_item {
                            metadata = PlayingItemMetadataSummary::default(); cover = None;
                            if let Some(item) = &value.item
                                && let Ok((summary, art)) = item_metadata(&node, item, true).await {
                                metadata = summary; cover = art;
                                if let Some(manager) = &controls { let _ = update_media_controls_metadata(manager.clone(), &metadata, cover.as_ref()).await; }
                                scrobbler.lock().await.update_now_playing_all(metadata_summary_to_scrobbling_track(&metadata));
                            }
                            playing_cover_changed(cover.as_deref());
                            last_item = value.item.clone();
                        }
                        let library = match &value.item {
                            Some(PlayingItem::InLibrary(_, id)) => match id.parse() { Ok(id) => node.library_pool.acquire(id).await.ok(), Err(_) => None },
                            _ => None,
                        };
                        if let Some(library) = &library { VISUALIZER.set_fsio(library.fsio.clone()); }
                        let root = library.as_ref().map(|library| library.media_root.clone()).unwrap_or_default();
                        let playing = value.state == PlaybackState::Playing;
                        VISUALIZER.set_playing(playing);
                        VISUALIZER.set_playback(value.path.as_deref(), value.position, playing);
                        let report = format_status(&value, &metadata, cover.clone(), root);
                        if let Some(manager) = &controls { let _ = update_media_controls_progress(manager.clone(), &report).await; }
                        broadcaster.broadcast(&report);
                    }
                    Ok(value) = playlist.recv() => {
                        if let Some(node) = weak.upgrade() { send_playlist(&node, &value, broadcaster.as_ref()).await; }
                    }
                    Ok(item) = played.recv() => {
                        if let Some(node) = weak.upgrade() {
                            if let Err(error) = record_played(&node, &item).await { log::warn!("Could not record played item: {error:#}"); }
                            if let Ok((metadata, _)) = item_metadata(&node, &item, false).await { scrobbler.lock().await.scrobble_all(metadata_summary_to_scrobbling_track(&metadata)); }
                        }
                    }
                    Ok(value) = fft.recv() => broadcaster.broadcast(&RealtimeFFT { value }),
                    Ok(detail) = crash.recv() => broadcaster.broadcast(&CrashResponse { detail }),
                    Ok(error) = logs.recv() => {
                        let item = player.lock().await.get_status().item;
                        if let (Some(node), Some(item)) = (weak.upgrade(), item) { let _ = item_log(&node, &item, error.domain, error.error).await; }
                    }
                    Ok(error) = errors.recv() => {
                        if let (Some(node), Some(library_id)) = (weak.upgrade(), error.library_id.clone()) {
                            let item = PlayingItem::InLibrary(0, library_id);
                            let _ = item_log(&node, &item, format!("scrobbler::{:?}::{:?}", error.action, error.service), format!("{error:#?}")).await;
                        } else { log::warn!("Scrobbler error without catalog context: {error:#?}"); }
                    }
                    Ok(value) = login.recv() => broadcaster.broadcast(&ScrobbleServiceStatusUpdated { services: value.into_iter().map(|x| ScrobbleServiceStatus { service_id: x.service.to_string(), is_available: x.is_available, error: x.error_message }).collect() }),
                    Some(value) = async { match &controller { Some(receiver) => receiver.recv().await.ok(), None => pending().await } } => { let _ = handle_media_control_event(&player, value).await; }
                }
            }
        });
        let state = self.state.clone();
        let path = self.path.clone();
        let player = self.player.clone();
        let cancel = self.cancel.clone();
        let persistence_task = tokio::spawn(async move {
            let mut tick = interval(Duration::from_millis(100));
            let mut written = state.lock().await.clone();
            let mut dirty_since = None;
            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    _ = tick.tick() => {
                        let status = player.lock().await.get_status();
                        *queue.write().unwrap_or_else(|e| e.into_inner()) = status.playlist.clone();
                        let mut current = state.lock().await;
                        current.update(&status);
                        if *current != written {
                            let since = dirty_since.get_or_insert_with(Instant::now);
                            if since.elapsed() >= Duration::from_millis(300) {
                                match current.write(&path) { Ok(()) => { written = current.clone(); dirty_since = None; }, Err(error) => log::error!("Could not persist player session: {error:#}") }
                            }
                        } else { dirty_since = None; }
                    }
                }
            }
        });
        self.tasks
            .lock()
            .await
            .extend([event_task, persistence_task]);
    }
}

async fn item_metadata(
    node: &NodeContext,
    item: &PlayingItem,
    with_cover: bool,
) -> Result<(PlayingItemMetadataSummary, Option<String>)> {
    let dispatcher = PlayingItemActionDispatcher::new();
    let items = from_ref(item);
    if let PlayingItem::InLibrary(_, library_id) = item {
        let library = node.library_pool.acquire(library_id.parse()?).await?;
        let metadata = dispatcher
            .get_metadata_summary(&library.fsio, &library.main_db, items)
            .await?
            .into_iter()
            .next()
            .context("Track metadata unavailable")?;
        let cover = if with_cover {
            dispatcher
                .bake_cover_art(&library.fsio, &library.media_root, &library.main_db, items)
                .await?
                .remove(item)
        } else {
            None
        };
        Ok((metadata, cover))
    } else {
        #[cfg(not(target_os = "android"))]
        let fsio = FsIo::new();
        #[cfg(target_os = "android")]
        let fsio = FsIo::new(Path::new(".rune/.android-fs.db"), "")?;
        let db = MainDbConnection::default();
        let metadata = dispatcher
            .get_metadata_summary(&fsio, &db, items)
            .await?
            .into_iter()
            .next()
            .context("File metadata unavailable")?;
        let cover = if with_cover {
            dispatcher
                .bake_cover_art(&fsio, &"", &db, items)
                .await?
                .remove(item)
        } else {
            None
        };
        Ok((metadata, cover))
    }
}

async fn record_played(node: &NodeContext, item: &PlayingItem) -> Result<()> {
    if let PlayingItem::InLibrary(id, library_id) = item {
        let library = node.library_pool.acquire(library_id.parse()?).await?;
        let store = library.library_sync.record_store();
        increase_played_through(&library.main_db, &store, *id, store.author()).await?;
    }
    Ok(())
}

async fn item_log(
    node: &NodeContext,
    item: &PlayingItem,
    domain: String,
    detail: String,
) -> Result<()> {
    if let PlayingItem::InLibrary(_, library_id) = item {
        let library = node.library_pool.acquire(library_id.parse()?).await?;
        let tx = library.main_db.begin().await?;
        insert_log(&tx, LogLevel::Error, domain, detail).await?;
        tx.commit().await?;
    }
    Ok(())
}

async fn send_playlist(
    node: &NodeContext,
    playlist: &PlaylistStatus,
    broadcaster: &dyn Broadcaster,
) {
    let mut items = Vec::with_capacity(playlist.items.len());
    for item in &playlist.items {
        let result = item_metadata(node, item, false).await;
        let unavailable = playlist.unavailable.contains(item) || result.is_err();
        let metadata = result.map(|(metadata, _)| metadata).unwrap_or_default();
        items.push(PlaylistItem {
            item: item.clone().into(),
            artist: metadata.artist,
            album: metadata.album,
            title: metadata.title,
            duration: metadata.duration,
            unavailable,
        });
    }
    broadcaster.broadcast(&PlaylistUpdate { items });
}

fn format_status(
    status: &PlayerStatus,
    metadata: &PlayingItemMetadataSummary,
    cover_art_path: Option<String>,
    lib_path: String,
) -> PlaybackStatus {
    PlaybackStatus {
        state: status.state.to_string(),
        progress_seconds: status.position.as_secs_f32(),
        progress_percentage: if metadata.duration > 0.0 {
            status.position.as_secs_f32() / metadata.duration as f32
        } else {
            0.0
        },
        artist: Some(metadata.artist.clone()),
        album: Some(metadata.album.clone()),
        title: Some(metadata.title.clone()),
        duration: metadata.duration,
        index: status.index.map(|i| i as i32),
        item: status.item.clone().map(Into::into),
        playback_mode: status.playback_mode.into(),
        ready: status.ready,
        cover_art_path,
        lib_path,
    }
}

impl Drop for PlayerSession {
    fn drop(&mut self) {
        self.cancel.cancel();
        for task in self.tasks.get_mut().drain(..) {
            task.abort();
        }
    }
}
