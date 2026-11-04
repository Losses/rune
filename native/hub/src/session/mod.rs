//! Node-owned render state. Catalog resources are leased only while resolving an item.
mod store;

use std::{
    collections::HashMap,
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
        playback_queue::list_legacy_playback_queue,
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
use scrobbling::manager::{
    LoginStatus, ScrobblingError, ScrobblingManager, ScrobblingServiceManager,
};
use sea_orm::TransactionTrait;
use tokio::{
    sync::{Mutex, mpsc},
    task::JoinHandle,
    time::{Instant, sleep_until},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

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
    pub async fn queue_for_library(&self, library_id: Uuid) -> Vec<i32> {
        let library_id = library_id.to_string();
        self.player
            .lock()
            .await
            .get_status()
            .playlist
            .into_iter()
            .filter_map(|item| match item {
                PlayingItem::InLibrary(file_id, id) if id == library_id => Some(file_id),
                _ => None,
            })
            .collect()
    }

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
        session.start_events(node).await;
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

    async fn start_events(&self, node: &Arc<NodeContext>) {
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
        let (persist_status_tx, mut persist_status_rx) = mpsc::channel(32);
        let (persist_playlist_tx, mut persist_playlist_rx) = mpsc::channel(32);
        let event_task = tokio::spawn(async move {
            let mut item_state = ItemEventState::default();
            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        if let Some(manager) = &controls { let _ = manager.lock().await.controls.detach(); }
                        return;
                    },
                    Ok(value) = status.recv() => {
                        let Some(node) = weak.upgrade() else { return };
                        if persist_status_tx.send(value.clone()).await.is_err() { return; }
                        handle_status_event(&node, value, &mut item_state, &controls, &scrobbler, &broadcaster).await;
                    }
                    Ok(value) = playlist.recv() => {
                        if persist_playlist_tx.send(value.clone()).await.is_err() { return; }
                        if let Some(node) = weak.upgrade() { handle_playlist_event(&node, value, broadcaster.as_ref()).await; }
                    }
                    Ok(item) = played.recv() => {
                        if let Some(node) = weak.upgrade() {
                            handle_played_event(&node, item, &scrobbler).await;
                        }
                    }
                    Ok(value) = fft.recv() => broadcaster.broadcast(&RealtimeFFT { value }),
                    Ok(detail) = crash.recv() => broadcaster.broadcast(&CrashResponse { detail }),
                    Ok(error) = logs.recv() => {
                        let item = player.lock().await.get_status().item;
                        if let (Some(node), Some(item)) = (weak.upgrade(), item) { handle_log_event(&node, item, error.domain, error.error).await; }
                    }
                    Ok(error) = errors.recv() => {
                        handle_scrobbler_error(weak.upgrade(), error).await;
                    }
                    Ok(value) = login.recv() => handle_login_event(value, broadcaster.as_ref()),
                    Some(value) = async { match &controller { Some(receiver) => receiver.recv().await.ok(), None => pending().await } } => { let _ = handle_media_control_event(&player, value).await; }
                }
            }
        });
        let state = self.state.clone();
        let path = self.path.clone();
        let cancel = self.cancel.clone();
        let persistence_task = tokio::spawn(async move {
            let mut written = state.lock().await.clone();
            let mut dirty_deadline = None;
            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        flush_session_state(&state, &path, &mut written).await;
                        return;
                    },
                    Some(value) = persist_playlist_rx.recv() => {
                        record_playlist_change(&state, value.items, &mut dirty_deadline).await;
                    }
                    Some(value) = persist_status_rx.recv() => {
                        record_status_change(&state, &value, &mut dirty_deadline).await;
                    }
                    _ = sleep_until(dirty_deadline.unwrap_or_else(Instant::now)), if dirty_deadline.is_some() =>
                        persist_when_due(&state, &path, &mut written, &mut dirty_deadline).await,
                }
            }
        });
        self.tasks
            .lock()
            .await
            .extend([event_task, persistence_task]);
    }
}

async fn update_persisted_queue(state: &Mutex<SessionState>, items: &[PlayingItem]) -> bool {
    let mut state = state.lock().await;
    let previous = state.clone();
    state.update_queue(items);
    *state != previous
}

async fn update_persisted_cursor(state: &Mutex<SessionState>, status: &PlayerStatus) -> bool {
    let mut state = state.lock().await;
    let previous = state.clone();
    state.update_cursor(status);
    *state != previous
}

async fn record_playlist_change(
    state: &Mutex<SessionState>,
    items: Vec<PlayingItem>,
    deadline: &mut Option<Instant>,
) {
    if update_persisted_queue(state, &items).await {
        deadline.get_or_insert_with(|| Instant::now() + Duration::from_millis(300));
    }
}

async fn record_status_change(
    state: &Mutex<SessionState>,
    status: &PlayerStatus,
    deadline: &mut Option<Instant>,
) {
    if update_persisted_cursor(state, status).await {
        deadline.get_or_insert_with(|| Instant::now() + Duration::from_millis(300));
    }
}

async fn persist_when_due(
    state: &Mutex<SessionState>,
    path: &Path,
    written: &mut SessionState,
    deadline: &mut Option<Instant>,
) {
    if deadline.take().is_some()
        && let Err(error) = persist_session_state(state, path, written).await
    {
        log::error!("Could not persist player session: {error:#}");
        *deadline = Some(Instant::now() + Duration::from_millis(300));
    }
}

async fn flush_session_state(state: &Mutex<SessionState>, path: &Path, written: &mut SessionState) {
    if let Err(error) = persist_session_state(state, path, written).await {
        log::error!("Could not flush player session: {error:#}");
    }
}

async fn persist_session_state(
    state: &Mutex<SessionState>,
    path: &Path,
    written: &mut SessionState,
) -> Result<()> {
    let current = state.lock().await;
    if *current != *written {
        current.write(path)?;
        *written = current.clone();
    }
    Ok(())
}

#[derive(Default)]
struct ItemEventState {
    last_item: Option<PlayingItem>,
    library: Option<ResolvedLibrary>,
    metadata: PlayingItemMetadataSummary,
    cover: Option<String>,
}

struct ResolvedLibrary {
    fsio: Arc<FsIo>,
    media_root: String,
}

async fn handle_status_event(
    node: &NodeContext,
    value: PlayerStatus,
    item_state: &mut ItemEventState,
    controls: &Option<Arc<Mutex<MediaControlManager>>>,
    scrobbler: &Arc<Mutex<dyn ScrobblingServiceManager>>,
    broadcaster: &Arc<dyn Broadcaster>,
) {
    if value.item != item_state.last_item {
        item_state.metadata = PlayingItemMetadataSummary::default();
        item_state.cover = None;
        item_state.library = None;
        if let Some(item) = &value.item {
            let result = match item {
                PlayingItem::InLibrary(_, library_id) => match library_id.parse() {
                    Ok(library_id) => match node.library_pool.acquire(library_id).await {
                        Ok(library) => {
                            let result = item_metadata_from_library(item, true, &library).await;
                            item_state.library = Some(ResolvedLibrary {
                                fsio: library.fsio.clone(),
                                media_root: library.media_root.clone(),
                            });
                            result
                        }
                        Err(error) => Err(error),
                    },
                    Err(error) => Err(error.into()),
                },
                _ => item_metadata(node, item, true).await,
            };
            if let Ok((metadata, cover)) = result {
                item_state.metadata = metadata;
                item_state.cover = cover;
                if let Some(manager) = controls {
                    let _ = update_media_controls_metadata(
                        manager.clone(),
                        &item_state.metadata,
                        item_state.cover.as_ref(),
                    )
                    .await;
                }
                scrobbler.lock().await.update_now_playing_all(
                    metadata_summary_to_scrobbling_track(&item_state.metadata),
                );
            }
        }
        playing_cover_changed(item_state.cover.as_deref());
        item_state.last_item = value.item.clone();
    }
    let root = item_state
        .library
        .as_ref()
        .map(|library| library.media_root.clone())
        .unwrap_or_default();
    let playing = value.state == PlaybackState::Playing;
    VISUALIZER.set_playing(playing);
    VISUALIZER.set_playback(
        value.path.as_deref(),
        value.position,
        playing,
        item_state
            .library
            .as_ref()
            .map(|library| library.fsio.clone()),
    );
    let report = format_status(&value, &item_state.metadata, item_state.cover.clone(), root);
    if let Some(manager) = controls {
        let _ = update_media_controls_progress(manager.clone(), &report).await;
    }
    broadcaster.broadcast(&report);
}

async fn handle_playlist_event(
    node: &NodeContext,
    value: PlaylistStatus,
    broadcaster: &dyn Broadcaster,
) {
    send_playlist(node, &value, broadcaster).await;
}

async fn handle_played_event(
    node: &NodeContext,
    item: PlayingItem,
    scrobbler: &Arc<Mutex<dyn ScrobblingServiceManager>>,
) {
    if let Err(error) = record_played(node, &item).await {
        log::warn!("Could not record played item: {error:#}");
    }
    if let Ok((metadata, _)) = item_metadata(node, &item, false).await {
        scrobbler
            .lock()
            .await
            .scrobble_all(metadata_summary_to_scrobbling_track(&metadata));
    }
}

async fn handle_log_event(node: &NodeContext, item: PlayingItem, domain: String, detail: String) {
    let _ = item_log(node, &item, domain, detail).await;
}

async fn handle_scrobbler_error(node: Option<Arc<NodeContext>>, error: ScrobblingError) {
    if let (Some(node), Some(library_id)) = (node, error.library_id.clone()) {
        let item = PlayingItem::InLibrary(0, library_id);
        let domain = format!("scrobbler::{:?}::{:?}", error.action, error.service);
        let detail = format!("{error:#?}");
        let _ = item_log(&node, &item, domain, detail).await;
    } else {
        log::warn!("Scrobbler error without catalog context: {error:#?}");
    }
}

fn handle_login_event(value: Vec<LoginStatus>, broadcaster: &dyn Broadcaster) {
    broadcaster.broadcast(&ScrobbleServiceStatusUpdated {
        services: value
            .into_iter()
            .map(|status| ScrobbleServiceStatus {
                service_id: status.service.to_string(),
                is_available: status.is_available,
                error: status.error_message,
            })
            .collect(),
    });
}

async fn item_metadata(
    node: &NodeContext,
    item: &PlayingItem,
    with_cover: bool,
) -> Result<(PlayingItemMetadataSummary, Option<String>)> {
    if let PlayingItem::InLibrary(_, library_id) = item {
        let library = node.library_pool.acquire(library_id.parse()?).await?;
        return item_metadata_from_library(item, with_cover, &library).await;
    }
    let dispatcher = PlayingItemActionDispatcher::new();
    let items = from_ref(item);
    {
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

async fn item_metadata_from_library(
    item: &PlayingItem,
    with_cover: bool,
    library: &LibraryHandle,
) -> Result<(PlayingItemMetadataSummary, Option<String>)> {
    let dispatcher = PlayingItemActionDispatcher::new();
    let items = from_ref(item);
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
    let mut library_items = HashMap::<Uuid, Vec<PlayingItem>>::new();
    let mut independent_items = Vec::new();
    for item in &playlist.items {
        match item {
            PlayingItem::InLibrary(_, library_id) => {
                if let Ok(library_id) = library_id.parse() {
                    library_items
                        .entry(library_id)
                        .or_default()
                        .push(item.clone());
                }
            }
            _ => independent_items.push(item.clone()),
        }
    }

    let dispatcher = PlayingItemActionDispatcher::new();
    let mut metadata = HashMap::<PlayingItem, PlayingItemMetadataSummary>::new();
    for (library_id, batch) in library_items {
        let Ok(library) = node.library_pool.acquire(library_id).await else {
            continue;
        };
        if let Ok(summaries) = dispatcher
            .get_metadata_summary(&library.fsio, &library.main_db, &batch)
            .await
        {
            metadata.extend(
                summaries
                    .into_iter()
                    .map(|summary| (summary.item.clone(), summary)),
            );
        }
    }

    if !independent_items.is_empty() {
        #[cfg(not(target_os = "android"))]
        let fsio_result = Ok::<_, anyhow::Error>(FsIo::new());
        #[cfg(target_os = "android")]
        let fsio_result = FsIo::new(Path::new(".rune/.android-fs.db"), "");
        let db = MainDbConnection::default();
        if let Ok(fsio) = fsio_result
            && let Ok(summaries) = dispatcher
                .get_metadata_summary(&fsio, &db, &independent_items)
                .await
        {
            metadata.extend(
                summaries
                    .into_iter()
                    .map(|summary| (summary.item.clone(), summary)),
            );
        }
    }

    let items = playlist
        .items
        .iter()
        .map(|item| {
            let summary = metadata.get(item);
            let unavailable = playlist.unavailable.contains(item) || summary.is_none();
            let metadata = summary.cloned().unwrap_or_default();
            PlaylistItem {
                item: item.clone().into(),
                artist: metadata.artist,
                album: metadata.album,
                title: metadata.title,
                duration: metadata.duration,
                unavailable,
            }
        })
        .collect();
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
