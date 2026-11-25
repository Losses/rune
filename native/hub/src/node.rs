mod reconnect;
use anyhow::Error as AnyhowError;
pub(crate) use reconnect::ReconnectConfig;
use reconnect::ReconnectTask;

use std::{
    collections::{HashMap, HashSet},
    fmt::{Debug, Formatter, Result as FmtResult},
    fs,
    path::Path,
    sync::{
        Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};
#[cfg(unix)]
use std::{io::Error, mem::MaybeUninit};
use uuid::Uuid;

use anyhow::{Context, Result, ensure};
use database::credentials::CredentialStore;
use database::error::ExpectedError;
use discovery::{
    client::CertValidator,
    protocol::DiscoveryService,
    server::{PermissionManager, UserStatus},
};
use playback::{
    PlayerCommand, hosted::is_hosted_path, player::HandoffBlockId as PlayerHandoffBlockId,
    sfx_player::SfxPlayer,
};
use rinf::DartSignal;
use tokio::{
    sync::{Mutex, RwLock, RwLockReadGuard, broadcast},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::access_policy::{Action, Denial, RuneAccessPolicy};
use crate::backends::remote::{RemoteConnectError, RendererLink, RendererLinkUse};
use crate::macros::{
    RequestAccess, RequestScope, StandaloneRequestReceiver, answer_node_request,
    capability_snapshot, request_access, request_scope,
};
use crate::request_error::{ExpectedRequestError, player_command_error};
use crate::request_owner::{CurrentPlayer, Implementation, OpenLibrary};
use crate::rpc::{Slot, TargetRequest};
use crate::server::sync_worker::SyncTokenStore;
use crate::service::{ServiceBinding, covers::ServiceCoverCache};
use crate::session::PlayerWrap;
use crate::visualizer::NodeVisualizer;
use crate::{Session, session::PlayerSession};
use crate::{
    library_pool::{LibraryHandle, LibraryPool},
    messages::{
        AttachedRenderer, CapabilityReason, ClientStatus, ClientSummary,
        IncommingClientPermissionNotification, InitializeNodeRequest, InitializeNodeResponse,
        LibraryTaskKind, LibraryTaskStateChanged, PauseRequest, PlaybackStatus, PlayerHostChanged,
        PlaylistItem, ProtocolUpdateSide, RefreshSessionRequest, RemoteLibraryReconnecting,
        RemoteLossKind, RendererAttachmentChanged, RendererLinkLost, RequestCapabilities,
        RequestCapability, TrustListUpdated, TrustedClientsChanged, TrustedServerCertificate,
        VisualizerInputsChanged,
    },
    server::{
        ServerManager, WebSocketService,
        utils::device::{RunMode, load_device_info},
    },
    utils::{
        Broadcaster, GlobalParams, LibraryTaskOwner, LocalGuiBroadcaster, RemoteEventScope,
        RinfRustSignal, nid::get_or_create_node_id, sync_settings::SyncSettings,
    },
};
use ::visualizer::{Visualizer, VisualizerInputs};
use database::library_registry::LibraryRecordId;
use discovery::DiscoveryParams;
use discovery::access_policy::{AccessPolicy, PeerIdentity};
use discovery::endpoint::DEFAULT_DEVICE_PORT;
use discovery::protocol_version::{PROTOCOL_VERSION, ProtocolVersion};
use discovery::utils::DeviceInfo;

static NODE: NodeSlot = NodeSlot::new();

fn renderer_connection_error(error: RemoteConnectError) -> anyhow::Error {
    let detail = format!("Renderer connection failed: {error:?}");
    match error {
        RemoteConnectError::UntrustedEndpoint | RemoteConnectError::NotAuthorized => {
            AnyhowError::new(Denial::Untrusted).context(detail)
        }
        RemoteConnectError::Unreachable | RemoteConnectError::Protocol(_) => {
            AnyhowError::new(ExpectedRequestError::Unavailable { what: "renderer" }).context(detail)
        }
        RemoteConnectError::PeerNeedsUpdate {
            local,
            peer,
            update_peer,
            peer_alias,
        } => AnyhowError::new(ExpectedRequestError::PeerNeedsUpdate {
            update_side: if update_peer {
                ProtocolUpdateSide::Peer
            } else {
                ProtocolUpdateSide::ThisDevice
            },
            peer_alias,
            local_version: local,
            peer_version: peer,
        })
        .context(detail),
    }
}

enum UiLibrary {
    Closed,
    Open(OpenLibraryState),
}

struct OpenLibraryState {
    record_id: Option<LibraryRecordId>,
    generation: u64,
    library_id: Option<Uuid>,
    catalog: LibraryBinding,
}

enum LibraryBinding {
    Local,
    Service(Arc<dyn ServiceBinding>),
    RemotePeer {
        fingerprint: String,
        reconnect: Option<ReconnectTask>,
    },
}

/// The local player's time as the UI's player. Requests routed to the local player run inside
/// it, and ending it waits until they have stopped, so no command one of them sends can follow
/// the pause that comes after the end. The ended tenure stays installed, refusing requests,
/// until that pause is queued, and only then does a new tenure admit them, so the pause never
/// stops a command sent by a request the new tenure admits.
///
/// A request runs in the tenure only when it is session-scoped and runs on the local player,
/// and no such request changes the current player. Player changes come from node-scoped
/// requests, requests sent to another player and background tasks, and none of these holds the
/// tenure, so ending the tenure never waits for the request that ends it.
#[derive(Default)]
pub(crate) struct LocalPlayerTenure {
    ended: CancellationToken,
    running: RwLock<()>,
}

impl LocalPlayerTenure {
    /// Enters the tenure for one request, or fails if it has ended.
    pub(crate) async fn enter(&self) -> Result<RwLockReadGuard<'_, ()>> {
        let running = self.running.read().await;
        if self.ended.is_cancelled() {
            return Err(ExpectedRequestError::PlayerChanged.into());
        }
        Ok(running)
    }

    /// Resolves once the tenure has ended.
    pub(crate) async fn ended(&self) {
        self.ended.cancelled().await;
    }

    /// Ends the tenure: a request arriving from now on is refused, and those running in it are
    /// cancelled.
    fn close(&self) {
        self.ended.cancel();
    }

    /// Ends the tenure and waits until no request is still running in it.
    async fn end(&self) {
        self.close();
        drop(self.running.write().await);
    }
}

fn lock_tenures(
    tenures: &StdMutex<Arc<LocalPlayerTenure>>,
) -> Result<StdMutexGuard<'_, Arc<LocalPlayerTenure>>> {
    tenures
        .lock()
        .map_err(|_| anyhow::anyhow!("Local player tenure lock poisoned"))
}

/// Waits until `ending`, the tenure the local player is leaving, has no request running, pauses
/// the local player after every command its requests sent, and only then admits requests to a
/// new tenure. A request arriving while this runs, if the local player is current again by then,
/// is refused, so nothing it sends can land before this pause.
async fn depart_local_player(
    tenures: Arc<StdMutex<Arc<LocalPlayerTenure>>>,
    ending: Arc<LocalPlayerTenure>,
    session: Arc<PlayerSession>,
) -> Result<()> {
    ending.end().await;
    let completion = {
        let mut player = session.player.lock().await;
        let mut tenure = lock_tenures(&tenures)?;
        // Every departure that ends this tenure waits for it, and the first one through pauses
        // after its requests and admits new ones. Another pause could follow a command a request
        // in the new tenure has sent since, so it is not sent.
        if !Arc::ptr_eq(&tenure, &ending) {
            return Ok(());
        }
        // The pause follows the commands already sent, so it also stops a Play or a track loading
        // to play that the cached status does not show as playing yet.
        let completion = player.command_with_completion(PlayerCommand::Pause);
        *tenure = Arc::default();
        completion
    };
    completion
        .await
        .map_err(|error| AnyhowError::new(player_command_error(error)))
        .context("Pause local player when another player becomes current")
}

/// Process services shared by every local library and remote controller.
pub struct NodeContext {
    protocol_version: StdMutex<ProtocolVersion>,
    pub config_path: Arc<String>,
    pub run_mode: RunMode,
    pub node_id: Arc<String>,
    pub library_pool: Arc<LibraryPool>,
    pub credentials: Arc<CredentialStore>,
    /// Orders the final credential check and binding installation with re-login's
    /// store and client update. Subsonic HTTP probes run outside this lock.
    pub(crate) subsonic_credential_commit: Mutex<()>,
    pub access_policy: Arc<RuneAccessPolicy>,
    pub permission_manager: Arc<RwLock<PermissionManager>>,
    pub cert_validator: Arc<RwLock<CertValidator>>,
    pub device_scanner: Arc<DiscoveryService>,
    pub sync_settings: Arc<RwLock<SyncSettings>>,
    pub broadcaster: Arc<dyn Broadcaster>,
    pub websocket_service: Arc<WebSocketService>,
    pub gui_broadcaster: Arc<dyn Broadcaster>,
    handoff_playback_events: broadcast::Sender<HandoffPlaybackEvent>,
    pub renderer_link: Arc<StdMutex<Option<Arc<RendererLink>>>>,
    ui_library: Arc<StdMutex<UiLibrary>>,
    current_player: Arc<StdMutex<CurrentPlayer>>,
    /// Ends when another player takes over from the local player, which cancels the UI
    /// requests to the local player that are still running. The ended tenure stays here until
    /// the local player's departure has queued its pause.
    local_player_tenure: Arc<StdMutex<Arc<LocalPlayerTenure>>>,
    /// How many `HiddenLocalSession`s are alive. The session's events stay out of the UI while
    /// any is.
    local_session_hidden: Arc<AtomicUsize>,
    /// Whether a platform host plays the current track of this device's own player, so that
    /// it gives the visualizer nothing. Changed only under the `current_player` lock, which
    /// orders the inputs published.
    local_track_hosted: AtomicBool,
    /// Whether this device's own player has a track at all. The volume control goes back to
    /// Rune's own volume without one, though `local_track_hosted` keeps what the last track said.
    local_track_loaded: AtomicBool,
    /// What the UI here was last told about `local_track_hosted` for the current player. Changed
    /// only under the `current_player` lock.
    reported_hosted: AtomicBool,
    /// What Dart was last told the current source gives the visualizer. Changed only under the
    /// `current_player` lock, like the inputs themselves: that lock is taken first, this one
    /// after it.
    published_visualizer_inputs: StdMutex<Option<VisualizerInputs>>,
    open_generation: AtomicU64,
    reconnect_config: ReconnectConfig,
    visualizer: NodeVisualizer,
    pub token: Arc<CancellationToken>,
    pub(crate) sync_tokens: SyncTokenStore,
    pub library_tasks: Arc<StdMutex<LibraryTaskOwner>>,
    sync_library_tokens: Mutex<HashMap<Uuid, CancellationToken>>,
    pub server_manager: OnceLock<Arc<ServerManager>>,
    local_session: OnceLock<Arc<PlayerSession>>,
    current_ui: Mutex<Option<Weak<GlobalParams>>>,
    last_capabilities: StdMutex<Option<RequestCapabilities>>,
    event_handles: Mutex<Vec<JoinHandle<()>>>,
    handoff_generation: AtomicU64,
    handoff_gate: Mutex<()>,
    handoff_blocks: Mutex<HashMap<String, PlayerHandoffBlockId>>,
    /// Weak references to service cover caches for cleanup when removing libraries.
    cover_caches: Arc<StdMutex<HashMap<Uuid, Vec<Weak<ServiceCoverCache>>>>>,
    /// Libraries currently being removed. A binding created during removal must not
    /// register a cover cache that can write after the library is deleted.
    removing_libraries: Arc<StdMutex<HashSet<Uuid>>>,
}

impl Debug for NodeContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("NodeContext")
            .field("config_path", &self.config_path)
            .finish()
    }
}

struct NodeBroadcaster {
    current_player: Arc<StdMutex<CurrentPlayer>>,
    local_session_hidden: Arc<AtomicUsize>,
    local: Arc<dyn Broadcaster>,
    remote: Arc<WebSocketService>,
}

/// Keeps this device's session out of the UI for as long as it lives. The session's event loop
/// drops it once it has caught up with its player. A switch that fails drops it at once, and so
/// does a request to show the session that no event loop is left to take.
pub(crate) struct HiddenLocalSession(Arc<AtomicUsize>);

impl HiddenLocalSession {
    fn new(hidden: &Arc<AtomicUsize>) -> Self {
        hidden.fetch_add(1, Ordering::AcqRel);
        Self(Arc::clone(hidden))
    }
}

impl Drop for HiddenLocalSession {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// A report a renderer sent over one link, for a handoff that waits for that renderer to start
/// the handed-off track. Only renderer links publish them: this device's own session reports the
/// same track at the same place while it is handed off, and must never stand in for the target.
#[derive(Clone)]
pub(crate) struct HandoffPlaybackEvent {
    pub(crate) link: Uuid,
    pub(crate) report: HandoffPlaybackReport,
}

#[derive(Clone)]
pub(crate) enum HandoffPlaybackReport {
    Status(Box<PlaybackStatus>),
    Playlist(Vec<PlaylistItem>),
}

impl Broadcaster for NodeBroadcaster {
    fn broadcast(&self, message: &dyn RinfRustSignal) {
        let remote_event = message.remote_event();
        let suppress_local_status = (self
            .current_player
            .lock()
            .is_ok_and(|player| player.suppresses_local_session_events())
            || self.local_session_hidden.load(Ordering::Acquire) > 0)
            && remote_event == Some(RemoteEventScope::Session);
        if !suppress_local_status {
            self.local.broadcast(message);
        }
        if self.remote.broadcast_tx.receiver_count() > 0 && remote_event.is_some() {
            self.remote.broadcast(message);
        }
    }
}

impl NodeContext {
    pub(crate) fn subscribe_handoff_playback_events(
        &self,
    ) -> broadcast::Receiver<HandoffPlaybackEvent> {
        self.handoff_playback_events.subscribe()
    }

    pub(crate) fn publish_handoff_playback_event(&self, event: HandoffPlaybackEvent) {
        let _ = self.handoff_playback_events.send(event);
    }

    pub(crate) async fn sync_library_token(&self, library_id: Uuid) -> CancellationToken {
        let mut tokens = self.sync_library_tokens.lock().await;
        tokens
            .entry(library_id)
            .or_insert_with(CancellationToken::new)
            .clone()
    }

    pub(crate) async fn stop_library_sync(&self, library_id: Uuid) {
        if let Some(token) = self.sync_library_tokens.lock().await.remove(&library_id) {
            token.cancel();
        }
    }

    pub(crate) fn protocol_version(&self) -> ProtocolVersion {
        *self
            .protocol_version
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    #[cfg(test)]
    pub(crate) fn set_protocol_version_for_test(&self, version: ProtocolVersion) {
        *self
            .protocol_version
            .lock()
            .expect("protocol version lock poisoned") = version;
    }

    /// Send the request capability snapshot to Dart if it changed. It reads the UI
    /// library and renderer link locks, so never call it while holding either.
    pub(crate) fn publish_capabilities(&self) -> Result<()> {
        let capabilities = capability_snapshot(self)?;
        let mut last = self
            .last_capabilities
            .lock()
            .map_err(|_| anyhow::anyhow!("Capability snapshot lock poisoned"))?;
        if last.as_ref() == Some(&capabilities) {
            return Ok(());
        }
        self.broadcaster.broadcast(&capabilities);
        *last = Some(capabilities);
        Ok(())
    }

    /// `os_media_controls` registers the local session with the OS (MPRIS, SMTC).
    /// Only the app and the headless server enable it; tests must not touch the
    /// desktop session.
    ///
    /// `visualizer` is the one every writer of this node's visualizer state uses: the app passes
    /// its own, and every other node gets an instance of its own.
    pub(crate) async fn new(
        config_path: &str,
        db_path: &str,
        broadcaster: Arc<dyn Broadcaster>,
        os_media_controls: bool,
        run_mode: RunMode,
        visualizer: Arc<Visualizer>,
    ) -> Result<Arc<Self>> {
        Self::with_reconnect_config(
            config_path,
            db_path,
            broadcaster,
            os_media_controls,
            run_mode,
            ReconnectConfig::default(),
            PlayerSession::real_player(),
            visualizer,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn with_reconnect_config(
        config_path: &str,
        db_path: &str,
        broadcaster: Arc<dyn Broadcaster>,
        os_media_controls: bool,
        run_mode: RunMode,
        reconnect_config: ReconnectConfig,
        wrap_player: PlayerWrap,
        visualizer: Arc<Visualizer>,
    ) -> Result<Arc<Self>> {
        #[cfg(unix)]
        raise_nofile_soft_limit();
        fs::create_dir_all(config_path).context("Create node configuration directory")?;
        let node_id = Arc::new(get_or_create_node_id(config_path).await?.to_string());
        let websocket_service = Arc::new(WebSocketService::new());
        let renderer_link = Arc::new(StdMutex::new(None));
        let ui_library = Arc::new(StdMutex::new(UiLibrary::Closed));
        let current_player = Arc::new(StdMutex::new(CurrentPlayer::Local));
        let local_session_hidden = Arc::new(AtomicUsize::new(0));
        let sync_tokens = SyncTokenStore::default();
        let credentials = Arc::new(CredentialStore::open(Path::new(config_path)));
        let permission_manager = PermissionManager::new(config_path)?;
        let access_policy = Arc::new(RuneAccessPolicy::load(
            Path::new(config_path),
            &permission_manager.list_users().await,
        )?);
        let (handoff_playback_events, _) = broadcast::channel(128);
        let node = Arc::new(Self {
            protocol_version: StdMutex::new(PROTOCOL_VERSION),
            config_path: Arc::new(config_path.to_owned()),
            run_mode,
            library_pool: LibraryPool::new_with_credentials(
                Path::new(config_path),
                Path::new(db_path),
                &node_id,
                credentials.clone(),
            )?,
            credentials,
            subsonic_credential_commit: Mutex::new(()),
            node_id,
            permission_manager: Arc::new(RwLock::new(permission_manager)),
            access_policy,
            cert_validator: Arc::new(RwLock::new(CertValidator::new(config_path).await?)),
            device_scanner: Arc::new(DiscoveryService::without_store()),
            sync_settings: Arc::new(RwLock::new(SyncSettings::load(Path::new(config_path)))),
            broadcaster: Arc::new(NodeBroadcaster {
                current_player: current_player.clone(),
                local_session_hidden: local_session_hidden.clone(),
                local: broadcaster.clone(),
                remote: websocket_service.clone(),
            }),
            gui_broadcaster: broadcaster,
            handoff_playback_events,
            websocket_service,
            renderer_link,
            ui_library,
            current_player,
            local_player_tenure: Arc::default(),
            local_session_hidden,
            local_track_hosted: AtomicBool::new(false),
            local_track_loaded: AtomicBool::new(false),
            reported_hosted: AtomicBool::new(false),
            published_visualizer_inputs: StdMutex::new(None),
            open_generation: AtomicU64::new(0),
            reconnect_config,
            visualizer: NodeVisualizer::new(visualizer),
            token: Arc::new(CancellationToken::new()),
            sync_tokens,
            library_tasks: Arc::new(StdMutex::new(LibraryTaskOwner::default())),
            sync_library_tokens: Mutex::new(HashMap::new()),
            server_manager: OnceLock::new(),
            local_session: OnceLock::new(),
            current_ui: Mutex::new(None),
            last_capabilities: StdMutex::new(None),
            event_handles: Mutex::new(Vec::new()),
            handoff_generation: AtomicU64::new(0),
            handoff_gate: Mutex::new(()),
            handoff_blocks: Mutex::new(HashMap::new()),
            cover_caches: Arc::new(StdMutex::new(HashMap::new())),
            removing_libraries: Arc::new(StdMutex::new(HashSet::new())),
        });
        let policy = node.access_policy.clone();
        let revoked_tokens = node.sync_tokens.clone();
        let weak_node = Arc::downgrade(&node);
        node.permission_manager
            .write()
            .await
            .set_trust_change_hook(Arc::new(move |fingerprint, status| {
                if status != Some(&UserStatus::Approved) {
                    revoked_tokens.remove_peer(fingerprint);
                    let weak_node = weak_node.clone();
                    let fingerprint = fingerprint.to_owned();
                    tokio::spawn(async move {
                        if let Some(node) = weak_node.upgrade() {
                            node.revoke_outgoing_for(&fingerprint).await;
                        }
                    });
                }
                policy
                    .trust_changed(fingerprint, status)
                    .map_err(|error| format!("{error:#}"))
            }));
        let trust_listener = node
            .permission_manager
            .read()
            .await
            .watch_external_changes(node.token.child_token())
            .await;
        node.event_handles.lock().await.push(trust_listener);
        let manager = Arc::new(ServerManager::new(node.clone()).await?);
        node.server_manager
            .set(manager.clone())
            .map_err(|_| anyhow::anyhow!("Node server already initialized"))?;
        node.local_session
            .set(PlayerSession::new(&node, "local", os_media_controls, wrap_player).await?)
            .map_err(|_| anyhow::anyhow!("Local session already initialized"))?;
        node.refresh_current_player(node.current_player()?)?;
        node.start_notifications().await;
        manager.start_worker().await?;
        if node.sync_settings.read().await.listen {
            let result = async {
                let device_info = node.device_info().await?;
                manager
                    .start_sync(
                        ([0, 0, 0, 0], DEFAULT_DEVICE_PORT).into(),
                        DiscoveryParams { device_info },
                    )
                    .await
            }
            .await;
            if let Err(error) = result {
                log::warn!("Could not start paired sync server: {error:#}");
            }
        }
        node.publish_capabilities()?;
        Ok(node)
    }

    /// What this device tells other devices about itself.
    pub async fn device_info(&self) -> Result<DeviceInfo> {
        load_device_info(Path::new(&*self.config_path), self.run_mode).await
    }

    async fn start_notifications(&self) {
        let mut certificates = self.cert_validator.read().await.subscribe_changes();
        let mut permissions = self.permission_manager.read().await.subscribe_new_user();
        let mut changes = self.permission_manager.read().await.changes();
        let broadcaster = self.broadcaster.clone();
        let cancel = self.token.child_token();
        let certificate_task = tokio::spawn(async move {
            loop {
                let report = tokio::select! {
                    _ = cancel.cancelled() => return,
                    report = certificates.recv() => report,
                };
                let Ok(report) = report else {
                    return;
                };
                broadcaster.broadcast(&TrustListUpdated {
                    certificates: report
                        .entries
                        .into_iter()
                        .map(|(fingerprint, hosts)| TrustedServerCertificate { fingerprint, hosts })
                        .collect(),
                });
            }
        });
        let broadcaster = self.broadcaster.clone();
        let cancel = self.token.child_token();
        let permission_task = tokio::spawn(async move {
            loop {
                let user = tokio::select! {
                    _ = cancel.cancelled() => return,
                    user = permissions.recv() => user,
                };
                let Ok(user) = user else {
                    return;
                };
                broadcaster.broadcast(&IncommingClientPermissionNotification {
                    user: ClientSummary {
                        alias: user.alias,
                        fingerprint: user.fingerprint,
                        device_model: user.device_model,
                        status: match user.status {
                            UserStatus::Approved => ClientStatus::Approved,
                            UserStatus::Pending => ClientStatus::Pending,
                            UserStatus::Blocked => ClientStatus::Blocked,
                        },
                    },
                });
            }
        });
        let broadcaster = self.broadcaster.clone();
        let cancel = self.token.child_token();
        let changes_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    result = changes.changed() => {
                        if result.is_err() { return; }
                        broadcaster.broadcast(&TrustedClientsChanged {});
                    }
                }
            }
        });
        self.event_handles
            .lock()
            .await
            .extend([certificate_task, permission_task, changes_task]);
    }

    /// The default local renderer, independent of the UI library.
    pub fn session(&self) -> Result<Arc<PlayerSession>> {
        self.local_session
            .get()
            .cloned()
            .context("Node session is not initialized")
    }

    /// What the item this device plays supports may have changed without a status from the
    /// player: the UI opened or closed a library, or a library record went away. The UI hears it
    /// again.
    pub(crate) fn item_capabilities_changed(&self) {
        if let Ok(session) = self.session() {
            session.item_capabilities_changed();
        }
    }

    /// Starts a handoff. Any pause retry left by an earlier handoff stops at its
    /// next attempt, so it never pauses a source this handoff starts again.
    pub(crate) async fn begin_handoff(&self) -> u64 {
        let _guard = self.handoff_gate.lock().await;
        self.handoff_generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub(crate) async fn remember_handoff_block(
        &self,
        fingerprint: String,
        block: PlayerHandoffBlockId,
    ) {
        self.handoff_blocks.lock().await.insert(fingerprint, block);
    }

    pub(crate) async fn take_handoff_block(
        &self,
        fingerprint: &str,
    ) -> Option<PlayerHandoffBlockId> {
        self.handoff_blocks.lock().await.remove(fingerprint)
    }

    pub(crate) fn handoff_generation(&self) -> u64 {
        self.handoff_generation.load(Ordering::Acquire)
    }

    pub(crate) async fn pause_handoff_source(
        self: &Arc<Self>,
        fingerprint: &str,
        source: &RendererLink,
        generation: u64,
    ) -> Result<bool> {
        let _guard = self.handoff_gate.lock().await;
        if self.handoff_generation() != generation {
            return Ok(false);
        }

        match source.call(&PauseRequest { call_id: None }).await {
            Ok(()) => Ok(true),
            Err(first_error) => {
                let (retry_link, _) = self
                    .connect_renderer(fingerprint, RendererLinkUse::Cast)
                    .await
                    .with_context(|| format!("Pause source renderer {fingerprint}"))?;
                let result = retry_link.call(&PauseRequest { call_id: None }).await;
                retry_link.close();
                result
                    .context("Pause source renderer after reconnect")
                    .map(|()| true)
                    .or(Err(first_error))
            }
        }
    }

    pub async fn session_queue_for_library(&self, library_id: Uuid) -> Result<Vec<i32>> {
        self.session()?.queue_for_library(library_id).await
    }

    pub async fn attach_renderer(self: &Arc<Self>, fingerprint: &str) -> Result<()> {
        if let Some(link) = self.renderer_link()?
            && link.fingerprint() == fingerprint
        {
            self.gui_broadcaster.broadcast(&RendererAttachmentChanged {
                renderer: Some(AttachedRenderer {
                    fingerprint: fingerprint.to_owned(),
                    alias: link.alias().to_owned(),
                }),
            });
            self.publish_capabilities()?;
            return Ok(());
        }
        let (link, alias) = self
            .connect_renderer(fingerprint, RendererLinkUse::Control)
            .await?;
        if let Some(previous) = self.renderer_link()?
            && let Err(error) = previous.call(&PauseRequest { call_id: None }).await
        {
            link.close();
            return Err(error).context("Pause previously attached renderer");
        }
        let previous_player = self.current_player_snapshot()?;
        self.install_renderer_link(link, alias)?;
        self.transition_current_player(previous_player, self.current_player()?)
            .await?;
        self.publish_capabilities()?;
        if let Some(link) = self.renderer_link()?
            && let Err(error) = link.call(&RefreshSessionRequest { call_id: None }).await
        {
            log::warn!("Renderer attached, but its state refresh failed: {error:#}");
        }
        Ok(())
    }

    /// Make `link` the attached renderer, closing the previous one, and tell the UI.
    /// Reconnect calls this under the UI library lock, so it must not publish
    /// capabilities; callers that change the target do that themselves.
    pub(crate) fn install_renderer_link(&self, link: RendererLink, alias: String) -> Result<()> {
        let fingerprint = link.fingerprint().to_owned();
        let old = {
            let mut slot = self
                .renderer_link
                .lock()
                .map_err(|_| anyhow::anyhow!("Renderer link lock poisoned"))?;
            slot.replace(Arc::new(link))
        };
        if let Some(old) = old {
            old.close();
        }
        self.gui_broadcaster.broadcast(&RendererAttachmentChanged {
            renderer: Some(AttachedRenderer { fingerprint, alias }),
        });
        Ok(())
    }

    /// Connect to a trusted renderer using the same error mapping as attachment.
    pub(crate) async fn connect_renderer(
        self: &Arc<Self>,
        fingerprint: &str,
        link_use: RendererLinkUse,
    ) -> Result<(RendererLink, String)> {
        if !self
            .cert_validator
            .read()
            .await
            .clone_trusted_entries()
            .await
            .contains_key(fingerprint)
        {
            return Err(AnyhowError::new(ExpectedError::NotFound {
                what: "renderer",
            }));
        }
        let (link, _host, alias) = RendererLink::connect(self.clone(), fingerprint, link_use)
            .await
            .map_err(renderer_connection_error)?;
        Ok((link, alias))
    }

    pub async fn detach_renderer(&self) -> Result<()> {
        let previous_player = self.current_player_snapshot()?;
        let detached = {
            let mut slot = self
                .renderer_link
                .lock()
                .map_err(|_| anyhow::anyhow!("Renderer link lock poisoned"))?;
            if let Some(link) = slot.take() {
                link.close();
                true
            } else {
                false
            }
        };
        if detached {
            self.gui_broadcaster
                .broadcast(&RendererAttachmentChanged { renderer: None });
            self.publish_capabilities()?;
        }
        self.transition_current_player(previous_player, self.current_player()?)
            .await?;
        Ok(())
    }

    pub(crate) async fn renderer_link_lost(self: &Arc<Self>, id: Uuid) {
        self.renderer_connection_lost(
            id,
            RemoteLossKind::NetworkLost,
            "Renderer connection ended".into(),
        )
        .await;
    }

    pub(crate) async fn renderer_connection_lost(
        self: &Arc<Self>,
        id: Uuid,
        kind: RemoteLossKind,
        detail: String,
    ) {
        let previous_player = match self.current_player_snapshot() {
            Ok(player) => player,
            Err(error) => {
                log::warn!("Could not read the current player after renderer loss: {error:#}");
                return;
            }
        };
        let removed = match self.renderer_link.lock() {
            Ok(mut slot) if slot.as_ref().is_some_and(|link| link.id() == id) => slot.take(),
            _ => None,
        };
        let Some(link) = removed else {
            return;
        };
        link.close();
        let reconnecting = if let Ok(mut ui) = self.ui_library.lock()
            && let UiLibrary::Open(OpenLibraryState {
                catalog:
                    LibraryBinding::RemotePeer {
                        fingerprint,
                        reconnect,
                    },
                ..
            }) = &mut *ui
            && fingerprint == link.fingerprint()
        {
            *reconnect = Some(ReconnectTask::start(
                Arc::downgrade(self),
                fingerprint.clone(),
                link.alias().to_owned(),
                self.token.child_token(),
                self.reconnect_config.clone(),
                (kind, detail),
            ));
            true
        } else {
            false
        };
        if reconnecting {
            match self.current_player() {
                Ok(current_player) => {
                    if let Err(error) = self
                        .transition_current_player(previous_player, current_player)
                        .await
                    {
                        log::warn!("Could not update the player after renderer loss: {error:#}");
                    }
                }
                Err(error) => {
                    log::warn!("Could not resolve the player after renderer loss: {error:#}");
                }
            }
            self.gui_broadcaster.broadcast(&RemoteLibraryReconnecting {
                fingerprint: link.fingerprint().to_owned(),
                alias: link.alias().to_owned(),
            });
            return;
        }
        self.gui_broadcaster.broadcast(&RendererLinkLost {
            fingerprint: link.fingerprint().to_owned(),
            alias: link.alias().to_owned(),
        });
        self.gui_broadcaster
            .broadcast(&RendererAttachmentChanged { renderer: None });
        match self.current_player() {
            Ok(current_player) => {
                if let Err(error) = self
                    .transition_current_player(previous_player, current_player)
                    .await
                {
                    log::warn!("Could not update the player after renderer loss: {error:#}");
                }
            }
            Err(error) => {
                log::warn!("Could not resolve the player after renderer loss: {error:#}");
            }
        }
        if let Err(error) = self.publish_capabilities() {
            log::warn!("Could not publish request capabilities after renderer loss: {error:#}");
        }
    }

    pub(crate) async fn revoke_outgoing_for(self: &Arc<Self>, fingerprint: &str) {
        let link = self
            .renderer_link()
            .ok()
            .flatten()
            .filter(|link| link.fingerprint() == fingerprint);
        if let Some(link) = link {
            link.close();
            self.renderer_connection_lost(
                link.id(),
                RemoteLossKind::Revoked,
                "Renderer trust was removed".into(),
            )
            .await;
        }
    }

    pub(crate) fn current_library(&self) -> Result<OpenLibrary> {
        let ui = self
            .ui_library
            .lock()
            .map_err(|_| anyhow::anyhow!("UI library lock poisoned"))?;
        Ok(match &*ui {
            UiLibrary::Open(OpenLibraryState {
                catalog: LibraryBinding::Local,
                ..
            }) => OpenLibrary::Local,
            UiLibrary::Open(OpenLibraryState {
                catalog: LibraryBinding::RemotePeer { reconnect, .. },
                ..
            }) => OpenLibrary::RunePeer {
                reconnecting: reconnect.as_ref().map(|task| task.alias.clone()),
                link: self.renderer_link()?,
            },
            UiLibrary::Open(OpenLibraryState {
                catalog: LibraryBinding::Service(binding),
                ..
            }) => OpenLibrary::Service(Arc::clone(binding)),
            UiLibrary::Closed => OpenLibrary::Closed,
        })
    }

    pub(crate) fn current_player(&self) -> Result<CurrentPlayer> {
        let renderer_link = self.renderer_link()?;
        let ui = self
            .ui_library
            .lock()
            .map_err(|_| anyhow::anyhow!("UI library lock poisoned"))?;
        Ok(match &*ui {
            UiLibrary::Open(OpenLibraryState {
                catalog: LibraryBinding::RemotePeer { reconnect, .. },
                ..
            }) => CurrentPlayer::RunePeer {
                link: renderer_link.clone(),
                reconnecting: reconnect.as_ref().map(|task| task.alias.clone()),
            },
            UiLibrary::Open(OpenLibraryState {
                catalog: LibraryBinding::Service(binding),
                ..
            }) => Arc::clone(binding)
                .own_player()
                .map_or(CurrentPlayer::Local, CurrentPlayer::ServicePlayer),
            _ => match renderer_link {
                Some(link) => CurrentPlayer::RunePeer {
                    link: Some(link),
                    reconnecting: None,
                },
                None => CurrentPlayer::Local,
            },
        })
    }

    pub(crate) fn request_slot<R: TargetRequest>(&self) -> Result<Slot> {
        Ok(R::slot(&self.implementation_for_scope::<R>()?.slots()))
    }

    pub(crate) fn capabilities_for_peer(
        &self,
        mut capabilities: RequestCapabilities,
        peer: &PeerIdentity,
        library_id: Option<Uuid>,
    ) -> RequestCapabilities {
        if !library_id
            .is_some_and(|library_id| self.access_policy.library_visible(peer, library_id))
        {
            capabilities.library_id = None;
            capabilities.record_id = None;
        }
        capabilities.for_each_capability_mut(|access, capability| {
            let allowed = match access {
                // Calls that only this device makes cannot be granted.
                RequestAccess::NodeLocal => {
                    *capability = RequestCapability::Disabled {
                        reason: CapabilityReason::OwnerOnly,
                    };
                    return;
                }
                RequestAccess::Control => self.access_policy.check(peer, &Action::Control).is_ok(),
                RequestAccess::ConnectionView => library_id.is_some_and(|library_id| {
                    self.access_policy
                        .check(peer, &Action::ConnectionView { library_id })
                        .is_ok()
                }),
                RequestAccess::ConnectionEdit => library_id.is_some_and(|library_id| {
                    self.access_policy
                        .check(peer, &Action::ConnectionEdit { library_id })
                        .is_ok()
                }),
                RequestAccess::Administer => {
                    self.access_policy.check(peer, &Action::Administer).is_ok()
                }
            };
            if !allowed {
                *capability = RequestCapability::Disabled {
                    reason: CapabilityReason::NotAllowed,
                };
            }
        });
        capabilities
    }

    pub(crate) fn implementation_for_scope<R: TargetRequest>(&self) -> Result<Implementation> {
        match R::SCOPE {
            RequestScope::Node => Ok(self.current_library()?.node_implementation()),
            RequestScope::Library => Ok(self.current_library()?.implementation()),
            RequestScope::Session => Ok(self.current_player()?.implementation()),
        }
    }

    pub(crate) fn current_player_snapshot(&self) -> Result<CurrentPlayer> {
        self.current_player
            .lock()
            .map(|player| player.clone())
            .map_err(|_| anyhow::anyhow!("Current player lock poisoned"))
    }

    /// The visualizer this node's session, feeds and current player write.
    pub(crate) fn visualizer(&self) -> &Arc<Visualizer> {
        self.visualizer.visualizer()
    }

    pub(crate) fn node_visualizer(&self) -> &NodeVisualizer {
        &self.visualizer
    }

    fn refresh_current_player(&self, player: CurrentPlayer) -> Result<()> {
        let mut current = self
            .current_player
            .lock()
            .map_err(|_| anyhow::anyhow!("Current player lock poisoned"))?;
        *current = player;
        self.publish_visualizer_inputs(&current)?;
        self.publish_player_host(&current);
        Ok(())
    }

    /// The current tenure of the local player as the UI's player.
    pub(crate) fn local_player_tenure(&self) -> Result<Arc<LocalPlayerTenure>> {
        lock_tenures(&self.local_player_tenure).map(|tenure| Arc::clone(&tenure))
    }

    /// Tells the UI here whether a platform host plays what the current player plays, when that
    /// changed. Only this device's own player can be hosted: with a renderer or a service's own
    /// player current, the host is out of the picture whatever the local player still holds.
    /// Called under the `current_player` lock, which orders what is told.
    fn publish_player_host(&self, current: &CurrentPlayer) {
        let hosted = matches!(current, CurrentPlayer::Local)
            && self.local_track_loaded.load(Ordering::Acquire)
            && self.local_track_hosted.load(Ordering::Acquire);
        if self.reported_hosted.swap(hosted, Ordering::AcqRel) != hosted {
            self.gui_broadcaster
                .broadcast(&PlayerHostChanged { hosted });
        }
    }

    /// Publishes the current player's visualizer inputs again, for when what that player
    /// can feed changed while it stayed the current player.
    fn republish_current_player_inputs(&self) -> Result<()> {
        let current = self
            .current_player
            .lock()
            .map_err(|_| anyhow::anyhow!("Current player lock poisoned"))?;
        self.publish_visualizer_inputs(&current)?;
        self.publish_player_host(&current);
        Ok(())
    }

    /// Gives the visualizer what `current` feeds it, and tells Dart when that changed, so the UI
    /// follows a source that stops or starts feeding the visualizer without asking. Call it
    /// under the `current_player` lock, which orders what is published. The message goes
    /// through the UI's own broadcaster, as the node's would take that lock again.
    fn publish_visualizer_inputs(&self, current: &CurrentPlayer) -> Result<()> {
        let inputs = self.visualizer_inputs(current);
        self.visualizer().set_inputs(inputs);
        let mut published = self
            .published_visualizer_inputs
            .lock()
            .map_err(|_| anyhow::anyhow!("Published visualizer inputs lock poisoned"))?;
        if *published != Some(inputs) {
            *published = Some(inputs);
            self.gui_broadcaster.broadcast(&VisualizerInputsChanged {
                inputs: inputs.into(),
            });
        }
        Ok(())
    }

    /// What `current` gives the visualizer. This device's own player gives nothing while a
    /// platform host plays its track.
    fn visualizer_inputs(&self, current: &CurrentPlayer) -> VisualizerInputs {
        match current {
            CurrentPlayer::Local if self.local_track_hosted.load(Ordering::Acquire) => {
                VisualizerInputs::None
            }
            current => current.visualizer_inputs(),
        }
    }

    /// Notes the current track of this device's own player: its path, and whether it has a track
    /// at all.
    ///
    /// The visualizer inputs are published again when the path changes whether a platform host
    /// plays the track. A report without a path, as while the player stops between two tracks,
    /// changes nothing there: the inputs do not come back between two hosted tracks, and stay
    /// away after a hosted track until a file plays.
    ///
    /// The volume control follows `has_track` as well: without a track it is Rune's own volume,
    /// whatever the last track was. A paused hosted track, restored at startup too, still counts
    /// as hosted, because the next play goes to the host.
    pub(crate) fn note_local_track(&self, path: Option<&Path>, has_track: bool) -> Result<()> {
        let hosted = path.map(is_hosted_path);
        let hosted_changes =
            hosted.is_some_and(|hosted| hosted != self.local_track_hosted.load(Ordering::Acquire));
        if !hosted_changes && has_track == self.local_track_loaded.load(Ordering::Acquire) {
            return Ok(());
        }
        let current = self
            .current_player
            .lock()
            .map_err(|_| anyhow::anyhow!("Current player lock poisoned"))?;
        if let Some(hosted) = hosted
            && self.local_track_hosted.swap(hosted, Ordering::AcqRel) != hosted
        {
            self.publish_visualizer_inputs(&current)?;
        }
        self.local_track_loaded.store(has_track, Ordering::Release);
        self.publish_player_host(&current);
        Ok(())
    }

    pub(crate) async fn transition_current_player(
        &self,
        previous: CurrentPlayer,
        current: CurrentPlayer,
    ) -> Result<()> {
        let leaving_local = matches!(
            (&previous, &current),
            (CurrentPlayer::Local, CurrentPlayer::RunePeer { .. })
                | (CurrentPlayer::Local, CurrentPlayer::ServicePlayer(_))
        );
        let returning_local = matches!(
            (&previous, &current),
            (CurrentPlayer::RunePeer { .. }, CurrentPlayer::Local)
                | (CurrentPlayer::ServicePlayer(_), CurrentPlayer::Local)
        );
        let renderer_lost = matches!(
            (&previous, &current),
            (
                CurrentPlayer::RunePeer { link: Some(_), .. },
                CurrentPlayer::RunePeer {
                    link: None,
                    reconnecting: None,
                    ..
                }
            )
        );
        // From the switch until its event loop has handled what the player reported before, this
        // device's session stays out of the UI, so that no status from before the pause reaches
        // it. A failed switch drops this and leaves the session as it was.
        let hidden = returning_local.then(|| HiddenLocalSession::new(&self.local_session_hidden));
        let leaving = {
            // A departure admits a new tenure under this lock, so none can be admitted between
            // the switch and taking the tenure it leaves. That tenure ends at the switch, so a
            // request arriving from now on is refused even if the departure's task starts late.
            let tenure = lock_tenures(&self.local_player_tenure)?;
            self.refresh_current_player(current)?;
            leaving_local.then(|| {
                tenure.close();
                Arc::clone(&tenure)
            })
        };
        if let Some(ending) = leaving {
            // The departure runs on its own task, so its pause still follows when the request
            // that changed the player is cancelled, as a UI replacement cancels the UI's requests.
            tokio::spawn(depart_local_player(
                Arc::clone(&self.local_player_tenure),
                ending,
                self.session()?,
            ))
            .await
            .context("Local player departure stopped")??;
        } else if let Some(hidden) = hidden {
            self.session()?.show_again(hidden).await;
        } else if renderer_lost {
            self.session()?.reemit_current().await?;
        }
        Ok(())
    }

    pub(crate) async fn attach_remote_ui(
        &self,
        params: &Arc<GlobalParams>,
        fingerprint: String,
        record_id: Option<LibraryRecordId>,
    ) -> Result<u64> {
        self.attach_ui(params).await;
        self.set_open_catalog(
            record_id,
            params.library_id,
            LibraryBinding::RemotePeer {
                fingerprint,
                reconnect: None,
            },
        )
        .await
    }

    pub(crate) async fn set_open_local_catalog(
        &self,
        record_id: Option<LibraryRecordId>,
        library_id: Uuid,
    ) -> Result<u64> {
        self.set_open_catalog(record_id, Some(library_id), LibraryBinding::Local)
            .await
    }

    pub(crate) async fn set_open_service_catalog(
        self: &Arc<Self>,
        record_id: Option<LibraryRecordId>,
        binding: Arc<dyn ServiceBinding>,
    ) -> Result<u64> {
        let input_changes = Arc::clone(&binding)
            .own_player()
            .map(|player| player.visualizer_input_changes());
        let library_id = binding.library_id();
        let generation = self
            .set_open_catalog(
                record_id,
                Some(library_id),
                LibraryBinding::Service(binding),
            )
            .await?;
        // The visualizer inputs are published only from the current player. What the service's
        // own player gives the visualizer may change, as when its feed drops or returns, which
        // publishes them again. The task ends when the binding is dropped.
        if let Some(mut input_changes) = input_changes {
            let node = Arc::downgrade(self);
            tokio::spawn(async move {
                while input_changes.changed().await.is_ok() {
                    let Some(node) = node.upgrade() else {
                        break;
                    };
                    if let Err(error) = node.republish_current_player_inputs() {
                        log::warn!("Could not republish the visualizer inputs: {error:#}");
                    }
                }
            });
        }
        Ok(generation)
    }

    /// The binding of the service library `library_id` when it is the UI's open library.
    pub(crate) fn service_binding(
        &self,
        library_id: Uuid,
    ) -> Result<Option<Arc<dyn ServiceBinding>>> {
        let ui = self
            .ui_library
            .lock()
            .map_err(|_| anyhow::anyhow!("UI library lock poisoned"))?;
        Ok(match &*ui {
            UiLibrary::Open(OpenLibraryState {
                catalog: LibraryBinding::Service(binding),
                ..
            }) if binding.library_id() == library_id => Some(Arc::clone(binding)),
            _ => None,
        })
    }

    /// Whether the UI's open library is a service library.
    pub(crate) fn service_library_open(&self) -> bool {
        self.ui_library.lock().is_ok_and(|ui| {
            matches!(
                &*ui,
                UiLibrary::Open(OpenLibraryState {
                    catalog: LibraryBinding::Service(_),
                    ..
                })
            )
        })
    }

    /// The open service library, when this device plays its tracks itself.
    pub(crate) fn open_service_played_here(&self) -> Result<Option<Arc<dyn ServiceBinding>>> {
        let ui = self
            .ui_library
            .lock()
            .map_err(|_| anyhow::anyhow!("UI library lock poisoned"))?;
        Ok(match &*ui {
            UiLibrary::Open(OpenLibraryState {
                catalog: LibraryBinding::Service(binding),
                ..
            }) if Arc::clone(binding).own_player().is_none() => Some(Arc::clone(binding)),
            _ => None,
        })
    }

    async fn set_open_catalog(
        &self,
        record_id: Option<LibraryRecordId>,
        library_id: Option<Uuid>,
        catalog: LibraryBinding,
    ) -> Result<u64> {
        let previous_player = self.current_player_snapshot()?;
        let own_player = match &catalog {
            LibraryBinding::Service(binding) => Arc::clone(binding).own_player(),
            _ => None,
        };
        let generation = {
            let mut ui = self
                .ui_library
                .lock()
                .map_err(|_| anyhow::anyhow!("UI library lock poisoned"))?;
            let generation = self.open_generation.fetch_add(1, Ordering::AcqRel) + 1;
            *ui = UiLibrary::Open(OpenLibraryState {
                record_id,
                generation,
                library_id,
                catalog,
            });
            generation
        };
        self.transition_current_player(previous_player, self.current_player()?)
            .await?;
        if let Some(player) = own_player {
            player.follow();
        }
        self.publish_capabilities()?;
        Ok(generation)
    }

    pub(crate) fn capability_library_identity(
        &self,
    ) -> Result<(Option<String>, u64, Option<Uuid>)> {
        let ui = self
            .ui_library
            .lock()
            .map_err(|_| anyhow::anyhow!("UI library lock poisoned"))?;
        Ok(match &*ui {
            UiLibrary::Closed => (None, 0, None),
            UiLibrary::Open(open) => (
                open.record_id.map(|record_id| record_id.0.to_string()),
                open.generation,
                open.library_id,
            ),
        })
    }

    pub fn renderer_link(&self) -> Result<Option<Arc<RendererLink>>> {
        let slot = self
            .renderer_link
            .lock()
            .map_err(|_| anyhow::anyhow!("Renderer link lock poisoned"))?;
        Ok(slot.clone())
    }

    pub fn manager(&self) -> Result<Arc<ServerManager>> {
        self.server_manager
            .get()
            .cloned()
            .context("Node server is not initialized")
    }

    /// Verify that a remote session is an approved device, as this node knows it.
    async fn verified_peer(&self, session: &Session) -> Result<PeerIdentity> {
        let user = self
            .permission_manager
            .read()
            .await
            .verify_by_fingerprint(&session.fingerprint)
            .await
            .filter(|user| {
                user.status == UserStatus::Approved && user.public_key == session.public_key
            })
            .ok_or(Denial::Untrusted)?;
        Ok(PeerIdentity {
            fingerprint: user.fingerprint,
            public_key: user.public_key,
        })
    }

    /// Authorize a remote session to read the given libraries through this node. A player
    /// request's own access rule is Control, which is node-wide, so the libraries its player
    /// reads need this separate library-scoped check. The session is verified once for the
    /// whole request.
    pub(crate) async fn authorize_library_views(
        &self,
        session: &Session,
        library_ids: impl IntoIterator<Item = Uuid>,
    ) -> Result<()> {
        let peer = self.verified_peer(session).await?;
        for library_id in library_ids {
            self.access_policy
                .check(&peer, &Action::ConnectionView { library_id })?;
        }
        Ok(())
    }

    /// Authorize against the exact UI library captured for this dispatch.
    pub(crate) async fn authorize_request(
        &self,
        session: &Session,
        name: &str,
        params: Option<&GlobalParams>,
        addressed_library_id: Option<Uuid>,
    ) -> Result<()> {
        let peer = self.verified_peer(session).await?;
        let access = request_access(name).ok_or(Denial::NodeLocal)?;
        if access == RequestAccess::NodeLocal {
            return Err(Denial::NodeLocal.into());
        }
        let scope = request_scope(name).ok_or(Denial::NodeLocal)?;
        let library_id = if scope == RequestScope::Library {
            Some(
                addressed_library_id
                    .or_else(|| params.and_then(|params| params.library_id))
                    .ok_or(ExpectedRequestError::Unavailable {
                        what: "UI library identity",
                    })?,
            )
        } else {
            None
        };
        let action = match access {
            RequestAccess::Control => Action::Control,
            RequestAccess::Administer => Action::Administer,
            RequestAccess::ConnectionView => Action::ConnectionView {
                library_id: library_id.ok_or(Denial::NodeLocal)?,
            },
            RequestAccess::ConnectionEdit => Action::ConnectionEdit {
                library_id: library_id.ok_or(Denial::NodeLocal)?,
            },
            RequestAccess::NodeLocal => return Err(Denial::NodeLocal.into()),
        };
        self.access_policy.check(&peer, &action)?;
        Ok(())
    }

    pub async fn current_ui(&self) -> Result<Arc<GlobalParams>> {
        self.current_ui
            .lock()
            .await
            .as_ref()
            .and_then(Weak::upgrade)
            .filter(|params| !params.main_token.is_cancelled())
            .ok_or_else(|| {
                ExpectedRequestError::Unavailable {
                    what: "no library is open on this device",
                }
                .into()
            })
    }

    /// Build request parameters from node-owned services for a node with no UI
    /// attached. With a UI, derive them from the UI's params instead (`for_library`),
    /// which shares its effect player and task tokens.
    pub(crate) fn params_for_library(
        self: &Arc<Self>,
        library: LibraryHandle,
    ) -> Result<GlobalParams> {
        Ok(self.params_for_ui()?.for_library(library))
    }

    pub(crate) fn params_for_ui(self: &Arc<Self>) -> Result<GlobalParams> {
        let token = Arc::new(self.token.child_token());
        let session = self.session()?;
        let server_manager = OnceLock::new();
        server_manager
            .set(self.manager()?)
            .map_err(|_| anyhow::anyhow!("Could not initialize request server manager"))?;
        let base = GlobalParams {
            node: self.clone(),
            library_id: None,
            library_pool: self.library_pool.clone(),
            library: None,
            ui_library: Arc::new(Mutex::new(None)),
            config_path: self.config_path.clone(),
            sync_settings: self.sync_settings.clone(),
            node_id: self.node_id.clone(),
            main_token: token.clone(),
            library_tasks: self.library_tasks.clone(),
            player: session.player.clone(),
            sfx_player: Arc::new(Mutex::new(SfxPlayer::new(Some((*token).clone())))),
            scrobbler: session.scrobbler.clone(),
            broadcaster: self.broadcaster.clone(),
            device_scanner: self.device_scanner.clone(),
            cert_validator: self.cert_validator.clone(),
            permission_manager: self.permission_manager.clone(),
            server_manager,
        };
        Ok(base)
    }

    pub async fn attach_ui(&self, params: &Arc<GlobalParams>) {
        let mut current = self.current_ui.lock().await;
        if let Some(old) = current.as_ref().and_then(Weak::upgrade) {
            if Arc::ptr_eq(&old, params) {
                if let Err(error) = self.publish_capabilities() {
                    log::warn!("Could not publish request capabilities on UI attach: {error:#}");
                }
                return;
            }
            Self::release_ui(&old).await;
        }
        *current = Some(Arc::downgrade(params));
        drop(current);
        self.leave_remote_ui().await;
        if let Err(error) = self.publish_capabilities() {
            log::warn!("Could not publish request capabilities on UI attach: {error:#}");
        }
        // A file played by its path is read through the UI's local library, if it has one.
        self.item_capabilities_changed();
    }

    async fn release_ui(params: &GlobalParams) {
        if let Some(library) = params.ui_library.lock().await.take() {
            let record_id = library.descriptor.record_id.0.to_string();
            for task in [
                LibraryTaskKind::Scan,
                LibraryTaskKind::Analyze,
                LibraryTaskKind::Deduplicate,
            ] {
                if let Some(state) =
                    LibraryTaskOwner::cancel(&params.library_tasks, &record_id, task).await
                {
                    params
                        .broadcaster
                        .broadcast(&LibraryTaskStateChanged { state });
                }
            }
            library.pool().unregister_ui(library.library_id).await;
        }
        params.main_token.cancel();
    }

    async fn leave_remote_ui(&self) {
        let previous_player = self.current_player_snapshot().ok();
        let (remote, service) = self.ui_library.lock().map_or((false, None), |mut ui| {
            let remote = matches!(
                &*ui,
                UiLibrary::Open(OpenLibraryState {
                    catalog: LibraryBinding::RemotePeer { .. },
                    ..
                })
            );
            let service = match &*ui {
                UiLibrary::Open(OpenLibraryState {
                    catalog: LibraryBinding::Service(binding),
                    ..
                }) => Some(Arc::clone(binding)),
                _ => None,
            };
            if matches!(
                &*ui,
                UiLibrary::Open(OpenLibraryState {
                    catalog: LibraryBinding::RemotePeer {
                        reconnect: Some(_),
                        ..
                    },
                    ..
                })
            ) {
                self.gui_broadcaster
                    .broadcast(&RendererAttachmentChanged { renderer: None });
            }
            *ui = UiLibrary::Closed;
            (remote, service)
        });
        if let Some(binding) = service {
            binding.close();
        }
        if !remote
            && let Some(previous_player) = previous_player
            && let Ok(current_player) = self.current_player()
            && let Err(error) = self
                .transition_current_player(previous_player, current_player)
                .await
        {
            log::warn!("Could not update current player after remote UI close: {error:#}");
        }
        if let Err(error) = self.publish_capabilities() {
            log::warn!("Could not publish request capabilities after remote UI close: {error:#}");
        }
        if remote && let Err(error) = self.detach_renderer().await {
            log::warn!("Could not detach remote library: {error:#}");
        }
    }

    pub async fn close_ui(&self, token: &Arc<CancellationToken>) {
        let params = {
            let mut current = self.current_ui.lock().await;
            let params = current
                .as_ref()
                .and_then(Weak::upgrade)
                .filter(|params| Arc::ptr_eq(&params.main_token, token));
            if params.is_some() {
                *current = None;
            }
            params
        };
        if let Some(params) = params {
            self.leave_remote_ui().await;
            Self::release_ui(&params).await;
            self.item_capabilities_changed();
        }
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.leave_remote_ui().await;
        let session_result = if let Some(session) = self.local_session.get() {
            session.shutdown().await
        } else {
            Ok(())
        };
        self.token.cancel();
        if let Some(params) = self
            .current_ui
            .lock()
            .await
            .take()
            .and_then(|params| params.upgrade())
        {
            Self::release_ui(&params).await;
        }
        LibraryTaskOwner::shutdown(&self.library_tasks, self.broadcaster.as_ref()).await;
        let mut result = session_result;
        if let Some(manager) = self.server_manager.get() {
            if manager.is_running() {
                result = result.and(manager.stop().await);
            }
            let worker_result = manager.stop_worker().await;
            result = result.and(worker_result);
        }
        for handle in self.event_handles.lock().await.drain(..) {
            result = result.and(handle.await.context("Node event listener failed"));
        }
        result = result.and(self.library_pool.close_idle().await);
        result
    }

    /// Register a service cover cache for tracking. The cache will be automatically
    /// cleaned up when the last reference to it is dropped (using Weak).
    pub(crate) fn register_cover_cache(&self, library_id: Uuid, cache: &Arc<ServiceCoverCache>) {
        // Check if library is being removed - if so, don't register
        {
            let removing = self
                .removing_libraries
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if removing.contains(&library_id) {
                // Library is being removed, skip registration to prevent
                // writes after removal completes
                return;
            }
        }
        let mut caches = self.cover_caches.lock().unwrap_or_else(|e| e.into_inner());
        caches
            .entry(library_id)
            .or_default()
            .push(Arc::downgrade(cache));
    }

    /// Cancel cover downloads for a library and wait for them to complete.
    pub(crate) async fn cancel_cover_downloads_for_library(&self, library_id: Uuid) {
        // Mark library as being removed so new bindings don't register caches
        {
            let mut removing = self
                .removing_libraries
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            removing.insert(library_id);
        }

        // Get all caches for this library and filter out expired ones
        let caches_to_cancel = {
            let caches = self.cover_caches.lock().unwrap_or_else(|e| e.into_inner());
            caches
                .get(&library_id)
                .map(|vec| {
                    vec.iter()
                        .filter_map(|weak| weak.upgrade())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };

        // Cancel and wait for downloads for each live cache
        for cache in caches_to_cancel {
            cache.close_and_wait_downloads().await;
        }

        // Remove from tracking
        {
            let mut removing = self
                .removing_libraries
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            removing.remove(&library_id);
        }
    }
}

#[cfg(unix)]
fn raise_nofile_soft_limit() {
    let mut limits = MaybeUninit::<libc::rlimit>::uninit();
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limits.as_mut_ptr()) } != 0 {
        log::warn!("Could not read RLIMIT_NOFILE: {}", Error::last_os_error());
        return;
    }
    let mut limits = unsafe { limits.assume_init() };
    let before = limits.rlim_cur;
    let target = limits.rlim_max.min(8192);
    if before < target {
        limits.rlim_cur = target;
        if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limits) } != 0 {
            log::warn!(
                "Could not raise RLIMIT_NOFILE soft limit from {before} towards {target}: {}",
                Error::last_os_error()
            );
        }
    }
    let mut after = MaybeUninit::<libc::rlimit>::uninit();
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, after.as_mut_ptr()) } != 0 {
        log::warn!(
            "Could not read RLIMIT_NOFILE after adjustment: {}",
            Error::last_os_error()
        );
        return;
    }
    let after = unsafe { after.assume_init() };
    log::info!(
        "RLIMIT_NOFILE soft limit: {before} -> {} (hard limit {})",
        after.rlim_cur,
        after.rlim_max
    );
}

#[cfg(all(test, unix))]
mod nofile_tests {
    use super::{MaybeUninit, raise_nofile_soft_limit};

    #[cfg(unix)]
    #[test]
    fn raises_soft_limit_towards_hard_limit() {
        let mut limits = MaybeUninit::<libc::rlimit>::uninit();
        let result = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limits.as_mut_ptr()) };
        assert_eq!(result, 0, "read RLIMIT_NOFILE");
        let limits = unsafe { limits.assume_init() };
        let target = limits.rlim_max.min(8192);

        raise_nofile_soft_limit();

        let mut after = MaybeUninit::<libc::rlimit>::uninit();
        let result = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, after.as_mut_ptr()) };
        assert_eq!(result, 0, "read RLIMIT_NOFILE after initialization");
        let after = unsafe { after.assume_init() };
        assert!(
            after.rlim_cur >= target,
            "soft limit should reach {target}, got {}",
            after.rlim_cur
        );
    }
}

impl Drop for NodeContext {
    fn drop(&mut self) {
        self.token.cancel();
    }
}

/// Serializes first initialization, including legacy library-open callers.
pub(crate) struct NodeSlot(Mutex<Option<Arc<NodeContext>>>);
impl Default for NodeSlot {
    fn default() -> Self {
        Self::new()
    }
}
impl NodeSlot {
    pub const fn new() -> Self {
        Self(Mutex::const_new(None))
    }
    pub async fn current(&self) -> Result<Arc<NodeContext>> {
        self.0
            .lock()
            .await
            .as_ref()
            .cloned()
            .context("Node is not initialized")
    }
    pub async fn handle(
        &self,
        request: &InitializeNodeRequest,
        broadcaster: Arc<dyn Broadcaster>,
    ) -> Result<InitializeNodeResponse> {
        let path = &request.config_path;
        self.initialize(path, path, broadcaster, RunMode::App)
            .await?;
        Ok(InitializeNodeResponse { call_id: None })
    }
    pub async fn initialize(
        &self,
        config_path: &str,
        db_path: &str,
        broadcaster: Arc<dyn Broadcaster>,
        run_mode: RunMode,
    ) -> Result<Arc<NodeContext>> {
        let mut slot = self.0.lock().await;
        if let Some(node) = slot.as_ref() {
            ensure!(
                Path::new(node.config_path.as_str()) == Path::new(config_path),
                ExpectedRequestError::invalid_input(
                    "Node configuration path cannot change while the node exists"
                )
            );
            return Ok(node.clone());
        }
        let node = NodeContext::new(
            config_path,
            db_path,
            broadcaster,
            true,
            run_mode,
            Arc::clone(&crate::visualizer::VISUALIZER),
        )
        .await?;
        *slot = Some(node.clone());
        Ok(node)
    }
}

pub async fn initialize_node(
    config_path: &str,
    db_path: &str,
    broadcaster: Arc<dyn Broadcaster>,
    run_mode: RunMode,
) -> Result<Arc<NodeContext>> {
    NODE.initialize(config_path, db_path, broadcaster, run_mode)
        .await
}

pub async fn receive_initialize_node() {
    let receiver = InitializeNodeRequest::get_dart_signal_receiver();
    let emit = |signal: &dyn RinfRustSignal| signal.send_to_dart();
    while let Some(signal) = receiver.recv().await {
        answer_initialize_node(&NODE, &signal.message, Arc::new(LocalGuiBroadcaster), &emit).await;
    }
}

/// Initialize the node for one request and answer it like a catalogue request.
async fn answer_initialize_node(
    slot: &NodeSlot,
    request: &InitializeNodeRequest,
    broadcaster: Arc<dyn Broadcaster>,
    emit: &impl Fn(&dyn RinfRustSignal),
) {
    let result = slot.handle(request, broadcaster).await.map(Some);
    answer_node_request(request, result, emit);
}

impl StandaloneRequestReceiver for InitializeNodeRequest {
    fn start() {
        tokio::spawn(receive_initialize_node());
    }
}

pub(crate) fn node_slot() -> &'static NodeSlot {
    &NODE
}

pub async fn shutdown_node() -> Result<()> {
    if let Some(node) = NODE.0.lock().await.as_ref() {
        node.shutdown().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path as std_path;
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;
    use visualizer::Visualizer as VisualizerVisualizer;

    use discovery::{server::UserStatus, utils::DeviceType};
    use tokio::{
        sync::mpsc,
        time::{Duration, timeout},
    };

    use super::{NodeBroadcaster, NodeContext};
    use crate::server::utils::device::RunMode;
    use crate::{
        messages::{LibraryContentChanged, TrustListUpdated},
        request_owner::CurrentPlayer,
        server::WebSocketService,
        utils::{Broadcaster, RinfRustSignal},
    };

    struct Quiet(Option<mpsc::UnboundedSender<String>>);
    impl Broadcaster for Quiet {
        fn broadcast(&self, message: &dyn RinfRustSignal) {
            if let Some(sender) = &self.0 {
                let _ = sender.send(message.name());
            }
        }
    }

    #[tokio::test]
    async fn permission_status_change_emits_notification() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        crate::test_node::disable_sync_listener(dir.path())?;
        let path = dir.path().to_string_lossy();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let node = NodeContext::new(
            &path,
            &path,
            Arc::new(Quiet(Some(tx))),
            false,
            RunMode::App,
            Arc::new(VisualizerVisualizer::new()),
        )
        .await?;
        let manager = node.permission_manager.read().await;
        manager
            .add_user(
                "key".into(),
                "fingerprint".into(),
                "alias".into(),
                "model".into(),
                DeviceType::Desktop,
                "ip".into(),
            )
            .await?;
        timeout(Duration::from_secs(2), async {
            while let Some(name) = rx.recv().await {
                if name == "TrustedClientsChanged" {
                    return;
                }
            }
        })
        .await?;
        manager
            .change_user_status("fingerprint", UserStatus::Approved)
            .await?;
        drop(manager);
        let result = timeout(Duration::from_secs(2), async {
            while let Some(name) = rx.recv().await {
                if name == "TrustedClientsChanged" {
                    return true;
                }
            }
            false
        })
        .await;
        node.shutdown().await?;
        assert_eq!(result.ok(), Some(true));
        Ok(())
    }

    #[test]
    fn only_controller_signals_reach_remote_clients() {
        let remote = Arc::new(WebSocketService::new());
        let mut received = remote.broadcast_tx.subscribe();
        let broadcaster = NodeBroadcaster {
            current_player: Arc::new(StdMutex::new(CurrentPlayer::Local)),
            local_session_hidden: Arc::default(),
            local: Arc::new(Quiet(None)),
            remote,
        };
        broadcaster.broadcast(&TrustListUpdated {
            certificates: Vec::new(),
        });
        assert!(received.try_recv().is_err());
        broadcaster.broadcast(&LibraryContentChanged { path: "a".into() });
        assert!(received.try_recv().is_ok());
    }

    #[tokio::test]
    async fn node_initialization_answers_name_their_call_and_failures_are_typed()
    -> anyhow::Result<()> {
        use super::{NodeSlot, answer_initialize_node};
        use crate::messages::{
            InitializeNodeRequest, InitializeNodeResponse, RequestErrorKind, RequestFailedResponse,
        };
        use crate::utils::sync_settings::SyncSettings;

        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        SyncSettings {
            listen: false,
            ..SyncSettings::default()
        }
        .save(first.path())?;
        let slot = NodeSlot::new();
        let answers = StdMutex::new(Vec::new());
        let emit = |signal: &dyn RinfRustSignal| {
            let any = signal.as_any();
            let answer = if let Some(response) = any.downcast_ref::<InitializeNodeResponse>() {
                (response.call_id, None)
            } else if let Some(failure) = any.downcast_ref::<RequestFailedResponse>() {
                (failure.call_id, Some(failure.kind))
            } else {
                panic!("unexpected answer {}", signal.name());
            };
            answers.lock().unwrap().push(answer);
        };

        let request = |path: &std_path::Path, call_id| InitializeNodeRequest {
            config_path: path.to_string_lossy().into_owned(),
            call_id: Some(call_id),
        };
        answer_initialize_node(
            &slot,
            &request(first.path(), 7),
            Arc::new(Quiet(None)),
            &emit,
        )
        .await;
        // The node exists, so another configuration path is refused.
        answer_initialize_node(
            &slot,
            &request(second.path(), 8),
            Arc::new(Quiet(None)),
            &emit,
        )
        .await;

        assert_eq!(
            *answers.lock().unwrap(),
            [
                (Some(7), None),
                (Some(8), Some(RequestErrorKind::InvalidInput))
            ]
        );
        slot.current().await?.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn ending_the_local_player_tenure_waits_for_the_requests_running_in_it()
    -> anyhow::Result<()> {
        use super::LocalPlayerTenure;
        use crate::request_error::ExpectedRequestError;

        let tenure = Arc::new(LocalPlayerTenure::default());
        let running = tenure.enter().await?;
        let mut end = Box::pin({
            let tenure = Arc::clone(&tenure);
            async move { tenure.end().await }
        });

        assert!(futures::poll!(end.as_mut()).is_pending());
        assert!(futures::poll!(Box::pin(tenure.ended())).is_ready());
        drop(running);
        end.await;

        let error = tenure
            .enter()
            .await
            .expect_err("an ended tenure must not admit a request");
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::PlayerChanged)
        ));
        Ok(())
    }
}
