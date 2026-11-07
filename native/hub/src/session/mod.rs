//! Node-owned render state. Catalog resources are leased only while resolving an item.
pub(crate) mod store;

use std::{
    collections::HashMap,
    future::pending,
    path::{Path, PathBuf},
    slice::from_ref,
    sync::{
        Arc, Mutex as StdMutex, MutexGuard, PoisonError, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use database::{
    actions::{
        file::{file_ids_by_uuid, file_uuids_by_id},
        logging::{LogLevel, insert_log},
        playback_queue::list_legacy_playback_queue,
        stats::increase_played_through,
    },
    connection::MainDbConnection,
    error::ExpectedError,
    playing_item::{PlayingItemMetadataSummary, dispatcher::PlayingItemActionDispatcher},
};
use fsio::FsIo;
use playback::{
    controller::{MediaControlManager, handle_media_control_event},
    player::{
        Playable, PlaybackState, Player, PlayerStatus, PlayingItem, PlaylistStatus,
        UnavailableReason,
    },
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
    handlers::playback::playing_item_requests,
    library_pool::LibraryHandle,
    messages::{
        AttachedControllerInfo, ControllersUpdated, CrashResponse, PlaybackStatus,
        PlayingItemRequest, PlaylistItem, PlaylistUnavailableReason, PlaylistUpdate, RealtimeFFT,
        ScrobbleServiceStatus, ScrobbleServiceStatusUpdated,
    },
    node::NodeContext,
    request_error::ExpectedRequestError,
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
    attached_controllers: StdMutex<Vec<ControllerEntry>>,
}

struct ControllerEntry {
    controller: AttachedController,
    cancel: CancellationToken,
}

#[derive(Clone)]
struct AttachedController {
    id: Uuid,
    fingerprint: String,
    alias: String,
    connected_at: u64,
}

/// Owns one attached controller entry for the lifetime of its WebSocket.
pub(crate) struct AttachedControllerGuard {
    session: Arc<PlayerSession>,
    id: Uuid,
    cancel: CancellationToken,
}

impl AttachedControllerGuard {
    pub(crate) fn cancellation_token(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

impl Drop for AttachedControllerGuard {
    fn drop(&mut self) {
        self.session.remove_controller(self.id);
    }
}

impl PlayerSession {
    pub(crate) fn attach_controller(
        self: &Arc<Self>,
        fingerprint: String,
        alias: String,
    ) -> AttachedControllerGuard {
        let id = Uuid::new_v4();
        let cancel = CancellationToken::new();
        let connected_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        let controller = AttachedController {
            id,
            fingerprint,
            alias,
            connected_at,
        };
        self.controllers().push(ControllerEntry {
            controller,
            cancel: cancel.clone(),
        });
        self.broadcast_controllers_updated();
        AttachedControllerGuard {
            session: self.clone(),
            id,
            cancel,
        }
    }

    /// The controller list stays consistent even if a holder panicked: every
    /// critical section only pushes, retains or reads.
    fn controllers(&self) -> MutexGuard<'_, Vec<ControllerEntry>> {
        self.attached_controllers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn attached_controllers(&self) -> Vec<AttachedControllerInfo> {
        self.controllers()
            .iter()
            .map(|entry| AttachedControllerInfo {
                id: entry.controller.id.to_string(),
                fingerprint: entry.controller.fingerprint.clone(),
                alias: entry.controller.alias.clone(),
                connected_at: entry.controller.connected_at,
            })
            .collect()
    }

    pub(crate) fn detach_controller(&self, id: Uuid) -> Result<()> {
        let controllers = self.controllers();
        let Some(entry) = controllers.iter().find(|entry| entry.controller.id == id) else {
            return Err(ExpectedError::NotFound { what: "controller" }.into());
        };
        entry.cancel.cancel();
        Ok(())
    }

    fn remove_controller(&self, id: Uuid) {
        let removed = {
            let mut controllers = self.controllers();
            let before = controllers.len();
            controllers.retain(|entry| entry.controller.id != id);
            controllers.len() != before
        };
        if removed {
            self.broadcast_controllers_updated();
        }
    }

    fn broadcast_controllers_updated(&self) {
        if let Some(node) = self.node.upgrade() {
            node.broadcaster.broadcast(&ControllersUpdated {
                controllers: self.attached_controllers(),
            });
        }
    }

    pub async fn edit_queue<F>(&self, expected_revision: Option<u64>, edit: F) -> Result<()>
    where
        F: FnOnce(&mut dyn Playable) -> bool + Send,
    {
        let mut state = self.state.lock().await;
        if let Some(expected) = expected_revision
            && expected != state.queue_revision
        {
            return Err(ExpectedRequestError::StaleQueueRevision {
                expected,
                current: state.queue_revision,
            }
            .into());
        }
        let mut player = self.player.lock().await;
        if edit(&mut *player) {
            state.queue_revision = state
                .queue_revision
                .checked_add(1)
                .context("Queue revision overflow")?;
        }
        Ok(())
    }

    pub async fn session_id(&self) -> String {
        self.state.lock().await.session_id.to_string()
    }

    pub async fn queue_revision(&self) -> u64 {
        self.state.lock().await.queue_revision
    }

    pub(crate) async fn playback_snapshot(&self) -> (PlayerStatus, u64) {
        let state = self.state.lock().await;
        let status = self.player.lock().await.get_status();
        (status, state.queue_revision)
    }

    /// Re-send this session's current player bar and queue after renderer control ends.
    pub(crate) async fn reemit_current(&self) -> Result<()> {
        let Some(node) = self.node.upgrade() else {
            return Ok(());
        };
        let status = self.player.lock().await.get_status();
        let (session_id, revision) = {
            let state = self.state.lock().await;
            (state.session_id.to_string(), state.queue_revision)
        };
        let controls = self.controls.lock().await.clone();
        let mut item_state = ItemEventState::default();
        handle_status_event(
            &node,
            status.clone(),
            &mut item_state,
            &controls,
            None,
            &node.gui_broadcaster,
            &session_id,
        )
        .await;
        let playlist = PlaylistStatus {
            items: status.playlist,
            unavailable: status.unavailable_reasons,
        };
        send_playlist(
            &node,
            &playlist,
            node.gui_broadcaster.as_ref(),
            &session_id,
            revision,
        )
        .await;
        Ok(())
    }

    pub async fn queue_for_library(&self, library_id: Uuid) -> Vec<i32> {
        let items = self
            .player
            .lock()
            .await
            .get_status()
            .playlist
            .into_iter()
            .filter_map(|item| match item {
                PlayingItem::InLibrary(identity) if identity.library_id == library_id => {
                    Some(identity.file_uuid)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let Some(node) = self.node.upgrade() else {
            return Vec::new();
        };
        let Ok(library) = node.library_pool.acquire(library_id).await else {
            return Vec::new();
        };
        let Ok(ids) = file_ids_by_uuid(&library.main_db, &items).await else {
            return Vec::new();
        };
        items
            .iter()
            .filter_map(|uuid| ids.get(uuid).copied())
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
        let state = SessionState::read(&path, &node.library_pool).await?;
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
            attached_controllers: StdMutex::new(Vec::new()),
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
        let identities = file_uuids_by_id(&library.main_db, &ids).await?;
        let items = ids
            .into_iter()
            .filter_map(|id| {
                identities.get(&id).copied().map(|file_uuid| {
                    PlayingItem::InLibrary(playback::player::LibraryItem {
                        library_id: library.descriptor.library_id,
                        file_uuid,
                    })
                })
            })
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
        let session_id = self.state.lock().await.session_id.to_string();
        let event_state = self.state.clone();
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
                        handle_status_event(&node, value, &mut item_state, &controls, Some(&scrobbler), &broadcaster, &session_id).await;
                    }
                    Ok(value) = playlist.recv() => {
                        if persist_playlist_tx.send(value.clone()).await.is_err() { return; }
                        if let Some(node) = weak.upgrade() {
                            let revision = event_state.lock().await.queue_revision;
                            handle_playlist_event(&node, value, broadcaster.as_ref(), &session_id, revision).await;
                        }
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
    /// `last_item` as sent to Dart, valid for `request_ui_library`.
    request: Option<PlayingItemRequest>,
    request_ui_library: Option<Uuid>,
}

struct ResolvedLibrary {
    fsio: Arc<FsIo>,
    media_root: String,
}

/// Turns a player status into the GUI report. `scrobbler` is `None` when only
/// re-showing the current state (e.g. after renderer control ends), so services
/// never see a "now playing" for a track that did not start.
async fn handle_status_event(
    node: &NodeContext,
    value: PlayerStatus,
    item_state: &mut ItemEventState,
    controls: &Option<Arc<Mutex<MediaControlManager>>>,
    scrobbler: Option<&Arc<Mutex<dyn ScrobblingServiceManager>>>,
    broadcaster: &Arc<dyn Broadcaster>,
    session_id: &str,
) {
    let item_changed = value.item != item_state.last_item;
    if item_changed {
        item_state.metadata = PlayingItemMetadataSummary::default();
        item_state.cover = None;
        item_state.library = None;
        if let Some(item) = &value.item {
            let result = match item {
                PlayingItem::InLibrary(identity) => {
                    match node.library_pool.acquire(identity.library_id).await {
                        Ok(library) => {
                            let result = item_metadata_from_library(item, true, &library).await;
                            item_state.library = Some(ResolvedLibrary {
                                fsio: library.fsio.clone(),
                                media_root: library.media_root.clone(),
                            });
                            result
                        }
                        Err(error) => Err(error),
                    }
                }
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
                if let Some(scrobbler) = scrobbler {
                    scrobbler.lock().await.update_now_playing_all(
                        metadata_summary_to_scrobbling_track(&item_state.metadata),
                    );
                }
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
    let ui_library_id = current_ui_library_id(node).await;
    if item_changed || ui_library_id != item_state.request_ui_library {
        item_state.request_ui_library = ui_library_id;
        item_state.request = match &value.item {
            Some(item) => playing_item_requests(from_ref(item), &node.library_pool, ui_library_id)
                .await
                .pop(),
            None => None,
        };
    }
    let item_request = item_state.request.clone();
    let report = format_status(
        &value,
        &item_state.metadata,
        item_state.cover.clone(),
        root,
        item_request,
        session_id,
    );
    if let Some(manager) = controls {
        let _ = update_media_controls_progress(manager.clone(), &report).await;
    }
    broadcaster.broadcast(&report);
}

async fn handle_playlist_event(
    node: &NodeContext,
    value: PlaylistStatus,
    broadcaster: &dyn Broadcaster,
    session_id: &str,
    revision: u64,
) {
    send_playlist(node, &value, broadcaster, session_id, revision).await;
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
        if let Ok(library_id) = library_id.parse() {
            let domain = format!("scrobbler::{:?}::{:?}", error.action, error.service);
            let detail = format!("{error:#?}");
            let _ = library_log(&node, library_id, domain, detail).await;
        } else {
            log::warn!("Scrobbler error with invalid library context: {error:#?}");
        }
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
    if let PlayingItem::InLibrary(identity) = item {
        let library = node.library_pool.acquire(identity.library_id).await?;
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
    if let PlayingItem::InLibrary(identity) = item {
        let library = node.library_pool.acquire(identity.library_id).await?;
        let ids = file_ids_by_uuid(&library.main_db, &[identity.file_uuid]).await?;
        let id = ids
            .get(&identity.file_uuid)
            .context("Playing track no longer exists in its library")?;
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
    if let PlayingItem::InLibrary(identity) = item {
        library_log(node, identity.library_id, domain, detail).await?;
    }
    Ok(())
}

async fn library_log(
    node: &NodeContext,
    library_id: Uuid,
    domain: String,
    detail: String,
) -> Result<()> {
    let library = node.library_pool.acquire(library_id).await?;
    let tx = library.main_db.begin().await?;
    insert_log(&tx, LogLevel::Error, domain, detail).await?;
    tx.commit().await?;
    Ok(())
}

async fn send_playlist(
    node: &NodeContext,
    playlist: &PlaylistStatus,
    broadcaster: &dyn Broadcaster,
    session_id: &str,
    revision: u64,
) {
    let mut library_items = HashMap::<Uuid, Vec<PlayingItem>>::new();
    let mut independent_items = Vec::new();
    for item in &playlist.items {
        match item {
            PlayingItem::InLibrary(identity) => {
                library_items
                    .entry(identity.library_id)
                    .or_default()
                    .push(item.clone());
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

    let ui_library_id = current_ui_library_id(node).await;
    let item_requests =
        playing_item_requests(&playlist.items, &node.library_pool, ui_library_id).await;
    let items = playlist
        .items
        .iter()
        .zip(item_requests)
        .map(|(item, item_request)| {
            let summary = metadata.get(item);
            let unavailable = playlist
                .unavailable
                .get(item)
                .copied()
                .or_else(|| summary.is_none().then_some(UnavailableReason::Unreadable))
                .map(PlaylistUnavailableReason::from);
            let metadata = summary.cloned().unwrap_or_default();
            PlaylistItem {
                item: item_request,
                artist: metadata.artist,
                album: metadata.album,
                title: metadata.title,
                duration: metadata.duration,
                unavailable,
            }
        })
        .collect();
    broadcaster.broadcast(&PlaylistUpdate {
        items,
        session_id: session_id.to_string(),
        revision,
    });
}

fn format_status(
    status: &PlayerStatus,
    metadata: &PlayingItemMetadataSummary,
    cover_art_path: Option<String>,
    lib_path: String,
    item_request: Option<PlayingItemRequest>,
    session_id: &str,
) -> PlaybackStatus {
    PlaybackStatus {
        session_id: session_id.to_string(),
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
        item: status.item.clone().map(|item| item.to_string()),
        item_request,
        playback_mode: status.playback_mode.into(),
        ready: status.ready,
        cover_art_path,
        lib_path,
    }
}

async fn current_ui_library_id(node: &NodeContext) -> Option<Uuid> {
    let params = node.current_ui().await.ok()?;
    params
        .ui_library
        .lock()
        .await
        .as_ref()
        .map(|library| library.descriptor.library_id)
}

impl Drop for PlayerSession {
    fn drop(&mut self) {
        self.cancel.cancel();
        for task in self.tasks.get_mut().drain(..) {
            task.abort();
        }
    }
}
