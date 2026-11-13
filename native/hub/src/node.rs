mod reconnect;
pub(crate) use reconnect::ReconnectConfig;
use reconnect::ReconnectTask;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;

use std::{
    collections::HashMap,
    fmt::{Debug, Formatter, Result as FmtResult},
    fs,
    path::Path,
    sync::{
        Arc, Mutex as StdMutex, OnceLock, Weak,
        atomic::{AtomicU64, Ordering},
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
    player::{HandoffBlockId as PlayerHandoffBlockId, PlaybackState},
    sfx_player::SfxPlayer,
};
use rinf::{DartSignal, RustSignal};
use tokio::{
    sync::{Mutex, RwLock, broadcast},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::access_policy::{Action, Denial, RuneAccessPolicy};
use crate::backends::remote::{RemoteConnectError, RendererLink, RendererLinkUse};
use crate::macros::{
    RequestAccess, RequestScope, StandaloneRequestReceiver, capability_snapshot, request_access,
    request_scope,
};
use crate::request_error::ExpectedRequestError;
use crate::request_owner::{CurrentPlayer, Implementation, OpenLibrary};
use crate::rpc::{Slot, TargetRequest};
use crate::server::sync_worker::SyncTokenStore;
use crate::subsonic_binding::SubsonicBinding;
use crate::visualizer::VISUALIZER;
use crate::{Session, session::PlayerSession};
use crate::{
    library_pool::{LibraryHandle, LibraryPool},
    messages::{
        AttachedRenderer, CapabilityReason, ClientStatus, ClientSummary,
        IncommingClientPermissionNotification, InitializeNodeRequest, InitializeNodeResponse,
        LibraryTaskKind, LibraryTaskStateChanged, PauseRequest, PlaybackStatus, PlaylistItem,
        PlaylistUpdate, ProtocolUpdateSide, RefreshSessionRequest, RemoteLibraryReconnecting,
        RemoteLossKind, RendererAttachmentChanged, RendererLinkLost, RequestCapabilities,
        RequestCapability, TrustListUpdated, TrustedClientsChanged, TrustedServerCertificate,
    },
    request_error::classify,
    server::{ServerManager, WebSocketService, utils::device::load_device_info},
    utils::{
        Broadcaster, GlobalParams, LibraryTaskOwner, LocalGuiBroadcaster, RemoteEventScope,
        RinfRustSignal, nid::get_or_create_node_id, sync_settings::SyncSettings,
    },
};
use database::library_registry::LibraryRecordId;
use discovery::DiscoveryParams;
use discovery::access_policy::{AccessPolicy, PeerIdentity};
use discovery::endpoint::DEFAULT_DEVICE_PORT;
use discovery::protocol_version::{PROTOCOL_VERSION, ProtocolVersion};

static NODE: NodeSlot = NodeSlot::new();

fn renderer_connection_error(error: RemoteConnectError) -> anyhow::Error {
    let detail = format!("Renderer connection failed: {error:?}");
    match error {
        RemoteConnectError::UntrustedEndpoint | RemoteConnectError::NotAuthorized => {
            anyhow::Error::new(Denial::Untrusted).context(detail)
        }
        RemoteConnectError::Unreachable | RemoteConnectError::Protocol(_) => {
            anyhow::Error::new(ExpectedRequestError::Unavailable { what: "renderer" })
                .context(detail)
        }
        RemoteConnectError::PeerNeedsUpdate {
            local,
            peer,
            update_peer,
            peer_alias,
        } => anyhow::Error::new(ExpectedRequestError::PeerNeedsUpdate {
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
    Subsonic(Arc<SubsonicBinding>),
    RemotePeer {
        fingerprint: String,
        reconnect: Option<ReconnectTask>,
    },
}

/// Process services shared by every local library and remote controller.
pub struct NodeContext {
    protocol_version: StdMutex<ProtocolVersion>,
    pub config_path: Arc<String>,
    pub node_id: Arc<String>,
    pub library_pool: Arc<LibraryPool>,
    pub credentials: Arc<CredentialStore>,
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
    open_generation: AtomicU64,
    reconnect_config: ReconnectConfig,
    #[cfg(test)]
    reconnect_attempts: AtomicUsize,
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
    #[cfg(test)]
    handoff_pause_failures: AtomicUsize,
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
    local: Arc<dyn Broadcaster>,
    remote: Arc<WebSocketService>,
    handoff_playback_events: broadcast::Sender<HandoffPlaybackEvent>,
}

#[derive(Clone)]
pub(crate) enum HandoffPlaybackEvent {
    Status(Box<PlaybackStatus>),
    Playlist(Vec<PlaylistItem>),
}

impl Broadcaster for NodeBroadcaster {
    fn broadcast(&self, message: &dyn RinfRustSignal) {
        if let Some(status) = message.as_any().downcast_ref::<PlaybackStatus>() {
            let _ = self
                .handoff_playback_events
                .send(HandoffPlaybackEvent::Status(Box::new(status.clone())));
        } else if let Some(playlist) = message.as_any().downcast_ref::<PlaylistUpdate>() {
            let _ = self
                .handoff_playback_events
                .send(HandoffPlaybackEvent::Playlist(playlist.items.clone()));
        }
        let remote_event = message.remote_event();
        let suppress_local_status = self
            .current_player
            .lock()
            .is_ok_and(|player| player.suppresses_local_session_events())
            && remote_event == Some(RemoteEventScope::Session)
            && message.name() != "RequestCapabilities";
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
    pub(crate) async fn new(
        config_path: &str,
        db_path: &str,
        broadcaster: Arc<dyn Broadcaster>,
        os_media_controls: bool,
    ) -> Result<Arc<Self>> {
        Self::with_reconnect_config(
            config_path,
            db_path,
            broadcaster,
            os_media_controls,
            ReconnectConfig::default(),
        )
        .await
    }

    pub(crate) async fn with_reconnect_config(
        config_path: &str,
        db_path: &str,
        broadcaster: Arc<dyn Broadcaster>,
        os_media_controls: bool,
        reconnect_config: ReconnectConfig,
    ) -> Result<Arc<Self>> {
        #[cfg(unix)]
        raise_nofile_soft_limit();
        fs::create_dir_all(config_path).context("Create node configuration directory")?;
        let node_id = Arc::new(get_or_create_node_id(config_path).await?.to_string());
        let websocket_service = Arc::new(WebSocketService::new());
        let renderer_link = Arc::new(StdMutex::new(None));
        let ui_library = Arc::new(StdMutex::new(UiLibrary::Closed));
        let current_player = Arc::new(StdMutex::new(CurrentPlayer::Local));
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
            library_pool: LibraryPool::new_with_credentials(
                Path::new(config_path),
                Path::new(db_path),
                &node_id,
                credentials.clone(),
            )?,
            credentials,
            node_id,
            permission_manager: Arc::new(RwLock::new(permission_manager)),
            access_policy,
            cert_validator: Arc::new(RwLock::new(CertValidator::new(config_path).await?)),
            device_scanner: Arc::new(DiscoveryService::without_store()),
            sync_settings: Arc::new(RwLock::new(SyncSettings::load(Path::new(config_path)))),
            broadcaster: Arc::new(NodeBroadcaster {
                current_player: current_player.clone(),
                local: broadcaster.clone(),
                remote: websocket_service.clone(),
                handoff_playback_events: handoff_playback_events.clone(),
            }),
            gui_broadcaster: broadcaster,
            handoff_playback_events,
            websocket_service,
            renderer_link,
            ui_library,
            current_player,
            open_generation: AtomicU64::new(0),
            reconnect_config,
            #[cfg(test)]
            reconnect_attempts: AtomicUsize::new(0),
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
            #[cfg(test)]
            handoff_pause_failures: AtomicUsize::new(0),
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
            .set(PlayerSession::new(&node, "local", os_media_controls).await?)
            .map_err(|_| anyhow::anyhow!("Local session already initialized"))?;
        node.refresh_current_player(node.current_player()?)?;
        node.start_notifications().await;
        manager.start_worker().await?;
        if node.sync_settings.read().await.listen {
            let result = async {
                let device_info = load_device_info(Path::new(config_path)).await?;
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

        #[cfg(test)]
        if self
            .handoff_pause_failures
            .try_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            anyhow::bail!("test pause failure");
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

    #[cfg(test)]
    pub(crate) fn fail_next_handoff_pauses(&self, attempts: usize) {
        self.handoff_pause_failures
            .store(attempts, Ordering::Release);
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
            return Err(anyhow::Error::new(ExpectedError::NotFound {
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
                catalog: LibraryBinding::Subsonic(binding),
                ..
            }) => OpenLibrary::Subsonic(Arc::clone(binding)),
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
                attached: false,
                reconnecting: reconnect.as_ref().map(|task| task.alias.clone()),
            },
            UiLibrary::Open(OpenLibraryState {
                catalog: LibraryBinding::Subsonic(binding),
                ..
            }) if binding.play_on_this_device => CurrentPlayer::Local,
            UiLibrary::Open(OpenLibraryState {
                catalog: LibraryBinding::Subsonic(binding),
                ..
            }) => CurrentPlayer::SubsonicJukebox(Arc::clone(binding)),
            _ => match renderer_link {
                Some(link) => CurrentPlayer::RunePeer {
                    link: Some(link),
                    attached: true,
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
        capabilities.for_each_capability_mut(|access, capability| {
            let allowed = match access {
                RequestAccess::NodeLocal => false,
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
                    reason: CapabilityReason::OwnerOnly,
                };
            }
        });
        capabilities
    }

    pub(crate) fn implementation_for_scope<R: TargetRequest>(&self) -> Result<Implementation> {
        match R::SCOPE {
            RequestScope::Node => Ok(Implementation::Local),
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

    fn refresh_current_player(&self, player: CurrentPlayer) -> Result<()> {
        VISUALIZER.set_inputs(player.visualizer_inputs());
        *self
            .current_player
            .lock()
            .map_err(|_| anyhow::anyhow!("Current player lock poisoned"))? = player;
        Ok(())
    }

    pub(crate) async fn transition_current_player(
        &self,
        previous: CurrentPlayer,
        current: CurrentPlayer,
    ) -> Result<()> {
        let leaving_local = matches!(
            (&previous, &current),
            (
                CurrentPlayer::Local,
                CurrentPlayer::RunePeer { attached: true, .. }
            ) | (
                CurrentPlayer::Local,
                CurrentPlayer::RunePeer {
                    attached: false,
                    ..
                }
            ) | (CurrentPlayer::Local, CurrentPlayer::SubsonicJukebox(_))
        );
        let returning_local = matches!(
            (&previous, &current),
            (
                CurrentPlayer::RunePeer { attached: true, .. },
                CurrentPlayer::Local
            ) | (
                CurrentPlayer::RunePeer {
                    attached: false,
                    ..
                },
                CurrentPlayer::Local
            ) | (CurrentPlayer::SubsonicJukebox(_), CurrentPlayer::Local)
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
        self.refresh_current_player(current)?;
        if leaving_local {
            let session = self.session()?;
            let mut player = session.player.lock().await;
            if player.get_status().state == PlaybackState::Playing {
                player
                    .pause_applied()
                    .await
                    .context("Pause local player when another player becomes current")?;
            }
        } else if returning_local || renderer_lost {
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

    pub(crate) async fn set_open_subsonic_catalog(
        &self,
        record_id: Option<LibraryRecordId>,
        binding: Arc<SubsonicBinding>,
    ) -> Result<u64> {
        self.set_open_catalog(
            record_id,
            Some(binding.library_id),
            LibraryBinding::Subsonic(binding),
        )
        .await
    }

    pub(crate) fn subsonic_binding(
        &self,
        library_id: Uuid,
    ) -> Result<Option<Arc<SubsonicBinding>>> {
        let ui = self
            .ui_library
            .lock()
            .map_err(|_| anyhow::anyhow!("UI library lock poisoned"))?;
        Ok(match &*ui {
            UiLibrary::Open(OpenLibraryState {
                catalog: LibraryBinding::Subsonic(binding),
                ..
            }) if binding.library_id == library_id => Some(Arc::clone(binding)),
            _ => None,
        })
    }

    pub(crate) fn open_subsonic_binding(&self) -> Result<Option<Arc<SubsonicBinding>>> {
        let ui = self
            .ui_library
            .lock()
            .map_err(|_| anyhow::anyhow!("UI library lock poisoned"))?;
        Ok(match &*ui {
            UiLibrary::Open(OpenLibraryState {
                catalog: LibraryBinding::Subsonic(binding),
                ..
            }) => Some(Arc::clone(binding)),
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
        let jukebox_binding = match &catalog {
            LibraryBinding::Subsonic(binding) if !binding.play_on_this_device => {
                Some(Arc::clone(binding))
            }
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
        if let Some(binding) = jukebox_binding {
            binding.start_jukebox_polling();
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

    /// Authorize against the exact UI library captured for this dispatch.
    pub(crate) async fn authorize_request(
        &self,
        session: &Session,
        name: &str,
        params: Option<&GlobalParams>,
        addressed_library_id: Option<Uuid>,
    ) -> Result<()> {
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
        self.access_policy.check(
            &PeerIdentity {
                fingerprint: user.fingerprint,
                public_key: user.public_key,
            },
            &action,
        )?;
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
        let (remote, subsonic) = self.ui_library.lock().map_or((false, None), |mut ui| {
            let remote = matches!(
                &*ui,
                UiLibrary::Open(OpenLibraryState {
                    catalog: LibraryBinding::RemotePeer { .. },
                    ..
                })
            );
            let subsonic = match &*ui {
                UiLibrary::Open(OpenLibraryState {
                    catalog: LibraryBinding::Subsonic(binding),
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
            (remote, subsonic)
        });
        if let Some(binding) = subsonic {
            binding.stop_jukebox_polling();
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
    ) -> InitializeNodeResponse {
        let path = &request.config_path;
        let result = self.initialize(path, path, broadcaster).await;
        let (error_kind, error) = match result {
            Ok(_) => (None, None),
            Err(error) => (Some(classify(&error)), Some(format!("{error:#}"))),
        };
        InitializeNodeResponse {
            call_id: None,
            success: error.is_none(),
            error_kind,
            error,
        }
    }
    pub async fn initialize(
        &self,
        config_path: &str,
        db_path: &str,
        broadcaster: Arc<dyn Broadcaster>,
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
        let node = NodeContext::new(config_path, db_path, broadcaster, true).await?;
        *slot = Some(node.clone());
        Ok(node)
    }
}

pub async fn initialize_node(
    config_path: &str,
    db_path: &str,
    broadcaster: Arc<dyn Broadcaster>,
) -> Result<Arc<NodeContext>> {
    NODE.initialize(config_path, db_path, broadcaster).await
}

pub async fn receive_initialize_node() {
    let receiver = InitializeNodeRequest::get_dart_signal_receiver();
    while let Some(signal) = receiver.recv().await {
        NODE.handle(&signal.message, Arc::new(LocalGuiBroadcaster))
            .await
            .send_signal_to_dart();
    }
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
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    use discovery::{server::UserStatus, utils::DeviceType};
    use tokio::{
        sync::mpsc,
        time::{Duration, timeout},
    };

    use super::{NodeBroadcaster, NodeContext};
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
        let path = dir.path().to_string_lossy();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let node = NodeContext::new(&path, &path, Arc::new(Quiet(Some(tx))), false).await?;
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
        use tokio::sync::broadcast;

        let remote = Arc::new(WebSocketService::new());
        let mut received = remote.broadcast_tx.subscribe();
        let (handoff_playback_events, _) = broadcast::channel(1);
        let broadcaster = NodeBroadcaster {
            current_player: Arc::new(StdMutex::new(CurrentPlayer::Local)),
            local: Arc::new(Quiet(None)),
            remote,
            handoff_playback_events,
        };
        broadcaster.broadcast(&TrustListUpdated {
            certificates: Vec::new(),
        });
        assert!(received.try_recv().is_err());
        broadcaster.broadcast(&LibraryContentChanged { path: "a".into() });
        assert!(received.try_recv().is_ok());
    }
}
