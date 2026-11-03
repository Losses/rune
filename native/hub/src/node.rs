use std::{
    fmt::{Debug, Formatter, Result as FmtResult},
    fs,
    path::Path,
    sync::{Arc, OnceLock, Weak},
};

use anyhow::{Context, Result, ensure};
use discovery::{
    client::CertValidator,
    protocol::DiscoveryService,
    server::{PermissionManager, UserStatus},
};
use rinf::{DartSignal, RustSignal};
use tokio::{
    sync::{Mutex, RwLock},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::Session;
use crate::access_policy::{Action, Denial, RuneAccessPolicy};
use crate::macros::{RequestClass, request_class};
use crate::request_error::ExpectedRequestError;
use crate::{
    library_pool::LibraryPool,
    messages::{
        ClientStatus, ClientSummary, IncommingClientPermissionNotification, InitializeNodeRequest,
        InitializeNodeResponse, TrustListUpdated, TrustedClientsChanged, TrustedServerCertificate,
    },
    request_error::classify,
    server::{ServerManager, WebSocketService, utils::device::load_device_info},
    utils::{
        Broadcaster, GlobalParams, LocalGuiBroadcaster, RinfRustSignal, nid::get_or_create_node_id,
        sync_settings::SyncSettings,
    },
};
use discovery::DiscoveryParams;
use discovery::access_policy::{AccessPolicy, PeerIdentity};

static NODE: NodeSlot = NodeSlot::new();

/// Process services shared by every local library and remote controller.
pub struct NodeContext {
    pub config_path: Arc<String>,
    pub node_id: Arc<String>,
    pub library_pool: Arc<LibraryPool>,
    pub access_policy: Arc<RuneAccessPolicy>,
    pub permission_manager: Arc<RwLock<PermissionManager>>,
    pub cert_validator: Arc<RwLock<CertValidator>>,
    pub device_scanner: Arc<DiscoveryService>,
    pub sync_settings: Arc<RwLock<SyncSettings>>,
    pub broadcaster: Arc<dyn Broadcaster>,
    pub websocket_service: Arc<WebSocketService>,
    pub token: Arc<CancellationToken>,
    pub server_manager: OnceLock<Arc<ServerManager>>,
    current_ui: Mutex<Option<Weak<GlobalParams>>>,
    event_handles: Mutex<Vec<JoinHandle<()>>>,
}

impl Debug for NodeContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("NodeContext")
            .field("config_path", &self.config_path)
            .finish()
    }
}

struct NodeBroadcaster {
    local: Arc<dyn Broadcaster>,
    remote: Arc<WebSocketService>,
}
/// Signals a remote controller renders. Everything else (trust lists, pairing
/// requests, crash details) stays on this device until access control decides
/// per peer what it may see.
const REMOTE_SIGNALS: &[&str] = &[
    "PlaybackStatus",
    "RealtimeFFT",
    "PlaylistUpdate",
    "LibraryContentChanged",
    "ScanAudioLibraryProgress",
    "ScanAudioLibraryResponse",
    "AnalyzeAudioLibraryProgress",
    "AnalyzeAudioLibraryResponse",
];

impl Broadcaster for NodeBroadcaster {
    fn broadcast(&self, message: &dyn RinfRustSignal) {
        self.local.broadcast(message);
        if self.remote.broadcast_tx.receiver_count() > 0
            && REMOTE_SIGNALS.contains(&message.name().as_str())
        {
            self.remote.broadcast(message);
        }
    }
}

impl NodeContext {
    pub(crate) async fn new(
        config_path: &str,
        db_path: &str,
        broadcaster: Arc<dyn Broadcaster>,
    ) -> Result<Arc<Self>> {
        fs::create_dir_all(config_path).context("Create node configuration directory")?;
        let node_id = Arc::new(get_or_create_node_id(config_path).await?.to_string());
        let websocket_service = Arc::new(WebSocketService::new());
        let mut permission_manager = PermissionManager::new(config_path)?;
        let access_policy = Arc::new(RuneAccessPolicy::load(
            Path::new(config_path),
            &permission_manager.list_users().await,
        )?);
        let policy = access_policy.clone();
        permission_manager.set_trust_change_hook(Arc::new(move |fingerprint, status| {
            policy
                .trust_changed(fingerprint, status)
                .map_err(|error| format!("{error:#}"))
        }));
        let node = Arc::new(Self {
            config_path: Arc::new(config_path.to_owned()),
            library_pool: LibraryPool::new(Path::new(config_path), Path::new(db_path), &node_id)?,
            node_id,
            permission_manager: Arc::new(RwLock::new(permission_manager)),
            access_policy,
            cert_validator: Arc::new(RwLock::new(CertValidator::new(config_path).await?)),
            device_scanner: Arc::new(DiscoveryService::without_store()),
            sync_settings: Arc::new(RwLock::new(SyncSettings::load(Path::new(config_path)))),
            broadcaster: Arc::new(NodeBroadcaster {
                local: broadcaster,
                remote: websocket_service.clone(),
            }),
            websocket_service,
            token: Arc::new(CancellationToken::new()),
            server_manager: OnceLock::new(),
            current_ui: Mutex::new(None),
            event_handles: Mutex::new(Vec::new()),
        });
        let manager = Arc::new(ServerManager::new(node.clone()).await?);
        node.server_manager
            .set(manager.clone())
            .map_err(|_| anyhow::anyhow!("Node server already initialized"))?;
        node.start_notifications().await;
        manager.start_worker().await?;
        if node.sync_settings.read().await.listen {
            let result = async {
                let device_info = load_device_info(Path::new(config_path)).await?;
                manager
                    .start_sync(([0, 0, 0, 0], 7863).into(), DiscoveryParams { device_info })
                    .await
            }
            .await;
            if let Err(error) = result {
                log::warn!("Could not start paired sync server: {error:#}");
            }
        }
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
        params: &GlobalParams,
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
        let action = match request_class(name) {
            Some(RequestClass::Control) => Action::Control,
            Some(RequestClass::Administer) => Action::Administer,
            Some(class @ (RequestClass::LibraryRead | RequestClass::LibraryWrite)) => {
                let library_id = params.library_id.ok_or(ExpectedRequestError::Unavailable {
                    what: "UI library identity",
                })?;
                if class == RequestClass::LibraryRead {
                    // The WebSocket is a control attachment: reads there browse.
                    Action::Browse { library_id }
                } else {
                    Action::LibraryWrite { library_id }
                }
            }
            _ => return Err(Denial::NodeLocal.into()),
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

    pub async fn attach_ui(&self, params: &Arc<GlobalParams>) {
        let mut current = self.current_ui.lock().await;
        if let Some(old) = current.as_ref().and_then(Weak::upgrade) {
            if Arc::ptr_eq(&old, params) {
                return;
            }
            Self::release_ui(&old).await;
        }
        *current = Some(Arc::downgrade(params));
    }

    async fn release_ui(params: &GlobalParams) {
        let mut tasks = params.task_tokens.lock().await;
        for token in [
            tasks.scan_token.take(),
            tasks.analyze_token.take(),
            tasks.deduplicate_token.take(),
        ]
        .into_iter()
        .flatten()
        {
            token.cancel();
        }
        drop(tasks);
        if let Some(library) = params.ui_library.lock().await.take() {
            library
                .pool()
                .unregister_ui(library.descriptor.library_id)
                .await;
        }
        params.main_token.cancel();
    }

    pub async fn close_ui(&self, token: &Arc<CancellationToken>) {
        let mut current = self.current_ui.lock().await;
        if let Some(params) = current.as_ref().and_then(Weak::upgrade)
            && Arc::ptr_eq(&params.main_token, token)
        {
            *current = None;
            Self::release_ui(&params).await;
        }
    }

    pub async fn shutdown(&self) -> Result<()> {
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
        let mut result = Ok(());
        if let Some(manager) = self.server_manager.get() {
            if manager.is_running() {
                result = manager.stop().await;
            }
            let worker_result = manager.stop_worker().await;
            result = result.and(worker_result);
        }
        for handle in self.event_handles.lock().await.drain(..) {
            result = result.and(handle.await.context("Node event listener failed"));
        }
        result
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
        let node = NodeContext::new(config_path, db_path, broadcaster).await?;
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

pub async fn shutdown_node() -> Result<()> {
    if let Some(node) = NODE.0.lock().await.as_ref() {
        node.shutdown().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use discovery::{server::UserStatus, utils::DeviceType};
    use tokio::{
        sync::mpsc,
        time::{Duration, timeout},
    };

    use super::{NodeBroadcaster, NodeContext};
    use crate::{
        messages::{LibraryContentChanged, TrustListUpdated},
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
        let node = NodeContext::new(&path, &path, Arc::new(Quiet(Some(tx)))).await?;
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
}
