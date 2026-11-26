use super::libraries::{cast_preflight, peer_access};
use super::sync_auth::SyncAuth;
use super::sync_worker::{authenticated_client, send_authenticated_request, wait_for_revocation};
use crate::backends::remote::{RemoteConnectError, token_error};
use database::actions::file::catalog_track;
use database::sync::model::RuneModel;
use std::net::TcpListener;
use std::sync::Mutex as StdMutex;
use std::{
    net::SocketAddr,
    path::Path,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time::Instant;
use tokio::{
    fs::{read_to_string, write},
    sync::{Mutex, RwLock, mpsc::unbounded_channel},
    task::JoinHandle,
};
use tokio_util::sync as tokio_util_sync;

use anyhow::{Context, Result};
use axum::{
    Extension, Router, middleware,
    routing::{delete, get, post, put},
};
use axum_server::{Handle, tls_rustls::RustlsConfig};
use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use database::actions::cover_art::COVER_TEMP_DIR;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use log::{error, info};
use playback::player::{RemoteAudioError, UnavailableReason};
use rand::{
    RngCore,
    distributions::{Alphanumeric, DistString},
    thread_rng,
};
use serde::{Deserialize, Serialize};
use tower_governor::{
    GovernorLayer, governor::GovernorConfigBuilder, key_extractor::PeerIpKeyExtractor,
};

use ::discovery::{DiscoveryParams, client::parse_certificate, ssl::generate_self_signed_cert};

use crate::{
    library_pool::LibraryHandle,
    messages::{LibraryContentChanged, PeerLibrary},
    node::NodeContext,
    server::{
        AppState, ServerState,
        http::{
            check_fingerprint::check_fingerprint_handler,
            device_info::device_info_handler,
            file::file_handler,
            list::list_users_handler,
            media::{get_cover_art_handler, get_media_metadata_handler},
            panel_alias::update_alias_handler,
            panel_auth_middleware::auth_middleware,
            panel_broadcast::toggle_broadcast_handler,
            panel_delete_user::delete_user_handler,
            panel_login::login_handler,
            panel_refresh::refresh_handler,
            panel_self::self_handler,
            panel_status::update_user_status_handler,
            ping::ping_handler,
            register::register_handler,
            websocket::websocket_handler,
        },
    },
};
use anyhow::Error;
use http_request::sync as http_request_sync;
use playback::player;
use reqwest::StatusCode;
use std::future;
use std::mem;
use std::pin;
use tokio::fs as tokio_fs;
use tokio::time as tokio_time;
use uuid::Uuid;

/// Asks the serving peer once per `interval` whether this device still has access to a remote
/// audio track, and ends playback only when the peer says the access was revoked. A peer that
/// cannot be reached (a network drop, a restart, a 5xx answer) is asked again at the next
/// interval, so audio that is already buffered keeps playing; a revocation that happens during
/// such an outage is caught by the first check that gets through. Returns when `cancel` is
/// cancelled, by this probe or by the check itself.
async fn run_access_probe<Check, Fut>(
    interval: Duration,
    mut check: Check,
    cancel: &tokio_util_sync::CancellationToken,
    failure_reason: &StdMutex<Option<UnavailableReason>>,
) where
    Check: FnMut() -> Fut,
    Fut: future::Future<Output = Result<()>>,
{
    let mut unreachable = false;
    loop {
        tokio_time::sleep(interval).await;
        match check().await {
            Ok(()) => {
                if mem::take(&mut unreachable) {
                    info!("Audio peer answers access checks again");
                }
            }
            Err(error) => {
                if cancel.is_cancelled() {
                    return;
                }
                if UnavailableReason::classify(&error) == UnavailableReason::AccessRevoked {
                    if let Ok(mut reason) = failure_reason.lock() {
                        *reason = Some(UnavailableReason::AccessRevoked);
                    }
                    cancel.cancel();
                    log::debug!("Audio library access revoked: {error:#}");
                    return;
                }
                if mem::replace(&mut unreachable, true) {
                    log::debug!("Audio library access check failed: {error:#}");
                } else {
                    log::warn!("Audio peer stopped answering access checks: {error:#}");
                }
            }
        }
    }
}

fn coalesce_worker_notify(
    notify: Arc<dyn Fn() + Send + Sync>,
    cancel: tokio_util_sync::CancellationToken,
) -> Arc<dyn Fn() + Send + Sync> {
    let (sender, mut receiver) = unbounded_channel();
    let worker_cancel = cancel.clone();
    tokio::spawn(async move {
        let mut pending = None;
        loop {
            let first = match pending.take() {
                Some(first) => first,
                None => tokio::select! {
                    _ = worker_cancel.cancelled() => return,
                    event = receiver.recv() => match event { Some(event) => event, None => return },
                },
            };
            let deadline = first + Duration::from_millis(500);
            tokio::select! {
                biased;
                _ = worker_cancel.cancelled() => return,
                _ = tokio_time::sleep_until(deadline) => {},
            }
            while let Ok(event) = receiver.try_recv() {
                if event >= deadline {
                    pending = Some(event);
                    break;
                }
            }
            notify();
        }
    });
    Arc::new(move || {
        if !cancel.is_cancelled() {
            let _ = sender.send(Instant::now());
        }
    })
}

#[derive(Debug, Serialize, Deserialize)]
struct JwtClaims {
    /// Expiration time (UNIX timestamp)
    exp: usize,
    /// Issued at time (UNIX timestamp)
    iat: usize,
}

impl JwtClaims {
    fn new(validity: Duration) -> Self {
        let now = SystemTime::now();
        let iat = now
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs() as usize;

        let exp = iat + validity.as_secs() as usize;

        Self { exp, iat }
    }
}

#[derive(Debug)]
pub struct ServerManager {
    node: Weak<NodeContext>,
    server_handle: Mutex<Option<JoinHandle<()>>>,
    start_lock: Mutex<()>,
    addr: Mutex<Option<SocketAddr>>,
    is_running: AtomicBool,
    sync_only: AtomicBool,
    shutdown_handle: Mutex<Option<Handle>>,
    sync_cancel: Mutex<Option<tokio_util_sync::CancellationToken>>,
    sync_handle: Mutex<Option<JoinHandle<()>>>,
    stream_handles: Mutex<Vec<JoinHandle<()>>>,
    sync_auth: Mutex<Option<SyncAuth>>,
    watch_cancel: Mutex<Option<tokio_util_sync::CancellationToken>>,
    watch_handle: Mutex<Option<JoinHandle<()>>>,
    certificate: String,
    private_key: String,
    jwt_secret: Vec<u8>,
}

impl ServerManager {
    /// Fetch the owned libraries advertised by a trusted paired device.
    pub async fn list_peer_libraries(
        &self,
        fingerprint: &str,
    ) -> Result<Vec<PeerLibrary>, RemoteConnectError> {
        let node = self.node().map_err(token_error)?;
        super::libraries::list_peer_libraries(&node, &self.private_key, fingerprint).await
    }

    pub fn node(&self) -> Result<Arc<NodeContext>> {
        self.node.upgrade().context("Node has shut down")
    }

    pub async fn new(node: Arc<NodeContext>) -> Result<Self> {
        let config_path = Path::new(&*node.config_path);

        let certificate_id = get_or_generate_alias(config_path).await?;

        let (_, certificate, private_key) =
            generate_or_load_certificates(config_path, &certificate_id)
                .await
                .context("Failed to initialize certificates")?;

        let jwt_secret = get_or_generate_jwt_secret(config_path)
            .await
            .context("Failed to initialize JWT secret")?;

        Ok(Self {
            node: Arc::downgrade(&node),
            server_handle: Mutex::new(None),
            start_lock: Mutex::new(()),
            addr: Mutex::new(None),
            is_running: AtomicBool::new(false),
            sync_only: AtomicBool::new(false),
            shutdown_handle: Mutex::new(None),
            sync_cancel: Mutex::new(None),
            sync_handle: Mutex::new(None),
            stream_handles: Mutex::new(Vec::new()),
            sync_auth: Mutex::new(None),
            watch_cancel: Mutex::new(None),
            watch_handle: Mutex::new(None),
            certificate,
            private_key,
            jwt_secret,
        })
    }

    pub async fn start(
        self: Arc<Self>,
        addr: SocketAddr,
        discovery_params: DiscoveryParams,
    ) -> Result<()> {
        let old_addr =
            if self.is_running.load(Ordering::SeqCst) && self.sync_only.load(Ordering::SeqCst) {
                let old_addr = self.get_address().await;
                self.stop().await?;
                old_addr
            } else {
                None
            };
        let fallback_info = discovery_params.device_info.clone();
        match self
            .clone()
            .start_inner(addr, discovery_params, false)
            .await
        {
            Ok(()) => Ok(()),
            Err(error) => {
                if let Some(old_addr) = old_addr
                    && let Err(restore_error) = self
                        .start_sync(
                            old_addr,
                            DiscoveryParams {
                                device_info: fallback_info,
                            },
                        )
                        .await
                {
                    log::error!("Could not restore paired sync service: {restore_error}");
                }
                Err(error)
            }
        }
    }
    pub fn start_sync(
        self: Arc<Self>,
        addr: SocketAddr,
        discovery_params: DiscoveryParams,
    ) -> pin::Pin<Box<dyn future::Future<Output = Result<()>> + Send>> {
        Box::pin(async move { self.start_inner(addr, discovery_params, true).await })
    }
    async fn start_inner(
        self: Arc<Self>,
        addr: SocketAddr,
        discovery_params: DiscoveryParams,
        sync_only: bool,
    ) -> Result<()> {
        let node = self.node()?;
        let _start_guard = self.start_lock.lock().await;
        anyhow::ensure!(!node.token.is_cancelled(), "node is shutting down");
        self.start_worker().await?;
        if self.is_running.load(Ordering::SeqCst) {
            return Err(anyhow::anyhow!("Server already running"));
        }
        let mut device_info = discovery_params.device_info.clone();
        let websocket_service = node.websocket_service.clone();

        let app_state = Arc::new(AppState {
            cover_temp_dir: COVER_TEMP_DIR.clone(),
        });

        let watch_cancel = tokio_util_sync::CancellationToken::new();
        let server_state = Arc::new(ServerState {
            node: Arc::downgrade(&node),
            session_cancel: watch_cancel.clone(),
            app_state: app_state.clone(),
            websocket_service: websocket_service.clone(),
            discovery_device_info: Arc::new(RwLock::new(discovery_params.device_info)),
            permission_manager: node.permission_manager.clone(),
            device_scanner: node.device_scanner.clone(),
        });

        let governor_conf = GovernorConfigBuilder::default()
            .per_second(60)
            .burst_size(5)
            .key_extractor(PeerIpKeyExtractor)
            .finish()
            .unwrap();

        let auth_routes: Router<Arc<ServerState>> = Router::new()
            .route("/panel/auth/login", post(login_handler))
            .layer(Extension(self.clone()));

        let protected_routes: Router<Arc<ServerState>> = Router::<Arc<ServerState>>::new()
            .route("/panel/self", get(self_handler))
            .route("/panel/broadcast", put(toggle_broadcast_handler))
            .route("/panel/alias", put(update_alias_handler))
            .route("/panel/auth/refresh", post(refresh_handler))
            .route("/panel/users", get(list_users_handler))
            .route("/panel/users/{fingerprint}", delete(delete_user_handler))
            .route(
                "/panel/users/{fingerprint}/status",
                put(update_user_status_handler),
            )
            .layer(middleware::from_fn(auth_middleware))
            .layer(Extension(self.clone()))
            .with_state(server_state.clone());

        let register_route = Router::new()
            .route("/register", post(register_handler))
            .layer(GovernorLayer {
                config: governor_conf.into(),
            });

        let node_id = Uuid::parse_str(&node.node_id)?;
        let (_, fingerprint) = parse_certificate(&self.certificate)?;
        let auth = {
            let mut sync_auth = self.sync_auth.lock().await;
            if let Some(auth) = sync_auth.as_ref() {
                auth.clone()
            } else {
                let auth = SyncAuth::new(node.permission_manager.clone(), fingerprint.clone());
                node.permission_manager
                    .write()
                    .await
                    .add_trust_change_hook(Arc::new(auth.trust_change_hook()));
                *sync_auth = Some(auth.clone());
                auth
            }
        };
        let sync_routes = Router::new()
            .nest(
                "/libraries/{library_id}",
                super::sync::pool_router(node.clone(), node_id),
            )
            .route("/libraries", get(super::libraries::list_libraries))
            .route("/access", get(peer_access))
            .route("/cast/preflight", post(cast_preflight))
            .layer(middleware::from_fn_with_state(
                auth.clone(),
                super::sync_auth::require_auth,
            ));
        let app = Router::new()
            .merge(register_route)
            .route("/ping", get(ping_handler))
            .route("/check-fingerprint", get(check_fingerprint_handler))
            .route("/device-info", get(device_info_handler));
        let library_routes = Router::new()
            .route("/files/{*file_path}", get(file_handler))
            .route("/media/metadata/{id}", get(get_media_metadata_handler))
            .route("/media/cover/{id}", get(get_cover_art_handler))
            .layer(middleware::from_fn_with_state(
                self.clone(),
                super::access::require_library_read,
            ))
            .layer(middleware::from_fn_with_state(
                auth.clone(),
                super::sync_auth::require_auth,
            ));
        let app = app.merge(library_routes);
        let app = if sync_only {
            app
        } else {
            let device_routes = Router::new().route("/ws", get(websocket_handler)).layer(
                middleware::from_fn_with_state(auth.clone(), super::sync_auth::require_auth),
            );
            app.merge(auth_routes)
                .merge(protected_routes)
                .merge(device_routes)
        };
        let app = app
            .with_state(server_state)
            .merge(auth.router())
            .merge(sync_routes)
            .layer(Extension(self.clone()));

        info!(
            "Temporary files path: {}",
            app_state.cover_temp_dir.to_string_lossy()
        );

        let handle = Handle::new();
        let shutdown_handle = handle.clone();

        let tls_config = {
            RustlsConfig::from_pem(
                self.certificate.as_bytes().to_vec(),
                self.private_key.as_bytes().to_vec(),
            )
            .await
            .context("Failed to create TLS configuration")?
        };

        let listener = TcpListener::bind(addr).context("Failed to bind sync server")?;
        let addr = listener.local_addr()?;
        device_info.api_port = addr.port();
        listener.set_nonblocking(true)?;
        let server_handle = tokio::spawn(async move {
            info!("Starting secure HTTPS/WSS server on {addr}");
            let server = axum_server::from_tcp_rustls(listener, tls_config)
                .handle(handle)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>());

            match server.await {
                Ok(_) => info!("Server stopped gracefully"),
                Err(e) => error!("Server error: {e}"),
            }
        });

        let watch_stop = watch_cancel.clone();
        let lifetime_handle = shutdown_handle.clone();
        let session_stop = watch_cancel.clone();
        let lifetime_token = node.token.clone();
        let lifetime_scanner = node.device_scanner.clone();
        *self.watch_handle.lock().await = Some(tokio::spawn(async move {
            tokio::select! {
                _ = watch_stop.cancelled() => {},
                _ = lifetime_token.cancelled() => {
                    session_stop.cancel();
                    lifetime_handle.graceful_shutdown(Some(Duration::from_secs(5)));
                    lifetime_scanner.stop_announcements().await;
                    lifetime_scanner.stop_listening().await;
                }
            }
        }));
        *self.watch_cancel.lock().await = Some(watch_cancel);
        if !node.device_scanner.is_listening().await
            && let Err(error) = node
                .device_scanner
                .start_listening(Some(fingerprint.clone()))
                .await
        {
            log::warn!("Could not start paired-device discovery: {error}");
        }
        if !node.device_scanner.is_announcing().await
            && let Err(error) = node
                .device_scanner
                .start_announcements(device_info, Duration::from_secs(3), None)
                .await
        {
            log::warn!("Could not announce paired-device service: {error}");
        }
        *self.server_handle.lock().await = Some(server_handle);
        *self.addr.lock().await = Some(addr);
        *self.shutdown_handle.lock().await = Some(shutdown_handle);
        self.sync_only.store(sync_only, Ordering::SeqCst);
        self.is_running.store(true, Ordering::SeqCst);
        if node.token.is_cancelled() {
            self.stop().await?;
            anyhow::bail!("node is shutting down");
        }
        Ok(())
    }

    pub async fn start_worker(self: &Arc<Self>) -> Result<()> {
        let node = self.node()?;
        let mut handle = self.sync_handle.lock().await;
        if handle.is_some() {
            return Ok(());
        }
        anyhow::ensure!(!node.token.is_cancelled(), "node is shutting down");
        let (_, fingerprint) = parse_certificate(&self.certificate)?;
        let weak = Arc::downgrade(&node);
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            if let Some(node) = weak.upgrade() {
                tokio::spawn(async move {
                    if let Ok(params) = node.current_ui().await
                        && let Some(library) = &params.library
                    {
                        node.broadcaster.broadcast(&LibraryContentChanged {
                            path: library.display_name.clone(),
                        });
                    }
                });
            }
        });
        let cancel = node.token.child_token();
        // Keep the worker future off nested library-startup stacks.
        let worker = Box::pin(super::sync_worker::run(
            Arc::downgrade(&node),
            coalesce_worker_notify(notify, cancel.clone()),
            fingerprint,
            self.private_key.clone(),
            cancel.clone(),
        ));
        *self.sync_cancel.lock().await = Some(cancel);
        *handle = Some(tokio::spawn(worker));
        Ok(())
    }

    pub async fn stop_worker(&self) -> Result<()> {
        let mut handle = self.sync_handle.lock().await;
        if let Some(cancel) = self.sync_cancel.lock().await.take() {
            cancel.cancel();
        }
        let mut first_error = None;
        if let Some(handle) = handle.take()
            && let Err(error) = handle.await
        {
            first_error = Some(Error::from(error));
        }
        let streams = mem::take(&mut *self.stream_handles.lock().await);
        for handle in streams {
            // A watcher whose task was already cancelled has nothing left to stop.
            if let Err(error) = handle.await
                && !error.is_cancelled()
            {
                first_error.get_or_insert_with(|| error.into());
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub async fn stop(&self) -> Result<()> {
        let node = self.node()?;
        if !self.is_running.load(Ordering::SeqCst) {
            return Err(anyhow::anyhow!("Server not running"));
        }

        let mut first_error = None;
        if let Some(cancel) = self.watch_cancel.lock().await.take() {
            cancel.cancel();
        }
        if let Some(handle) = self.watch_handle.lock().await.take()
            && let Err(error) = handle.await
        {
            log::error!("Watch task failed during shutdown: {error}");
            first_error = Some(Error::from(error));
        }
        if let Some(handle) = self.shutdown_handle.lock().await.as_ref() {
            handle.graceful_shutdown(Some(Duration::from_secs(5)));
        }

        if let Some(handle) = self.server_handle.lock().await.take()
            && let Err(error) = handle.await
        {
            log::error!("Server task failed during shutdown: {error}");
            first_error.get_or_insert_with(|| error.into());
        }

        node.device_scanner.stop_announcements().await;
        node.device_scanner.stop_listening().await;
        *self.addr.lock().await = None;
        *self.shutdown_handle.lock().await = None;
        self.is_running.store(false, Ordering::SeqCst);

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Locate an online paired replica and prepare a pinned authenticated audio stream.
    pub async fn resolve_remote_audio(
        &self,
        library: &LibraryHandle,
        lifetime: tokio_util_sync::CancellationToken,
        id: i32,
    ) -> Result<player::RemoteAudioSource> {
        let node = self.node()?;
        let (track_id, blob, extension) = catalog_track(&library.main_db, id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("track is not available for remote streaming"))?;
        let library_id = library.library_sync.identity().await?.namespace;
        let (_, own) = parse_certificate(&self.certificate)?;
        let entries = super::sync_worker::peer_endpoints(&node, &own).await;
        for hosts in entries {
            for endpoint in hosts {
                let fingerprint = endpoint.fingerprint;
                let host = endpoint.host;
                let trusted_host = endpoint.trusted_host;
                let port = endpoint.port;
                let result = async {
                    // Check access first: it renews a cached token the peer no longer knows.
                    let response = send_authenticated_request(
                        &node,
                        &own,
                        &self.private_key,
                        &fingerprint,
                        &host,
                        port,
                        |base, client| {
                            client
                                .head(format!("{}/sync/blobs/{}", super::sync::library_base(base, library_id), blob.digest))
                                .query(&[("track_id", track_id.to_string())])
                        },
                    )
                    .await
                    .map_err(RemoteAudioError::unreachable)?;
                    let (base, client) = authenticated_client(
                        &node,
                        &own,
                        &self.private_key,
                        &fingerprint,
                        &host,
                        port,
                    )
                    .await?;
                    let base = super::sync::library_base(&base, library_id);
                    let peer =
                        http_request_sync::MetadataPeer::<RuneModel>::new(&base, client.clone());
                    use sync::SyncPeer;
                    anyhow::ensure!(
                        peer.identity().await?.namespace == library_id,
                        "peer library mismatch"
                    );
                    let url = format!("{base}/sync/blobs/{}", blob.digest);
                    let access_url = format!("{url}?track_id={track_id}");
                    if matches!(response.status(), StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND) {
                        lifetime.cancel();
                        anyhow::bail!(RemoteAudioError::AccessRevoked);
                    }
                    if !response.status().is_success() {
                        anyhow::bail!(RemoteAudioError::unreachable(format!("unexpected status {}", response.status())));
                    }
                    let mut stream_handles = self.stream_handles.lock().await;
                    let cancellation = self.sync_cancel.lock().await.as_ref()
                        .ok_or_else(|| anyhow::anyhow!("node synchronization is closing"))?.child_token();
                    let library_cancel = lifetime.clone();
                    stream_handles.retain(|handle| !handle.is_finished());
                    let failure_reason = Arc::new(StdMutex::new(None));
                    let request = Arc::new(super::sync_worker::RenewableAudio {
                        global: Arc::downgrade(&node), own: own.clone(), private_key: self.private_key.clone(),
                        endpoint: super::sync_worker::PeerEndpoint { fingerprint: fingerprint.clone(), trusted_host: trusted_host.clone(), host: host.clone(), port },
                        url: url.clone(), access_url, size: blob.size,
                        client: Arc::new(Mutex::new(super::sync_worker::AudioCredentials { client, generation: 0 })), cancel: cancellation.clone(),
                        failure_reason: Some(failure_reason.clone()),
                    });
                    let watcher_failure_reason = failure_reason.clone();
                    let watch_cancel = cancellation.clone();
                    let watch_params = Arc::downgrade(&node);
                    let watch_fingerprint = fingerprint.clone();
                    let revocation_host = trusted_host.clone();
                    let watch_request = request.clone();
                    let watcher = tokio::spawn(async move {
                        // This device's own grant and trust changes end playback through
                        // `wait_for_revocation`, and the peer cuts an audio body still being
                        // downloaded when it revokes us. A track that is already fully buffered
                        // keeps no connection to the peer, and the peer has no push for its own
                        // revocation, so asking it is the only way to stop such a track.
                        let access_probe = run_access_probe(
                            Duration::from_secs(1),
                            || watch_request.check_library_access(),
                            &watch_cancel,
                            &watcher_failure_reason,
                        );
                        tokio::select! {
                            _ = watch_cancel.cancelled() => {},
                            _ = library_cancel.cancelled() => {
                                if let Ok(mut reason) = watcher_failure_reason.lock() { *reason = Some(UnavailableReason::SourceUnreachable); }
                                watch_cancel.cancel()
                            },
                            _ = wait_for_revocation(watch_params, &watch_fingerprint, &revocation_host) => {
                                if let Ok(mut reason) = watcher_failure_reason.lock() { *reason = Some(UnavailableReason::AccessRevoked); }
                                watch_cancel.cancel()
                            },
                            _ = access_probe => {},
                        }
                    });
                    stream_handles.push(watcher);
                    Ok::<_, anyhow::Error>(player::RemoteAudioSource {
                        request,
                        url,
                        extension: extension.clone(),
                        size: blob.size,
                        cancellation,
                        failure_reason,
                    })
                }
                .await;
                match result {
                    Ok(source) => return Ok(source),
                    Err(error)
                        if error.chain().any(|cause| {
                            cause
                                .downcast_ref::<RemoteAudioError>()
                                .is_some_and(|remote| {
                                    matches!(remote, RemoteAudioError::AccessRevoked)
                                })
                        }) =>
                    {
                        return Err(error);
                    }
                    Err(_) => {}
                }
            }
        }
        anyhow::bail!(RemoteAudioError::unreachable(
            "no paired device currently has this audio"
        ))
    }

    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    /// Whether the running listener is the sync-only service started for paired sync, rather
    /// than the full server started through `StartServerRequest`.
    pub fn is_sync_listener(&self) -> bool {
        self.is_running() && self.sync_only.load(Ordering::SeqCst)
    }

    pub async fn get_address(&self) -> Option<SocketAddr> {
        *self.addr.lock().await
    }

    pub fn generate_jwt_token(&self, validity: Option<Duration>) -> Result<String> {
        let validity = validity.unwrap_or(Duration::from_secs(7 * 24 * 60 * 60));
        let claims = JwtClaims::new(validity);

        Ok(encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(&self.jwt_secret),
        )?)
    }

    /// Whether `token` is a panel token this device issued and that has not expired.
    pub fn verify_jwt_token(&self, token: &str) -> bool {
        let mut validation = Validation::new(Algorithm::HS256);
        // The device that issues a token is the one that checks it, so there is no clock
        // difference to tolerate after the expiry.
        validation.leeway = 0;

        let Ok(token) = decode::<JwtClaims>(
            token,
            &DecodingKey::from_secret(&self.jwt_secret),
            &validation,
        ) else {
            return false;
        };
        // The library still accepts a token during the second its `exp` names. A token is
        // expired from that second on.
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .is_ok_and(|now| token.claims.exp as u64 > now.as_secs())
    }
}

pub async fn get_or_generate_alias(config_path: &Path) -> Result<String> {
    info!("Generating certificate in: {config_path:?}");

    let certificate_id_path = config_path.join("cid");
    if certificate_id_path.exists() {
        tokio_fs::read_to_string(&certificate_id_path)
            .await
            .map(|s| s.trim().to_string())
            .context("Failed to read cid file")
    } else {
        let certificate_id = generate_random_alias();
        tokio_fs::write(&certificate_id_path, &certificate_id)
            .await
            .context("Failed to save cid file")?;
        Ok(certificate_id)
    }
}

pub async fn update_alias(config_path: &Path, new_alias: &str) -> Result<()> {
    info!("Updating certificate alias in: {config_path:?} to: {new_alias}");

    let certificate_id_path = config_path.join("cid");

    tokio_fs::write(&certificate_id_path, new_alias)
        .await
        .context("Failed to write cid file")?;

    Ok(())
}

fn generate_random_alias() -> String {
    let mut rng = rand::thread_rng();
    let suffix: String = Alphanumeric.sample_string(&mut rng, 8);
    format!("R-{suffix}")
}

pub async fn generate_or_load_certificates<P: AsRef<Path>>(
    config_path: P,
    certificate_id: &str,
) -> Result<(String, String, String)> {
    let config_path: &Path = config_path.as_ref();

    let cert_path = config_path.join("certificate.pem");
    let key_path = config_path.join("private_key.pem");

    if cert_path.exists() && key_path.exists() {
        let cert = tokio_fs::read_to_string(&cert_path)
            .await
            .context("Failed to read certificate")?;
        let private_key = tokio_fs::read_to_string(&key_path)
            .await
            .context("Failed to read private key")?;

        let (_, fingerprint) = parse_certificate(&cert)?;
        Ok((fingerprint, cert, private_key))
    } else {
        let cert = generate_self_signed_cert(certificate_id, "Rune Player", "NET", 3650)?;

        tokio_fs::write(&cert_path, &cert.certificate)
            .await
            .context("Failed to save certificate")?;
        tokio_fs::write(&key_path, &cert.private_key)
            .await
            .context("Failed to save private key")?;

        let fingerprint = cert.public_key_fingerprint;
        Ok((fingerprint, cert.certificate, cert.private_key))
    }
}

async fn get_or_generate_jwt_secret<P: AsRef<Path>>(config_path: P) -> Result<Vec<u8>> {
    let config_path: &Path = config_path.as_ref();

    let secret_path = config_path.join("jwt_secret.key");

    if secret_path.exists() {
        let base64_secret = read_to_string(&secret_path)
            .await
            .context("Failed to read JWT secret file")?;
        URL_SAFE
            .decode(base64_secret.trim())
            .context("Failed to decode JWT secret")
    } else {
        let mut secret = vec![0u8; 32];
        thread_rng().fill_bytes(&mut secret);

        let base64_secret = URL_SAFE.encode(&secret);
        write(&secret_path, base64_secret)
            .await
            .context("Failed to save JWT secret")?;

        Ok(secret)
    }
}

pub async fn update_root_password<P: AsRef<Path>>(config_path: P, password: &str) -> Result<()> {
    let config_path: &Path = config_path.as_ref();

    let password_path = config_path.join("root_password.hash");

    if password_path.exists() {
        return Err(anyhow::anyhow!("Root password already initialized"));
    }

    let hash = bcrypt::hash(password, bcrypt::DEFAULT_COST).context("Failed to hash password")?;

    tokio_fs::write(&password_path, &hash)
        .await
        .context("Failed to write password hash")?;

    Ok(())
}

#[cfg(test)]
mod lifecycle_tests {
    use super::super::sync_worker;
    use super::super::sync_worker::share_worker_tests::{TestPeerSession, test_peer_pass};
    use super::super::sync_worker::{PeerEndpoint, PeerPass, authenticated_client};
    use super::super::utils::device::RunMode;
    use crate::Signal;
    use crate::macros::{RequestScope, dispatch_remote, request_scope};
    use crate::messages::*;
    use crate::request_error::classify;
    use crate::server::WebSocketService;
    use crate::utils::{ParamsExtractor, register_local_library_on_node};
    use ::discovery::client::CertValidator;
    use anyhow::Error;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use discovery::capabilities;
    use discovery::endpoint;
    use discovery::protocol_version;
    use discovery::utils;
    use metadata::cover_art;
    use migration::catalog_identity;
    use migration::catalog_identity::{ParentKind, parent};
    use playback::decoder::RuneDecoder;
    use playback::stream_utils::{ManagedStream, create_stream_with_authorizer};
    use reqwest::Client;
    use reqwest::StatusCode;
    use reqwest::header;
    use std::fs;
    use std::io::ErrorKind;
    use std::net::SocketAddr;
    use std::net::TcpListener;
    use std::path::Path;
    use tokio::net::TcpSocket;
    use tokio::task;
    use tokio::time as tokio_time;
    use tracing_subscriber::filter::LevelFilter;
    use url::Url;
    use visualizer::Visualizer;

    #[tokio::test(start_paused = true)]
    async fn access_probe_keeps_buffered_audio_through_unreachable_peer_and_stops_on_revocation() {
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        };
        use std::time::Duration;
        let outcomes: Vec<fn() -> anyhow::Result<()>> = vec![
            || Ok(()),
            || Err(RemoteAudioError::unreachable("connection refused").into()),
            || Err(RemoteAudioError::unreachable("unexpected status 503").into()),
            || Err(anyhow::anyhow!("peer closed the connection")),
            || Ok(()),
            || Err(RemoteAudioError::unreachable("unexpected status 503").into()),
            || Err(RemoteAudioError::AccessRevoked.into()),
        ];
        let checks = Arc::new(AtomicUsize::new(0));
        let cancel = CancellationToken::new();
        let reason = Arc::new(Mutex::new(None));
        let probe = super::run_access_probe(
            Duration::from_secs(1),
            || {
                // Until the revocation is returned, an unreachable peer must not stop playback.
                assert!(
                    !cancel.is_cancelled(),
                    "playback stopped before the revocation"
                );
                assert_eq!(*reason.lock().unwrap(), None);
                let index = checks.fetch_add(1, Ordering::SeqCst);
                let outcome = outcomes[index]();
                async move { outcome }
            },
            &cancel,
            &reason,
        );
        tokio_time::timeout(Duration::from_secs(60), probe)
            .await
            .expect("the probe must stop after the revocation");
        assert!(cancel.is_cancelled());
        assert_eq!(
            *reason.lock().unwrap(),
            Some(UnavailableReason::AccessRevoked)
        );
        assert_eq!(checks.load(Ordering::SeqCst), 7);
    }

    #[tokio::test(start_paused = true)]
    async fn worker_notifications_are_coalesced() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use std::time::Duration;
        let count = Arc::new(AtomicUsize::new(0));
        let cancel = CancellationToken::new();
        let notify = super::coalesce_worker_notify(
            Arc::new({
                let count = count.clone();
                move || {
                    count.fetch_add(1, Ordering::SeqCst);
                }
            }),
            cancel.clone(),
        );
        for _ in 0..100 {
            notify();
            tokio_time::advance(Duration::from_millis(1)).await;
        }
        tokio_time::advance(Duration::from_millis(500)).await;
        task::yield_now().await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        notify();
        tokio_time::advance(Duration::from_millis(500)).await;
        task::yield_now().await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
        cancel.cancel();
    }

    use super::super::libraries::ListedLibrary;
    use super::*;
    use crate::utils::nid::get_or_create_node_id;
    use crate::utils::{Broadcaster, GlobalParams, RinfRustSignal, sync_settings::SyncSettings};
    use database::actions::downloads::{prepare_download, set_download_rule};
    use database::actions::file::{attach_download, catalog_track, hash_file, is_materialized};
    use database::actions::metadata::{
        MissingHostFilePolicy, ScanOptions, scan_audio_file, scan_audio_library,
    };
    use database::connection::StorageMode;
    use database::credentials::CredentialRef;
    use database::entities::{media_files, media_files::Column};
    use database::library_registry::{
        CatalogPlacement, FsAdapter, LibraryRecord, LibraryRecordId, LibraryRegistry,
        ProtocolAdapter, RuneForwardMode, SubsonicOpenMode, create_replica, resolve_library,
    };
    use database::sync::{
        LibrarySyncStore,
        model::{Album, Collection, MetadataTag, ObjectKey, Payload, Track},
    };
    use discovery::{
        capabilities::Capabilities,
        server::{PermissionManager, UserStatus},
        utils::DeviceType,
    };
    use playback::sfx_player::SfxPlayer;
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
    use std::future::Future;
    use std::{ops::Deref, sync::OnceLock, thread};
    use tokio::net::TcpStream;
    use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    use crate::backends::remote::encode_message;
    use crate::backends::remote::{
        RemoteConnectError, connect_controller, connect_remote, decode_message,
    };
    use crate::handlers::neighbors::select_pinned_pair_host;
    use crate::library_pool::{LibraryHandle, LibraryPool};
    use crate::messages::{
        DeviceConnectionLevel, DeviceSyncLevel, LibraryInitializeMode, OpenLibraryRequest,
        PlaybackState, PlaybackStatus, RegisterLocalLibraryRequest, RegisterRemoteControlRequest,
        RequestErrorKind, RequestFailureCause, TrustListUpdated, TrustedServerCertificate,
    };
    use crate::node::{NodeSlot, ReconnectConfig};
    use crate::request_error::{ExpectedRequestError, request_failure_cause};
    use crate::server::api::{add_trusted_server, edit_trusted_hosts};
    use crate::server::sync::client::sync_library;
    use crate::test_wait::{TEST_DEADLINE, wait_until};
    use database::sync::model::RuneModel;
    use futures::{SinkExt, StreamExt};
    use reqwest::header::CONNECTION;
    use sync::{CheckpointKey, ReceiveStore, Scope, SyncModel, scope_id};
    use tokio_tungstenite::tungstenite::Message;

    include!("access_policy_tests.rs");
    mod player_session_tests {
        use super::*;
        include!("player_session_tests.rs");
        include!("handoff_reconcile_tests.rs");
    }

    mod gui_listener_tests {
        include!("gui_listener_tests.rs");
    }

    struct Quiet;
    impl Broadcaster for Quiet {
        fn broadcast(&self, _: &dyn RinfRustSignal) {}
    }

    struct Fixture {
        manager: Arc<ServerManager>,
        global_params: Arc<GlobalParams>,
    }

    async fn open_remote_control_record(
        node: Arc<NodeContext>,
        fingerprint: &str,
    ) -> Result<Arc<GlobalParams>> {
        use crate::utils::register_remote_control_on_node;

        let registered = register_remote_control_on_node(
            &RegisterRemoteControlRequest {
                call_id: None,
                fingerprint: fingerprint.to_owned(),
                alias: "Test remote".into(),
            },
            &node,
        )
        .await?;
        let record = registered.record.context("remote record")?;
        open_registered_library_record(node, record.record_id).await
    }

    async fn open_registered_library_record(
        node: Arc<NodeContext>,
        record_id: String,
    ) -> Result<Arc<GlobalParams>> {
        use crate::utils::open_library_by_record_on_node;
        use scrobbling::manager::ScrobblingManager;
        use tokio::sync::Mutex;

        let opened = open_library_by_record_on_node(
            &OpenLibraryRequest {
                call_id: Some(1),
                record_id: record_id.clone(),
            },
            node.clone(),
            Arc::new(Mutex::new(ScrobblingManager::new(
                10,
                Duration::from_secs(5),
            ))),
        )
        .await?;
        anyhow::ensure!(opened.record_id == record_id, "opened another record");
        node.current_ui().await
    }

    async fn peer_step<T>(
        name: &'static str,
        future: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        peer_step_with_timeout(name, TEST_DEADLINE, future).await
    }

    /// A diagnostic watchdog: it names the step a hung test is stuck in. The limit is far
    /// above any normal step, even on a loaded host, and still well below the suite limit.
    async fn step<T>(name: &'static str, future: impl Future<Output = T>) -> T {
        step_with_timeout(name, TEST_DEADLINE, future).await
    }

    async fn step_with_timeout<T>(
        name: &'static str,
        limit: Duration,
        future: impl Future<Output = T>,
    ) -> T {
        // The test thread carries the test's name, so interleaved --nocapture output stays attributable.
        let test = thread::current().name().unwrap_or("?").to_owned();
        eprintln!("test step started: {test}: {name}");
        match tokio_time::timeout(limit, future).await {
            Ok(result) => {
                eprintln!("test step finished: {test}: {name}");
                result
            }
            Err(_) => panic!("test step did not finish: {test}: {name}"),
        }
    }

    async fn peer_step_with_timeout<T>(
        name: &'static str,
        limit: Duration,
        future: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        tokio_time::timeout(limit, future)
            .await
            .with_context(|| format!("peer step timed out: {name}"))?
    }

    /// Anonymous client that trusts the certificates `params` has pinned.
    /// `max_idle_per_host = Some(0)` disables connection reuse.
    async fn pinned_anonymous_client(
        params: &GlobalParams,
        max_idle_per_host: Option<usize>,
    ) -> Result<reqwest::Client> {
        let tls = Arc::new(params.cert_validator.read().await.clone()).into_client_config();
        let mut builder = Client::builder().use_preconfigured_tls(tls).no_proxy();
        if let Some(max_idle) = max_idle_per_host {
            builder = builder.pool_max_idle_per_host(max_idle);
        }
        Ok(builder.build()?)
    }
    impl Deref for Fixture {
        type Target = Arc<ServerManager>;
        fn deref(&self) -> &Self::Target {
            &self.manager
        }
    }

    async fn fixture(dir: &Path) -> Result<Fixture> {
        fixture_with_library(dir, Uuid::new_v4()).await
    }

    async fn fixture_with_library(dir: &Path, library_id: Uuid) -> Result<Fixture> {
        fixture_with_library_timeout(dir, library_id, Duration::from_secs(300)).await
    }

    async fn fixture_with_library_timeout(
        dir: &Path,
        library_id: Uuid,
        idle_timeout: Duration,
    ) -> Result<Fixture> {
        fixture_with_broadcaster(dir, library_id, idle_timeout, Arc::new(Quiet)).await
    }

    async fn fixture_with_broadcaster(
        dir: &Path,
        library_id: Uuid,
        idle_timeout: Duration,
        broadcaster: Arc<dyn Broadcaster>,
    ) -> Result<Fixture> {
        fixture_with_reconnect_config(
            dir,
            library_id,
            idle_timeout,
            broadcaster,
            ReconnectConfig::default(),
        )
        .await
    }

    async fn fixture_with_reconnect_config(
        dir: &Path,
        library_id: Uuid,
        idle_timeout: Duration,
        broadcaster: Arc<dyn Broadcaster>,
        reconnect_config: ReconnectConfig,
    ) -> Result<Fixture> {
        fixture_with_player(
            dir,
            library_id,
            idle_timeout,
            broadcaster,
            reconnect_config,
            RunMode::App,
            crate::session::PlayerSession::real_player(),
        )
        .await
    }

    /// A fixture whose node runs in `run_mode` and plays through the player `wrap_player`
    /// makes of the real one.
    async fn fixture_with_player(
        dir: &Path,
        library_id: Uuid,
        idle_timeout: Duration,
        broadcaster: Arc<dyn Broadcaster>,
        reconnect_config: ReconnectConfig,
        run_mode: RunMode,
        wrap_player: crate::session::PlayerWrap,
    ) -> Result<Fixture> {
        use database::credentials::CredentialStore;

        fs::create_dir_all(dir.join(".rune"))?;
        let node = get_or_create_node_id(dir.to_str().context("config path")?).await?;
        let credentials = Arc::new(CredentialStore::open(dir));
        let pool = LibraryPool::with_idle_timeout_and_credentials(
            dir,
            dir,
            &node.to_string(),
            idle_timeout,
            credentials,
        )?;
        let _ = migration::initialize_node_id(node.to_string());
        LibraryRegistry::update(dir, |registry| {
            registry.libraries.push(LibraryRecord {
                record_id: LibraryRecordId(Uuid::new_v4()),
                alias: "Owned".into(),
                library_id: Some(library_id),
                protocol: ProtocolAdapter::Native,
                fs: vec![FsAdapter::Local {
                    path: dir.to_string_lossy().into_owned(),
                }],
                catalog: CatalogPlacement::InMediaRoot,
                background_sync: true,
            });
            Ok(())
        })
        .await?;
        let library = pool.acquire_ui(library_id).await?;
        crate::test_node::disable_sync_listener(dir)?;
        let node_context = NodeContext::with_reconnect_config(
            dir.to_str().context("config path")?,
            dir.to_str().context("db path")?,
            broadcaster,
            false,
            run_mode,
            reconnect_config,
            wrap_player,
            Arc::new(Visualizer::new()),
        )
        .await?;
        fixture_from_library(node_context, library).await
    }

    async fn fixture_open_record(source: &Fixture, record_id: String) -> Result<Fixture> {
        let node = source.global_params.node.clone();
        let params = open_registered_library_record(node, record_id).await?;
        fixture_from_params(params)
    }

    async fn fixture_open_local_library(source: &Fixture, path: &Path) -> Result<Fixture> {
        let node = source.global_params.node.clone();
        let response = register_local_library_on_node(
            &RegisterLocalLibraryRequest {
                call_id: None,
                path: path.to_str().context("test library path")?.to_owned(),
                alias: "Test local".into(),
                mode: Some(LibraryInitializeMode::Portable),
            },
            &node,
        )
        .await?;
        let record = response.record.context("registered local record")?;
        fixture_open_record(source, record.record_id).await
    }

    async fn fixture_open_replica(source: &Fixture, library_id: Uuid) -> Result<Fixture> {
        let node = source.global_params.node.clone();
        let registry = LibraryRegistry::load(Path::new(&*node.config_path))?;
        let record_id = registry.catalog(library_id)?.record_id.0.to_string();
        fixture_open_record(source, record_id).await
    }

    async fn reopen_fixture(original: &Fixture) -> Result<Fixture> {
        original.global_params.node.shutdown().await?;
        let config = &original.global_params.config_path;
        let node = NodeContext::new(
            config,
            config,
            Arc::new(Quiet),
            false,
            RunMode::App,
            Arc::new(Visualizer::new()),
        )
        .await?;
        let library = node
            .library_pool
            .acquire_ui(original.global_params.library_id.context("library")?)
            .await?;
        fixture_from_library(node, library).await
    }

    async fn fixture_from_library(
        node_context: Arc<NodeContext>,
        library: LibraryHandle,
    ) -> Result<Fixture> {
        let token = Arc::new(node_context.token.child_token());
        let params = Arc::new(GlobalParams {
            node: node_context.clone(),
            library_id: Some(library.library_id),
            library_pool: node_context.library_pool.clone(),
            sync_settings: node_context.sync_settings.clone(),
            config_path: node_context.config_path.clone(),
            node_id: node_context.node_id.clone(),
            library: Some(library.clone()),
            ui_library: Arc::new(Mutex::new(Some(library))),
            main_token: token.clone(),
            library_tasks: node_context.library_tasks.clone(),
            player: node_context.session()?.player.clone(),
            sfx_player: Arc::new(Mutex::new(SfxPlayer::new(Some((*token).clone())))),
            scrobbler: node_context.session()?.scrobbler.clone(),
            broadcaster: node_context.broadcaster.clone(),
            device_scanner: node_context.device_scanner.clone(),
            cert_validator: node_context.cert_validator.clone(),
            permission_manager: node_context.permission_manager.clone(),
            server_manager: OnceLock::new(),
        });
        let manager = node_context.manager()?;
        node_context.attach_ui(&params).await;
        params
            .server_manager
            .set(manager.clone())
            .expect("manager set once");
        Ok(Fixture {
            manager,
            global_params: params,
        })
    }

    fn fixture_from_params(global_params: Arc<GlobalParams>) -> Result<Fixture> {
        let manager = global_params.node.manager()?;
        Ok(Fixture {
            manager,
            global_params,
        })
    }
    #[tokio::test]
    async fn request_gate_server_dispatch_rejects_local_requests() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let manager = fixture(dir.path()).await?;
        let service = Arc::new(WebSocketService::new());
        assert!(
            service
                .handle_message(
                    "FetchRemoteFileRequest",
                    Vec::new(),
                    None,
                    &manager.global_params.node
                )
                .await
                .is_none()
        );
        let payload = rinf::serialize(&GetMediaFilesCountRequest { call_id: None })?;
        let library_id = manager
            .global_params
            .library_id
            .context("current library")?;
        let (name, response) = service
            .handle_addressed_message(
                "GetMediaFilesCountRequest",
                payload,
                None,
                Some(library_id),
                &manager.global_params.node,
            )
            .await
            .context("forwardable request must be dispatched")?;
        assert_eq!(name, "GetMediaFilesCountResponse");
        let response: GetMediaFilesCountResponse = rinf::deserialize(&response?)?;
        assert_eq!(response.count, 0);
        let params = &manager.global_params;
        let library = params
            .library_pool
            .acquire(params.library_id.unwrap())
            .await?;
        assert!(Arc::ptr_eq(
            &library.main_db,
            &params.test_library().main_db
        ));
        assert!(params.ui_library.lock().await.is_some());
        let close = CloseLibraryRequest { call_id: None };
        assert!(
            close
                .handle(close.extract_params(params), None, &close)
                .await?
                .is_some()
        );
        assert!(params.ui_library.lock().await.is_none());
        assert!(params.main_token.is_cancelled());
        Ok(())
    }

    #[tokio::test]
    async fn closing_a_replica_closes_the_open_record() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let source = fixture(dir.path()).await?;
        let config = Path::new(&*source.global_params.config_path);
        let id = Uuid::new_v4();
        let mut registry = LibraryRegistry::load(config)?;
        create_replica(config, &mut registry, id, "Replica", "owner").await?;
        let opened = fixture_open_replica(&source, id).await?;
        let params = &opened.global_params;
        assert_eq!(params.library_id, Some(id));
        let close = CloseLibraryRequest { call_id: None };
        assert!(
            close
                .handle(close.extract_params(params), None, &close)
                .await?
                .is_some()
        );
        assert!(params.main_token.is_cancelled());
        source.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn request_gate_owner_requests_are_not_forwardable() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let peer = fixture(dir.path()).await?;
        for name in [
            "ListLocalLibrariesRequest",
            "SetLibraryBackgroundSyncRequest",
            "ListPeerLibrariesRequest",
            "AddRemoteLibraryRequest",
            "ListControllersRequest",
            "DetachControllerRequest",
            "UpdateClientStatusRequest",
            "AuthenticateSingleServiceRequest",
            "AuthenticateMultipleServiceRequest",
            "LogoutSingleServiceRequest",
        ] {
            assert!(
                dispatch_remote(name, &[], None, None, &peer.global_params.node)
                    .await
                    .is_none(),
                "{name} must remain local"
            );
        }
        peer.global_params.node.shutdown().await?;
        Ok(())
    }

    async fn info(manager: &Fixture) -> Result<utils::DeviceInfo> {
        manager.global_params.node.device_info().await
    }

    async fn start_test_tls_server(fixture: &Fixture) -> Result<(SocketAddr, String)> {
        let device_info = info(fixture).await?;
        fixture
            .manager
            .clone()
            .start(([127, 0, 0, 1], 0).into(), DiscoveryParams { device_info })
            .await?;
        let address = fixture
            .manager
            .get_address()
            .await
            .context("server address")?;
        Ok((address, info(fixture).await?.fingerprint))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn add_trusted_server_accepts_matching_certificate_and_stores_host() -> Result<()> {
        let server_dir = tempfile::tempdir()?;
        let client_dir = tempfile::tempdir()?;
        let server = fixture(server_dir.path()).await?;
        let client = fixture(client_dir.path()).await?;
        let (address, fingerprint) = start_test_tls_server(&server).await?;
        let validator = client.global_params.cert_validator.read().await.clone();
        let host = format!("127.0.0.1:{}", address.port());
        let unreachable_host = "127.0.0.1:1".to_owned();

        add_trusted_server(
            &validator,
            Some(&TrustedServerCertificate {
                fingerprint: fingerprint.clone(),
                hosts: vec![unreachable_host.clone(), host.clone()],
            }),
        )
        .await?;

        let stored_hosts = validator.get_hosts_for_fingerprint(&fingerprint).await;
        assert!(stored_hosts.contains(&host));
        assert!(stored_hosts.contains(&unreachable_host));
        server.manager.stop().await?;
        Ok(())
    }

    async fn paired_servers(
        client: &Fixture,
    ) -> Result<Vec<(String, String, String, &'static str)>> {
        let request = ListPairedServersRequest { call_id: None };
        let servers = request
            .handle(
                request.extract_params(&client.global_params),
                None,
                &request,
            )
            .await?
            .context("paired servers response")?
            .servers;
        Ok(servers
            .into_iter()
            .map(|server| {
                let status = match server.status {
                    ClientStatus::Approved => "Approved",
                    ClientStatus::Pending => "Pending",
                    ClientStatus::Blocked => "Blocked",
                };
                (
                    server.alias,
                    server.fingerprint,
                    server.device_model,
                    status,
                )
            })
            .collect())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn paired_servers_list_each_server_with_the_status_it_reports() -> Result<()> {
        let server_dir = tempfile::tempdir()?;
        let client_dir = tempfile::tempdir()?;
        let server = step("server fixture", fixture(server_dir.path())).await?;
        let client = step("client fixture", fixture(client_dir.path())).await?;
        let (address, server_fp) = step("start server", start_test_tls_server(&server)).await?;
        let server_info = info(&server).await?;
        let client_info = info(&client).await?;
        let host = format!("127.0.0.1:{}", address.port());

        let trust = AddTrustedServerRequest {
            certificate: Some(TrustedServerCertificate {
                fingerprint: server_fp.clone(),
                hosts: vec![host.clone()],
            }),
            call_id: None,
        };
        step(
            "trust the server",
            trust.handle(trust.extract_params(&client.global_params), None, &trust),
        )
        .await?;
        let register = RegisterDeviceOnServerRequest {
            alias: client_info.alias.clone(),
            hosts: vec![host.clone()],
            fingerprint: server_fp.clone(),
            call_id: None,
        };
        step(
            "register on the server",
            register.handle(
                register.extract_params(&client.global_params),
                None,
                &register,
            ),
        )
        .await?;
        assert_eq!(
            client
                .global_params
                .cert_validator
                .read()
                .await
                .get_hosts_for_fingerprint(&server_fp)
                .await,
            [host]
        );
        let registered = server
            .global_params
            .permission_manager
            .read()
            .await
            .verify_by_fingerprint(&client_info.fingerprint)
            .await
            .context("the server holds the client")?;
        assert_eq!(registered.alias, client_info.alias);
        assert_eq!(registered.status, UserStatus::Pending);

        let device_model = server_info
            .device_model
            .clone()
            .context("the server reports its device model")?;
        let listed = |status| {
            vec![(
                server_info.alias.clone(),
                server_fp.clone(),
                device_model.clone(),
                status,
            )]
        };
        assert_eq!(
            step("list while pending", paired_servers(&client)).await?,
            listed("Pending")
        );

        server
            .global_params
            .permission_manager
            .write()
            .await
            .change_user_status(&client_info.fingerprint, UserStatus::Approved)
            .await?;
        assert_eq!(
            step("list once approved", paired_servers(&client)).await?,
            listed("Approved")
        );

        server
            .global_params
            .permission_manager
            .write()
            .await
            .change_user_status(&client_info.fingerprint, UserStatus::Blocked)
            .await?;
        assert_eq!(
            step("list once blocked", paired_servers(&client)).await?,
            listed("Blocked")
        );

        let client_permissions = client.global_params.permission_manager.write().await;
        client_permissions
            .add_user(
                server.certificate.clone(),
                server_fp.clone(),
                server_info.alias.clone(),
                "test".into(),
                DeviceType::Desktop,
                "127.0.0.1".into(),
            )
            .await?;
        client_permissions
            .change_user_status(&server_fp, UserStatus::Blocked)
            .await?;
        drop(client_permissions);
        let refused = step("list while refused here", paired_servers(&client)).await?;
        assert!(refused.is_empty(), "{refused:?}");

        client
            .global_params
            .permission_manager
            .write()
            .await
            .remove_user(&server_fp)
            .await?;
        assert_eq!(
            step("list once no longer refused", paired_servers(&client)).await?,
            listed("Blocked")
        );
        server.manager.stop().await?;
        let unreachable = step("list while unreachable", paired_servers(&client)).await?;
        assert!(unreachable.is_empty(), "{unreachable:?}");

        server.global_params.node.shutdown().await?;
        client.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pinned_pair_host_reports_a_certificate_mismatch() -> Result<()> {
        let server_dir = tempfile::tempdir()?;
        let client_dir = tempfile::tempdir()?;
        let server = fixture(server_dir.path()).await?;
        let client = fixture(client_dir.path()).await?;
        let (address, _) = start_test_tls_server(&server).await?;
        let host = format!("127.0.0.1:{}", address.port());
        let validator = client.global_params.cert_validator.read().await.clone();

        let error = select_pinned_pair_host(vec![host], &validator, "a different fingerprint")
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::CertificateMismatch)
        ));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn add_trusted_server_rejects_mismatch_with_typed_failure() -> Result<()> {
        let server_dir = tempfile::tempdir()?;
        let client_dir = tempfile::tempdir()?;
        let server = fixture(server_dir.path()).await?;
        let client = fixture(client_dir.path()).await?;
        let (address, _) = start_test_tls_server(&server).await?;
        let validator = client.global_params.cert_validator.read().await.clone();
        let host = format!("127.0.0.1:{}", address.port());
        let error = add_trusted_server(
            &validator,
            Some(&TrustedServerCertificate {
                fingerprint: "different fingerprint".into(),
                hosts: vec![host],
            }),
        )
        .await
        .expect_err("mismatched certificate must be refused");

        assert_eq!(classify(&error), RequestErrorKind::InvalidInput);
        assert_eq!(
            request_failure_cause(&error),
            Some(RequestFailureCause::CertificateMismatch)
        );
        assert!(validator.list_trusted_fingerprints().await.is_empty());
        server.manager.stop().await?;
        Ok(())
    }

    #[tokio::test]
    async fn add_trusted_server_rejects_missing_certificate_as_invalid_input() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let validator = CertValidator::new(dir.path()).await?;
        let error = add_trusted_server(&validator, None)
            .await
            .expect_err("missing certificate must be refused");
        assert_eq!(classify(&error), RequestErrorKind::InvalidInput);
        Ok(())
    }

    #[tokio::test]
    async fn add_trusted_server_rejects_when_no_host_is_reachable() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let client = step("fixture", fixture(dir.path())).await?;
        let validator = client.global_params.cert_validator.read().await.clone();
        let error = step(
            "verify unreachable certificate host",
            add_trusted_server(
                &validator,
                Some(&TrustedServerCertificate {
                    fingerprint: "expected fingerprint".into(),
                    hosts: vec!["127.0.0.1:1".into()],
                }),
            ),
        )
        .await
        .expect_err("unreachable hosts must be refused");
        assert_eq!(classify(&error), RequestErrorKind::Unavailable);
        assert!(validator.list_trusted_fingerprints().await.is_empty());
        step(
            "shutdown client fixture",
            client.global_params.node.shutdown(),
        )
        .await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn edit_hosts_refuses_untrusted_fingerprint() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let client = fixture(dir.path()).await?;
        let validator = client.global_params.cert_validator.read().await.clone();
        let error = edit_trusted_hosts(&validator, "unknown fingerprint", vec![])
            .await
            .expect_err("untrusted fingerprint must be refused");
        assert_eq!(classify(&error), RequestErrorKind::InvalidInput);
        assert!(validator.list_trusted_fingerprints().await.is_empty());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn edit_hosts_refuses_host_with_different_certificate() -> Result<()> {
        let trusted_dir = tempfile::tempdir()?;
        let other_dir = tempfile::tempdir()?;
        let client_dir = tempfile::tempdir()?;
        let trusted = fixture(trusted_dir.path()).await?;
        let other = fixture(other_dir.path()).await?;
        let client = fixture(client_dir.path()).await?;
        let (trusted_address, trusted_fingerprint) = start_test_tls_server(&trusted).await?;
        let (other_address, _) = start_test_tls_server(&other).await?;
        let validator = client.global_params.cert_validator.read().await.clone();
        let original_host = format!("127.0.0.1:{}", trusted_address.port());
        validator
            .add_trusted_domains([original_host.clone()], &trusted_fingerprint)
            .await?;
        let error = edit_trusted_hosts(
            &validator,
            &trusted_fingerprint,
            vec![format!("127.0.0.1:{}", other_address.port())],
        )
        .await
        .expect_err("different certificate must be refused");

        assert_eq!(classify(&error), RequestErrorKind::InvalidInput);
        assert_eq!(
            request_failure_cause(&error),
            Some(RequestFailureCause::CertificateMismatch)
        );
        assert_eq!(
            validator
                .get_hosts_for_fingerprint(&trusted_fingerprint)
                .await,
            [original_host]
        );
        trusted.manager.stop().await?;
        other.manager.stop().await?;
        Ok(())
    }

    /// Deliver a discovery announcement for `info` at `addr` to `receiver`,
    /// as a multicast datagram would.
    async fn announce_peer(
        receiver: &Fixture,
        info: &utils::DeviceInfo,
        addr: SocketAddr,
    ) -> Result<()> {
        let announcement = serde_json::json!({
            "fingerprint": info.fingerprint, "alias": info.alias,
            "deviceModel": info.device_model, "deviceType": info.device_type,
            "api_port": addr.port(), "announce": true,
        });
        receiver
            .global_params
            .device_scanner
            .observe_announcement(&serde_json::to_vec(&announcement)?, addr, None)
            .await
    }

    #[tokio::test]
    async fn device_info_returns_rune_capabilities() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let client_dir = tempfile::tempdir()?;
        let manager = fixture(dir.path()).await?;
        let client_fixture = fixture(client_dir.path()).await?;
        let addr = paired_full(&manager, &client_fixture).await?;
        let client = pinned_anonymous_client(&client_fixture.global_params, Some(0)).await?;

        let response = client
            .get(format!("https://{addr}/device-info"))
            .send()
            .await?
            .error_for_status()?;
        let body: serde_json::Value = response.json().await?;

        assert!(body.get("active_library_id").is_none());
        let capabilities: capabilities::Capabilities =
            serde_json::from_value(body["capabilities"].clone())?;
        assert_eq!(capabilities, Capabilities::rune_native());
        Ok(())
    }

    #[tokio::test]
    async fn authenticated_access_and_cast_preflight_require_control() -> Result<()> {
        let server_dir = tempfile::tempdir()?;
        let peer_dir = tempfile::tempdir()?;
        let server = fixture(server_dir.path()).await?;
        let peer = fixture(peer_dir.path()).await?;
        let address = paired_full(&server, &peer).await?;
        let peer_fingerprint = info(&peer).await?.fingerprint;
        let mut grants = server
            .global_params
            .node
            .access_policy
            .grants(&peer_fingerprint)?;
        grants.control = false;
        server
            .global_params
            .node
            .access_policy
            .set(&peer_fingerprint, grants)?;
        let (_, client) = authenticated_client(
            &peer.global_params.node,
            &info(&peer).await?.fingerprint,
            &peer.private_key,
            &info(&server).await?.fingerprint,
            "127.0.0.1",
            address.port(),
        )
        .await?;

        let access: serde_json::Value = client
            .get(format!("https://{address}/access"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(access["control"], false);
        assert!(access.get("connection").is_some());
        assert!(access.get("sync").is_some());
        assert_eq!(access["connection"]["all_libraries"], "ViewEdit");
        assert_eq!(access["sync"]["all_libraries"], "PullPush");

        let response = client
            .post(format!("https://{address}/cast/preflight"))
            .json(&serde_json::json!({ "libraryIds": [] }))
            .send()
            .await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        server.global_params.node.shutdown().await?;
        peer.global_params.node.shutdown().await?;
        Ok(())
    }

    async fn close_ui(params: &GlobalParams) -> Result<()> {
        if params.library.is_some() {
            let request = CloseLibraryRequest { call_id: None };
            request
                .handle(request.extract_params(params), None, &request)
                .await?;
        } else {
            params.node.close_ui(&params.main_token).await;
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn node_listener_and_worker_survive_ui_close() -> Result<()> {
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let a = fixture(first.path()).await?;
        let b = fixture(second.path()).await?;
        let addr = paired_full(&a, &b).await?;
        let worker = a.sync_handle.lock().await.as_ref().context("worker")?.id();
        let (_, client) = authenticated_client(
            &b.global_params.node,
            &info(&b).await?.fingerprint,
            &b.private_key,
            &info(&a).await?.fingerprint,
            "127.0.0.1",
            addr.port(),
        )
        .await?;
        close_ui(&a.global_params).await?;
        assert_eq!(
            a.get_address().await,
            Some(addr),
            "UI close stopped the node listener"
        );
        assert_eq!(
            a.sync_handle
                .lock()
                .await
                .as_ref()
                .context("worker stopped")?
                .id(),
            worker
        );
        client
            .get(format!("https://{addr}/libraries"))
            .send()
            .await?
            .error_for_status()?;
        let node = a.global_params.node.clone();
        let original_id = a.global_params.library_id.context("original library")?;
        let other_id = held_owned_library(&a, &first.path().join("B")).await?;
        let original_connection = a.global_params.test_library().main_db.clone();
        let mut current = None;
        for id in [other_id, original_id, original_id] {
            let opened =
                fixture_from_library(node.clone(), node.library_pool.acquire_ui(id).await?).await?;
            if id == original_id {
                assert!(Arc::ptr_eq(
                    &opened.global_params.test_library().main_db,
                    &original_connection
                ));
            }
            assert!(Arc::ptr_eq(
                &opened.global_params.permission_manager,
                &a.global_params.permission_manager
            ));
            assert!(Arc::ptr_eq(
                &opened.global_params.cert_validator,
                &a.global_params.cert_validator
            ));
            assert!(Arc::ptr_eq(
                &opened.global_params.library_pool,
                &node.library_pool
            ));
            node.attach_ui(&opened.global_params).await;
            assert_eq!(a.get_address().await, Some(addr));
            assert_eq!(
                a.sync_handle
                    .lock()
                    .await
                    .as_ref()
                    .context("worker stopped")?
                    .id(),
                worker
            );
            client
                .get(format!("https://{addr}/libraries"))
                .send()
                .await?
                .error_for_status()?;
            current = Some(opened);
        }
        close_ui(&current.context("current UI")?.global_params).await?;
        let reopened = fixture_from_library(
            node.clone(),
            node.library_pool.acquire_ui(original_id).await?,
        )
        .await?;
        close_ui(&reopened.global_params).await?;
        set_library_background(&a, original_id, false).await?;
        set_library_background(&a, other_id, false).await?;
        assert!(
            node.library_pool.served_library_ids().await?.is_empty(),
            "UI registration leaked"
        );
        assert_eq!(a.get_address().await, Some(addr));
        assert_eq!(
            a.sync_handle
                .lock()
                .await
                .as_ref()
                .context("worker stopped")?
                .id(),
            worker
        );
        node.shutdown().await?;
        b.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn removing_owned_record_revokes_serving_and_sync_and_preserves_files() -> Result<()> {
        let owner_dir = tempfile::tempdir()?;
        let peer_dir = tempfile::tempdir()?;
        let owner = fixture(owner_dir.path()).await?;
        let peer = fixture(peer_dir.path()).await?;
        let address = paired_full(&owner, &peer).await?;
        let owner_info = info(&owner).await?;
        let peer_info = info(&peer).await?;
        let (_, client) = authenticated_client(
            &peer.global_params.node,
            &peer_info.fingerprint,
            &peer.private_key,
            &owner_info.fingerprint,
            "127.0.0.1",
            address.port(),
        )
        .await?;
        let root = owner_dir.path().join("removable-owned");
        fs::create_dir_all(&root)?;
        let registered = register_local_library_on_node(
            &RegisterLocalLibraryRequest {
                call_id: None,
                path: root.to_string_lossy().into_owned(),
                alias: "Removable".into(),
                mode: Some(LibraryInitializeMode::Portable),
            },
            &owner.global_params.node,
        )
        .await?;
        anyhow::ensure!(registered.outcome == RegisterLibraryOutcome::Registered);
        let summary = registered.record.context("registered owned library")?;
        let library_id = match &summary.kind {
            LibraryRecordKindSummary::Local { library_id, .. } => Uuid::parse_str(library_id)?,
            _ => anyhow::bail!("expected local owned library record"),
        };
        let record = LibraryRegistry::load(owner_dir.path())?
            .catalog(library_id)?
            .clone();
        let root = record.root(owner_dir.path()).context("owned root")?;
        let marker = root.join(".rune");
        anyhow::ensure!(marker.is_dir(), "library metadata directory missing");
        let before: Vec<ListedLibrary> = client
            .get(format!("https://{address}/libraries"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        anyhow::ensure!(before.iter().any(|entry| entry.library_id == library_id));

        let worker_token = owner
            .global_params
            .node
            .sync_library_token(library_id)
            .await;
        let request = RemoveLibraryRecordRequest {
            call_id: None,
            record_id: record.record_id.0.to_string(),
        };
        request
            .handle(owner.global_params.node.clone(), None, &request)
            .await?
            .context("remove response")?;
        anyhow::ensure!(worker_token.is_cancelled(), "sync worker was not cancelled");
        let after: Vec<ListedLibrary> = client
            .get(format!("https://{address}/libraries"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        anyhow::ensure!(!after.iter().any(|entry| entry.library_id == library_id));
        anyhow::ensure!(
            !owner
                .global_params
                .node
                .library_pool
                .served_library_ids()
                .await?
                .contains(&library_id),
            "removed library remained eligible for background work"
        );
        anyhow::ensure!(
            owner
                .global_params
                .node
                .library_pool
                .acquire_for_request(library_id)
                .await
                .is_err(),
            "removed library could still be acquired for sync"
        );
        anyhow::ensure!(
            owner
                .global_params
                .node
                .library_pool
                .acquire_served(library_id)
                .await?
                .is_none(),
            "background sync could still acquire the removed library"
        );
        anyhow::ensure!(
            root.is_dir() && marker.is_dir(),
            "removal deleted library files"
        );

        let registered = register_local_library_on_node(
            &RegisterLocalLibraryRequest {
                call_id: None,
                path: root.to_string_lossy().into_owned(),
                alias: "Removable".into(),
                mode: Some(LibraryInitializeMode::Portable),
            },
            &owner.global_params.node,
        )
        .await?;
        let summary = registered.record.context("re-registered record")?;
        anyhow::ensure!(
            matches!(summary.kind, LibraryRecordKindSummary::Local { library_id: ref id, .. } if id == &library_id.to_string()),
            "re-registering folder did not restore the same library id"
        );
        owner.global_params.node.shutdown().await?;
        peer.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn node_dispatch_requires_current_ui_library() -> Result<()> {
        use crate::backends::remote::connect_remote;
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let a = fixture(first.path()).await?;
        let b = fixture(second.path()).await?;
        let addr = peer_step("pair headless controller nodes", paired_full(&a, &b)).await?;
        let mut socket = peer_step("connect headless controller", async {
            Ok(connect_remote(
                "127.0.0.1",
                addr.port(),
                &b.global_params.config_path,
                b.global_params.cert_validator.read().await.clone(),
                &b.global_params.node.sync_tokens,
            )
            .await?)
        })
        .await?;
        let id = a.global_params.library_id.context("library")?;
        close_ui(&a.global_params).await?;
        let payload = rinf::serialize(&GetMediaFilesCountRequest { call_id: None })?;
        let (name, bytes) =
            socket_request(&mut socket, "GetMediaFilesCountRequest", &payload).await?;
        assert_eq!(name, "RemoteRequestFailed");
        let failure: RemoteRequestFailed = rinf::deserialize(&bytes)?;
        assert_eq!(failure.kind, RequestErrorKind::Unavailable);
        let node = a.global_params.node.clone();
        let reopened =
            fixture_from_library(node.clone(), node.library_pool.acquire_ui(id).await?).await?;
        let (name, bytes) =
            socket_request_for_library(&mut socket, "GetMediaFilesCountRequest", &payload, id)
                .await?;
        assert_eq!(name, "GetMediaFilesCountResponse");
        assert_eq!(
            rinf::deserialize::<GetMediaFilesCountResponse>(&bytes)?.count,
            0
        );
        close_ui(&reopened.global_params).await?;
        socket.close(None).await?;
        node.shutdown().await?;
        b.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn node_initialization_is_idempotent_and_rejects_another_path() -> Result<()> {
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        SyncSettings {
            listen: false,
            ..SyncSettings::default()
        }
        .save(first.path())?;
        let slot = NodeSlot::new();
        let request = InitializeNodeRequest {
            call_id: None,
            config_path: first.path().to_string_lossy().into_owned(),
        };
        let node = slot
            .initialize(
                &request.config_path,
                &request.config_path,
                Arc::new(Quiet),
                RunMode::App,
            )
            .await?;
        let worker = node
            .manager()?
            .sync_handle
            .lock()
            .await
            .as_ref()
            .context("worker")?
            .id();
        for _ in 0..2 {
            slot.handle(&request, Arc::new(Quiet)).await?;
            let same = slot
                .initialize(
                    &request.config_path,
                    &request.config_path,
                    Arc::new(Quiet),
                    RunMode::App,
                )
                .await?;
            assert!(Arc::ptr_eq(&node, &same));
            assert_eq!(
                same.manager()?
                    .sync_handle
                    .lock()
                    .await
                    .as_ref()
                    .context("worker")?
                    .id(),
                worker
            );
        }
        let error = match slot
            .handle(
                &InitializeNodeRequest {
                    call_id: None,
                    config_path: second.path().to_string_lossy().into_owned(),
                },
                Arc::new(Quiet),
            )
            .await
        {
            Ok(_) => anyhow::bail!("another configuration path must be refused"),
            Err(error) => error,
        };
        assert_eq!(classify(&error), RequestErrorKind::InvalidInput);
        assert!(format!("{error:#}").contains("configuration path cannot change"));
        node.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn outbound_worker_survives_listener_stop() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let manager = fixture(dir.path()).await?;
        manager.start_worker().await?;
        let device_info = info(&manager).await?;
        manager
            .clone()
            .start_sync(([127, 0, 0, 1], 0).into(), DiscoveryParams { device_info })
            .await?;
        manager.stop().await?;
        assert!(
            manager
                .sync_handle
                .lock()
                .await
                .as_ref()
                .is_some_and(|task| !task.is_finished())
        );
        manager.stop_worker().await?;
        assert!(manager.sync_handle.lock().await.is_none());
        manager.global_params.node.token.cancel();
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn failed_stop_restores_the_sync_listener_before_reporting() -> Result<()> {
        let _port = crate::test_node::SYNC_PORT.lock().await;
        let dir = tempfile::tempdir()?;
        let fixture = fixture(dir.path()).await?;
        let manager = &fixture.manager;
        let params = &fixture.global_params;
        let device_info = info(&fixture).await?;
        manager
            .clone()
            .start_sync(([127, 0, 0, 1], 0).into(), DiscoveryParams { device_info })
            .await?;
        assert!(manager.is_sync_listener());

        let settings = SyncSettings {
            listen: true,
            ..SyncSettings::default()
        };
        settings.save(Path::new(&*params.config_path))?;
        *params.sync_settings.write().await = settings;

        // Abort the server task so stop() clears the listener state and then returns a
        // join error instead of a clean shutdown.
        manager
            .server_handle
            .lock()
            .await
            .as_ref()
            .context("sync server task")?
            .abort();

        let request = SetSyncSettingsRequest {
            request_id: "failed-stop".into(),
            listen: false,
            missing_host_file: crate::messages::MissingHostFilePolicy::RemoveEverywhere,
            call_id: None,
        };
        let error = request
            .handle(
                (
                    Arc::clone(&params.sync_settings),
                    Arc::clone(&params.config_path),
                    Some(Arc::clone(manager)),
                    params.node.run_mode,
                ),
                None,
                &request,
            )
            .await
            .err()
            .expect("disabling a listener whose shutdown fails");
        assert!(format!("{error:#}").contains("cancelled"), "{error:#}");
        assert!(
            manager.is_sync_listener(),
            "the stopped sync listener was not restored"
        );
        assert!(params.sync_settings.read().await.listen);
        assert!(SyncSettings::load(Path::new(&*params.config_path)).listen);

        fixture.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn failed_stop_reports_a_failed_restore() -> Result<()> {
        let _port = crate::test_node::SYNC_PORT.lock().await;
        let dir = tempfile::tempdir()?;
        let fixture = fixture(dir.path()).await?;
        let manager = &fixture.manager;
        let params = &fixture.global_params;
        let device_info = info(&fixture).await?;
        manager
            .clone()
            .start_sync(([127, 0, 0, 1], 0).into(), DiscoveryParams { device_info })
            .await?;
        let settings = SyncSettings {
            listen: true,
            ..SyncSettings::default()
        };
        settings.save(Path::new(&*params.config_path))?;
        *params.sync_settings.write().await = settings;

        manager
            .server_handle
            .lock()
            .await
            .as_ref()
            .context("sync server task")?
            .abort();
        let occupied = TcpListener::bind(SocketAddr::from((
            [0, 0, 0, 0],
            discovery::endpoint::DEFAULT_DEVICE_PORT,
        )))?;

        let request = SetSyncSettingsRequest {
            request_id: "failed-restore".into(),
            listen: false,
            missing_host_file: crate::messages::MissingHostFilePolicy::RemoveEverywhere,
            call_id: None,
        };
        let error = request
            .handle(
                (
                    Arc::clone(&params.sync_settings),
                    Arc::clone(&params.config_path),
                    Some(Arc::clone(manager)),
                    params.node.run_mode,
                ),
                None,
                &request,
            )
            .await
            .err()
            .expect("disabling a listener whose shutdown and restore fail");
        let text = format!("{error:#}");
        assert!(text.contains("cancelled"), "{text}");
        assert!(text.contains("Failed to bind sync server"), "{text}");
        assert!(!manager.is_running());
        assert!(params.sync_settings.read().await.listen);
        assert!(SyncSettings::load(Path::new(&*params.config_path)).listen);
        drop(occupied);

        fixture.global_params.node.shutdown().await?;
        Ok(())
    }

    async fn pair(a: &Fixture, b: &Fixture) -> Result<()> {
        let a_info = info(a).await?;
        let b_info = info(b).await?;
        a.global_params
            .permission_manager
            .write()
            .await
            .add_user(
                b.certificate.clone(),
                b_info.fingerprint.clone(),
                "paired".into(),
                "test".into(),
                DeviceType::Desktop,
                "127.0.0.2".into(),
            )
            .await?;
        a.global_params
            .permission_manager
            .write()
            .await
            .change_user_status(&b_info.fingerprint, UserStatus::Approved)
            .await?;
        b.global_params
            .cert_validator
            .read()
            .await
            .add_trusted_domains(["127.0.0.1"], &a_info.fingerprint)
            .await?;
        if let Some(address) = a.manager.get_address().await {
            b.global_params
                .cert_validator
                .read()
                .await
                .replace_hosts_for_fingerprint(
                    &a_info.fingerprint,
                    vec![format!("127.0.0.1:{}", address.port())],
                )
                .await?;
        }
        Ok(())
    }

    async fn paired_full(a: &Fixture, b: &Fixture) -> Result<SocketAddr> {
        paired_full_with_protocol_version(a, b, protocol_version::PROTOCOL_VERSION).await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn server_availability_test_accepts_reachable_typed_host() -> Result<()> {
        let server_dir = tempfile::tempdir()?;
        let client_dir = tempfile::tempdir()?;
        let server = fixture_with_library(server_dir.path(), Uuid::new_v4()).await?;
        let client = fixture_with_library(client_dir.path(), Uuid::new_v4()).await?;
        let address = paired_full(&server, &client).await?;
        let request = ServerAvailabilityTestRequest {
            hosts: vec![format!("127.0.0.1:{}", address.port())],
            call_id: None,
        };

        let response = request
            .handle(
                Arc::clone(&client.global_params.cert_validator),
                None,
                &request,
            )
            .await?
            .context("availability response")?;
        assert_eq!(response.outcome, ServerAvailabilityTestOutcome::Success);
        Ok(())
    }

    async fn paired_full_with_protocol_version(
        a: &Fixture,
        b: &Fixture,
        protocol_version: protocol_version::ProtocolVersion,
    ) -> Result<SocketAddr> {
        let a_fingerprint = info(a).await?.fingerprint;
        let b_fingerprint = info(b).await?.fingerprint;
        if a.global_params
            .permission_manager
            .read()
            .await
            .verify_by_fingerprint(&b_fingerprint)
            .await
            .is_none()
        {
            pair(a, b).await?;
        }
        if b.global_params
            .permission_manager
            .read()
            .await
            .verify_by_fingerprint(&a_fingerprint)
            .await
            .is_none()
        {
            pair(b, a).await?;
        }
        a.manager
            .clone()
            .start(
                ([127, 0, 0, 1], 0).into(),
                DiscoveryParams {
                    device_info: {
                        let mut info = info(a).await?;
                        info.protocol_version = Some(protocol_version);
                        info
                    },
                },
            )
            .await?;
        let address = a.get_address().await.context("missing listener address")?;
        b.global_params
            .cert_validator
            .read()
            .await
            .replace_hosts_for_fingerprint(
                &a_fingerprint,
                vec![format!("127.0.0.1:{}", address.port())],
            )
            .await?;
        Ok(address)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn websocket_handshake_rejects_major_mismatch_in_both_directions() -> Result<()> {
        use crate::messages::{ProtocolUpdateSide, RequestErrorKind};
        use crate::request_error::{ExpectedRequestError, classify};
        use discovery::protocol_version::ProtocolVersion;

        for (client_version, server_version, expected_side) in [
            ("1.0.0", "2.0.0", ProtocolUpdateSide::ThisDevice),
            ("2.0.0", "1.0.0", ProtocolUpdateSide::Peer),
        ] {
            let server_dir = tempfile::tempdir()?;
            let client_dir = tempfile::tempdir()?;
            let server = step("create handshake server", fixture(server_dir.path())).await?;
            let client = step("create handshake client", fixture(client_dir.path())).await?;
            client
                .global_params
                .node
                .set_protocol_version_for_test(ProtocolVersion::parse(client_version)?);
            let address = step(
                "pair handshake fixtures",
                paired_full_with_protocol_version(
                    &server,
                    &client,
                    ProtocolVersion::parse(server_version)?,
                ),
            )
            .await?;
            let server_info = step("read handshake server identity", info(&server)).await?;
            step("store handshake endpoint", async {
                client
                    .global_params
                    .cert_validator
                    .read()
                    .await
                    .replace_hosts_for_fingerprint(
                        &server_info.fingerprint,
                        vec![format!("127.0.0.1:{}", address.port())],
                    )
                    .await?;
                Ok::<_, anyhow::Error>(())
            })
            .await?;

            let error = step(
                "reject incompatible websocket peer",
                client
                    .global_params
                    .node
                    .attach_renderer(&server_info.fingerprint),
            )
            .await
            .unwrap_err();
            assert_eq!(classify(&error), RequestErrorKind::PeerNeedsUpdate);
            assert!(
                matches!(
                    error.downcast_ref::<ExpectedRequestError>(),
                Some(ExpectedRequestError::PeerNeedsUpdate { update_side, peer_alias: Some(alias), .. })
                    if *update_side == expected_side && !alias.is_empty()
                ),
                "unexpected typed handshake error: {error:#?}"
            );
            step(
                "shutdown handshake client",
                client.global_params.node.shutdown(),
            )
            .await?;
            step(
                "shutdown handshake server",
                server.global_params.node.shutdown(),
            )
            .await?;
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn repl_handshake_rejects_an_incompatible_server_version() -> Result<()> {
        use discovery::protocol_version::ProtocolVersion;

        let server_dir = tempfile::tempdir()?;
        let client_dir = tempfile::tempdir()?;
        let server = step("create version server", fixture(server_dir.path())).await?;
        let client = step("create version client", fixture(client_dir.path())).await?;
        let address = step(
            "pair version fixtures",
            paired_full_with_protocol_version(&server, &client, ProtocolVersion::parse("3.0.0")?),
        )
        .await?;
        let validator = client.global_params.cert_validator.read().await.clone();
        let error = match step(
            "reject the incompatible REPL handshake",
            connect_controller(
                "127.0.0.1",
                address.port(),
                client.global_params.config_path.as_str(),
                validator,
            ),
        )
        .await
        {
            Ok(_) => panic!("an incompatible server version must be rejected"),
            Err(error) => error,
        };
        assert!(
            matches!(
                error,
                RemoteConnectError::PeerNeedsUpdate {
                    peer: Some(version),
                    ..
                } if version == ProtocolVersion::parse("3.0.0").unwrap()
            ),
            "unexpected handshake error: {error:?}"
        );
        step("stop version server", server.manager.stop()).await?;
        step(
            "shutdown version client",
            client.global_params.node.shutdown(),
        )
        .await?;
        step(
            "shutdown version server",
            server.global_params.node.shutdown(),
        )
        .await?;
        Ok(())
    }

    async fn assert_websocket_request_is_ignored(name: &str, payload: Vec<u8>) -> Result<()> {
        use crate::backends::remote::encode_request;

        let renderer_dir = tempfile::tempdir()?;
        let controller_dir = tempfile::tempdir()?;
        let renderer = fixture(renderer_dir.path()).await?;
        let controller = fixture(controller_dir.path()).await?;
        let addr = paired_full(&renderer, &controller).await?;
        let mut socket = connect_remote(
            "127.0.0.1",
            addr.port(),
            &controller.global_params.config_path,
            controller.global_params.cert_validator.read().await.clone(),
            &controller.global_params.node.sync_tokens,
        )
        .await?;
        let ignored_id = Uuid::new_v4();
        let following_id = Uuid::new_v4();
        let library_id = renderer
            .global_params
            .library_id
            .context("renderer library id")?;
        socket
            .send(Message::Binary(
                encode_message(name, &payload, Some(ignored_id)).into(),
            ))
            .await?;
        let payload = rinf::serialize(&GetMediaFilesCountRequest { call_id: None })?;
        socket
            .send(Message::Binary(
                encode_request(
                    "GetMediaFilesCountRequest",
                    &payload,
                    following_id,
                    library_id,
                )
                .into(),
            ))
            .await?;
        tokio_time::timeout(TEST_DEADLINE, async {
            loop {
                let frame = socket.next().await.context("socket closed")??;
                let Message::Binary(bytes) = frame else {
                    continue;
                };
                let (response_name, payload, id) =
                    decode_message(&bytes).context("response frame")?;
                assert_ne!(id, ignored_id, "{name} must receive no reply");
                if id == following_id {
                    assert_eq!(response_name, "GetMediaFilesCountResponse");
                    let response: GetMediaFilesCountResponse = rinf::deserialize(&payload)?;
                    assert_eq!(response.count, 0);
                    break;
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("following request received no reply")??;
        drop(socket);
        renderer.global_params.node.shutdown().await?;
        controller.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn websocket_node_local_request_is_ignored() -> Result<()> {
        let payload = rinf::serialize(&FetchRemoteFileRequest {
            call_id: None,
            url: String::new(),
        })?;
        assert_websocket_request_is_ignored("FetchRemoteFileRequest", payload).await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn websocket_unknown_request_is_ignored() -> Result<()> {
        assert_websocket_request_is_ignored("UnknownRequest", vec![255]).await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn one_connection_keeps_frame_order() -> Result<()> {
        use crate::backends::remote::encode_message;
        use crate::server::register_gated_handler;
        use futures::SinkExt;
        use std::time::Duration;
        use tokio::sync::Notify;
        use tokio::time::timeout;
        use tokio_tungstenite::tungstenite::Message;

        let renderer_dir = tempfile::tempdir()?;
        let controller_dir = tempfile::tempdir()?;
        let renderer = fixture(renderer_dir.path()).await?;
        let controller = fixture(controller_dir.path()).await?;
        let service = &renderer.global_params.node.websocket_service;
        let (first_entered, release_first) = register_gated_handler(service, "OrderFirst").await;
        let second_entered = Arc::new(Notify::new());
        let second_entered_handler = second_entered.clone();
        service
            .override_handler("OrderSecond", move |_, _, _| {
                let entered = second_entered_handler.clone();
                async move {
                    entered.notify_one();
                    ("OrderSecondResponse".into(), Ok(Vec::new()))
                }
            })
            .await;

        let addr = paired_full(&renderer, &controller).await?;
        let mut socket = connect_remote(
            "127.0.0.1",
            addr.port(),
            &controller.global_params.config_path,
            controller.global_params.cert_validator.read().await.clone(),
            &controller.global_params.node.sync_tokens,
        )
        .await?;
        socket
            .send(Message::Binary(
                encode_message("OrderFirst", &[], Some(Uuid::new_v4())).into(),
            ))
            .await?;
        socket
            .send(Message::Binary(
                encode_message("OrderSecond", &[], Some(Uuid::new_v4())).into(),
            ))
            .await?;

        tokio_time::timeout(TEST_DEADLINE, first_entered.notified())
            .await
            .context("first handler did not enter")?;
        assert!(
            timeout(Duration::from_millis(200), second_entered.notified())
                .await
                .is_err(),
            "second frame ran before the first handler finished"
        );

        release_first.notify_one();
        tokio_time::timeout(TEST_DEADLINE, second_entered.notified())
            .await
            .context("second handler did not enter after the first completed")?;

        drop(socket);
        renderer.global_params.node.shutdown().await?;
        controller.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn remote_connection_classification_and_revocation() -> Result<()> {
        use crate::backends::remote::{RemoteConnectError, connect_remote};
        use crate::server::sync_worker::{obtain_token, peer_base};
        use futures::StreamExt;
        use tokio_tungstenite::tungstenite::Message;

        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let a = fixture(first.path()).await?;
        let b = fixture(second.path()).await?;
        let validator = b.global_params.cert_validator.read().await.clone();
        let config = &b.global_params.config_path;
        assert!(matches!(
            connect_remote(
                "127.0.0.1",
                1,
                config,
                validator.clone(),
                &b.global_params.node.sync_tokens
            )
            .await,
            Err(RemoteConnectError::UntrustedEndpoint)
        ));
        let addr = paired_full(&a, &b).await?;
        let validator = b.global_params.cert_validator.read().await.clone();
        let fingerprint = info(&b).await?.fingerprint;
        let base = peer_base("127.0.0.1", addr.port())?;
        let client = pinned_anonymous_client(&b.global_params, None).await?;
        let server_fingerprint = info(&a).await?.fingerprint;
        for remove in [false, true] {
            // Keep this token out of the shared cache so it is unused during revocation.
            let token = obtain_token(
                &client,
                &base,
                &fingerprint,
                &b.private_key,
                &server_fingerprint,
            )
            .await?;
            let access = || client.get(format!("{base}/access")).bearer_auth(&token);
            assert_eq!(access().send().await?.status(), StatusCode::OK);
            let mut socket = connect_remote(
                "127.0.0.1",
                addr.port(),
                config,
                validator.clone(),
                &b.global_params.node.sync_tokens,
            )
            .await?;
            if remove {
                a.global_params
                    .permission_manager
                    .read()
                    .await
                    .remove_user(&fingerprint)
                    .await?;
            } else {
                let mut changes = a.global_params.permission_manager.read().await.changes();
                PermissionManager::new(a.global_params.config_path.as_str())?
                    .change_user_status(&fingerprint, UserStatus::Blocked)
                    .await?;
                // The file watcher publishes this change after running the trust hooks.
                tokio_time::timeout(TEST_DEADLINE, changes.changed())
                    .await
                    .context("file-driven client block was not applied")??;
            }
            tokio_time::timeout(TEST_DEADLINE, async {
                loop {
                    match socket
                        .next()
                        .await
                        .context("socket closed without a revocation close frame")??
                    {
                        Message::Close(Some(frame))
                            if u16::from(frame.code) == 4403 && frame.reason == "revoked" =>
                        {
                            break;
                        }
                        Message::Close(frame) => {
                            anyhow::bail!("unexpected revocation close frame: {frame:?}");
                        }
                        _ => {}
                    }
                }
                Ok::<_, anyhow::Error>(())
            })
            .await
            .context("revocation close frame was not received")??;
            wait_until("revoked controller to detach", || async {
                Ok(a.global_params
                    .node
                    .session()?
                    .attached_controllers()
                    .iter()
                    .all(|controller| controller.fingerprint != fingerprint)
                    .then_some(()))
            })
            .await?;
            if remove {
                pair(&a, &b).await?;
            } else {
                assert!(matches!(
                    connect_remote(
                        "127.0.0.1",
                        addr.port(),
                        config,
                        validator.clone(),
                        &b.global_params.node.sync_tokens
                    )
                    .await,
                    Err(RemoteConnectError::NotAuthorized)
                ));
                a.global_params
                    .permission_manager
                    .read()
                    .await
                    .change_user_status(&fingerprint, UserStatus::Approved)
                    .await?;
            }
            // Approval is restored, so denial proves the old token was purged.
            assert_eq!(access().send().await?.status(), StatusCode::UNAUTHORIZED);
        }
        let mut socket = connect_remote(
            "127.0.0.1",
            addr.port(),
            config,
            validator.clone(),
            &b.global_params.node.sync_tokens,
        )
        .await?;
        let close_frame = tokio::spawn(async move {
            tokio_time::timeout(TEST_DEADLINE, socket.next())
                .await
                .context("server-stop close frame timed out")?
                .context("socket closed without a server-stop close frame")?
                .map_err(Error::new)
                .context("reading the server-stop close frame")
        });
        a.stop()
            .await
            .context("stopping server after revocation checks")?;
        let frame = close_frame.await??;
        assert!(
            matches!(frame, Message::Close(Some(frame)) if u16::from(frame.code) == 1001 && frame.reason == "server stopping")
        );
        assert!(matches!(
            connect_remote(
                "127.0.0.1",
                addr.port(),
                config,
                validator,
                &b.global_params.node.sync_tokens
            )
            .await,
            Err(RemoteConnectError::Unreachable)
        ));
        a.global_params.node.token.cancel();
        b.global_params.node.token.cancel();
        Ok(())
    }

    /// The load a controller sends to start playback on the paired player.
    async fn controller_load_playing(
        player: &Fixture,
        controller: &Fixture,
    ) -> Result<(
        SocketAddr,
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
    )> {
        use ::database::actions::file::file_uuids_by_id;
        use playback::player::PlaybackState as PlayerPlaybackState;

        let library_id = player
            .global_params
            .library_id
            .context("player library id")?;
        let track = scan_held_track(player, library_id, "revoked-playback.ogg").await?;
        let file_uuid = file_uuids_by_id(&player.global_params.test_library().main_db, &[track.id])
            .await?
            .get(&track.id)
            .copied()
            .context("scanned track uuid")?;
        let addr = paired_full(player, controller).await?;
        let mut socket = connect_remote(
            "127.0.0.1",
            addr.port(),
            &controller.global_params.config_path,
            controller.global_params.cert_validator.read().await.clone(),
            &controller.global_params.node.sync_tokens,
        )
        .await?;
        let load = LoadResolvedItemsRequest {
            call_id: None,
            items: vec![PlayingItemRef::LibraryTrack {
                library_id: library_id.to_string(),
                track: file_uuid.to_string(),
            }],
            start_index: Some(0),
            mode: PlaylistOperateMode::Replace,
            expected_revision: None,
            playback_mode: Some(PlaybackMode::RepeatAll),
            position_seconds: None,
            play: LoadPlay::Play,
        };
        let (name, payload) = socket_request(
            &mut socket,
            "LoadResolvedItemsRequest",
            &rinf::serialize(&load)?,
        )
        .await?;
        assert_eq!(name, "LoadResolvedItemsResponse");
        let response: LoadResolvedItemsResponse = rinf::deserialize(&payload)?;
        assert!(response.result.left_out_items.is_empty());
        wait_until("controller-started playback", || async {
            let status = player.global_params.player.lock().await.get_status();
            Ok((status.state == PlayerPlaybackState::Playing).then_some(()))
        })
        .await?;
        Ok((addr, socket))
    }

    async fn revoke_and_assert_playback_stops(remove: bool) -> Result<()> {
        use futures::StreamExt;
        use playback::player::PlaybackState as PlayerPlaybackState;
        use tokio_tungstenite::tungstenite::Message;

        let player_dir = tempfile::tempdir()?;
        let controller_dir = tempfile::tempdir()?;
        let player = fixture(player_dir.path()).await?;
        let controller = fixture(controller_dir.path()).await?;
        let fingerprint = info(&controller).await?.fingerprint;
        let (_addr, mut socket) = controller_load_playing(&player, &controller).await?;
        if remove {
            player
                .global_params
                .permission_manager
                .read()
                .await
                .remove_user(&fingerprint)
                .await?;
        } else {
            let mut changes = player
                .global_params
                .permission_manager
                .read()
                .await
                .changes();
            PermissionManager::new(player.global_params.config_path.as_str())?
                .change_user_status(&fingerprint, UserStatus::Blocked)
                .await?;
            tokio_time::timeout(TEST_DEADLINE, changes.changed())
                .await
                .context("file-driven client block was not applied")??;
        }
        // The revocation close frame is the barrier: the server has detached the controller by
        // the time it arrives, so the playback the controller started must be stopping too.
        tokio_time::timeout(TEST_DEADLINE, async {
            loop {
                match socket
                    .next()
                    .await
                    .context("socket closed without a revocation close frame")??
                {
                    Message::Close(Some(frame))
                        if u16::from(frame.code) == 4403 && frame.reason == "revoked" =>
                    {
                        break;
                    }
                    Message::Close(frame) => {
                        anyhow::bail!("unexpected revocation close frame: {frame:?}");
                    }
                    _ => {}
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("revocation close frame was not received")??;
        wait_until(
            "playback the revoked controller started to stop",
            || async {
                let status = player.global_params.player.lock().await.get_status();
                Ok((status.state == PlayerPlaybackState::Paused).then_some(()))
            },
        )
        .await?;
        player.global_params.node.token.cancel();
        controller.global_params.node.token.cancel();
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn playback_stops_when_starting_controller_is_removed() -> Result<()> {
        revoke_and_assert_playback_stops(true).await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn playback_stops_when_starting_controller_is_blocked() -> Result<()> {
        revoke_and_assert_playback_stops(false).await
    }

    /// Revoking the controller must only stop the playback that controller started: once the
    /// device itself has loaded its own playback, the revoked controller no longer owns it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn local_playback_survives_revoking_the_controller_it_replaced() -> Result<()> {
        use ::database::actions::file::file_uuids_by_id;
        use playback::player::PlaybackState as PlayerPlaybackState;

        let player_dir = tempfile::tempdir()?;
        let controller_dir = tempfile::tempdir()?;
        let player = fixture(player_dir.path()).await?;
        let controller = fixture(controller_dir.path()).await?;
        let fingerprint = info(&controller).await?.fingerprint;
        let (_addr, socket) = controller_load_playing(&player, &controller).await?;
        // The device itself loads its own track, taking playback over from the controller.
        let library_id = player
            .global_params
            .library_id
            .context("player library id")?;
        let track = scan_held_track(&player, library_id, "local-takeover.ogg").await?;
        let file_uuid = file_uuids_by_id(&player.global_params.test_library().main_db, &[track.id])
            .await?
            .get(&track.id)
            .copied()
            .context("takeover track uuid")?;
        let load = LoadResolvedItemsRequest {
            call_id: None,
            items: vec![PlayingItemRef::LibraryTrack {
                library_id: library_id.to_string(),
                track: file_uuid.to_string(),
            }],
            start_index: Some(0),
            mode: PlaylistOperateMode::Replace,
            expected_revision: None,
            playback_mode: Some(PlaybackMode::RepeatAll),
            position_seconds: None,
            play: LoadPlay::Play,
        };
        load.handle(load.extract_params(&player.global_params), None, &load)
            .await?;
        wait_until("locally loaded playback", || async {
            let status = player.global_params.player.lock().await.get_status();
            Ok((status.state == PlayerPlaybackState::Playing).then_some(()))
        })
        .await?;
        player
            .global_params
            .permission_manager
            .read()
            .await
            .remove_user(&fingerprint)
            .await?;
        // The detached controller is the barrier: the revocation has finished acting on the
        // playback owner by the time the controller leaves the list, so playback still running
        // then is playback the revocation did not stop.
        wait_until("revoked controller to detach", || async {
            Ok(player
                .global_params
                .node
                .session()?
                .attached_controllers()
                .iter()
                .all(|controller| controller.fingerprint != fingerprint)
                .then_some(()))
        })
        .await?;
        let status = player.global_params.player.lock().await.get_status();
        assert_eq!(
            status.state,
            PlayerPlaybackState::Playing,
            "local playback must survive the controller's revocation"
        );
        drop(socket);
        player.global_params.node.token.cancel();
        controller.global_params.node.token.cancel();
        Ok(())
    }

    /// The revocation watch must also reach playback started before the controller
    /// reconnected: ownership belongs to the device, not to one socket.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn playback_stops_when_controller_revoked_after_reconnect() -> Result<()> {
        use futures::StreamExt;
        use playback::player::PlaybackState as PlayerPlaybackState;
        use tokio_tungstenite::tungstenite::Message;

        let player_dir = tempfile::tempdir()?;
        let controller_dir = tempfile::tempdir()?;
        let player = fixture(player_dir.path()).await?;
        let controller = fixture(controller_dir.path()).await?;
        let fingerprint = info(&controller).await?.fingerprint;
        let (addr, socket) = controller_load_playing(&player, &controller).await?;
        // The controller disconnects and connects again; the playback it started runs on.
        drop(socket);
        let mut socket = connect_remote(
            "127.0.0.1",
            addr.port(),
            &controller.global_params.config_path,
            controller.global_params.cert_validator.read().await.clone(),
            &controller.global_params.node.sync_tokens,
        )
        .await?;
        player
            .global_params
            .permission_manager
            .read()
            .await
            .remove_user(&fingerprint)
            .await?;
        tokio_time::timeout(TEST_DEADLINE, async {
            loop {
                match socket
                    .next()
                    .await
                    .context("socket closed without a revocation close frame")??
                {
                    Message::Close(Some(frame))
                        if u16::from(frame.code) == 4403 && frame.reason == "revoked" =>
                    {
                        break;
                    }
                    Message::Close(frame) => {
                        anyhow::bail!("unexpected revocation close frame: {frame:?}");
                    }
                    _ => {}
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("revocation close frame was not received")??;
        wait_until(
            "playback the revoked controller started to stop",
            || async {
                let status = player.global_params.player.lock().await.get_status();
                Ok((status.state == PlayerPlaybackState::Paused).then_some(()))
            },
        )
        .await?;
        player.global_params.node.token.cancel();
        controller.global_params.node.token.cancel();
        Ok(())
    }

    /// Revoking a controller that did not start the playback leaves it alone; only the
    /// revocation of the controller that started it stops it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn playback_survives_revoking_a_controller_that_did_not_start_it() -> Result<()> {
        use futures::StreamExt;
        use playback::player::PlaybackState as PlayerPlaybackState;
        use tokio_tungstenite::tungstenite::Message;

        let player_dir = tempfile::tempdir()?;
        let starter_dir = tempfile::tempdir()?;
        let bystander_dir = tempfile::tempdir()?;
        let player = fixture(player_dir.path()).await?;
        let starter = fixture(starter_dir.path()).await?;
        let bystander = fixture(bystander_dir.path()).await?;
        let starter_fingerprint = info(&starter).await?.fingerprint;
        let bystander_fingerprint = info(&bystander).await?.fingerprint;
        let (_addr, mut socket) = controller_load_playing(&player, &starter).await?;
        // The player's server is already running from the starter's load, so the bystander is
        // only granted access both ways instead of pairing through the server start.
        pair(&player, &bystander).await?;
        pair(&bystander, &player).await?;
        // The bystander's removal is applied before the assertion reads the state.
        let mut changes = player
            .global_params
            .permission_manager
            .read()
            .await
            .changes();
        PermissionManager::new(player.global_params.config_path.as_str())?
            .remove_user(&bystander_fingerprint)
            .await?;
        tokio_time::timeout(TEST_DEADLINE, changes.changed())
            .await
            .context("bystander removal was not applied")??;
        let status = player.global_params.player.lock().await.get_status();
        assert_eq!(
            status.state,
            PlayerPlaybackState::Playing,
            "playback must survive revoking a controller that did not start it"
        );
        // Revoking the starter still stops the playback it started.
        player
            .global_params
            .permission_manager
            .read()
            .await
            .remove_user(&starter_fingerprint)
            .await?;
        tokio_time::timeout(TEST_DEADLINE, async {
            loop {
                match socket
                    .next()
                    .await
                    .context("socket closed without a revocation close frame")??
                {
                    Message::Close(Some(frame))
                        if u16::from(frame.code) == 4403 && frame.reason == "revoked" =>
                    {
                        break;
                    }
                    Message::Close(frame) => {
                        anyhow::bail!("unexpected revocation close frame: {frame:?}");
                    }
                    _ => {}
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("revocation close frame was not received")??;
        wait_until(
            "playback the revoked controller started to stop",
            || async {
                let status = player.global_params.player.lock().await.get_status();
                Ok((status.state == PlayerPlaybackState::Paused).then_some(()))
            },
        )
        .await?;
        player.global_params.node.token.cancel();
        starter.global_params.node.token.cancel();
        bystander.global_params.node.token.cancel();
        Ok(())
    }

    /// The playback stays the controller's own across a server restart: the persisted owner
    /// survives with the queue it names, so revoking the controller still stops the playback
    /// the restart resumed. Resuming what the queue already held is no new start and does not
    /// take the playback over.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn playback_stops_when_controller_revoked_after_server_restart() -> Result<()> {
        use futures::StreamExt;
        use playback::player::PlaybackState as PlayerPlaybackState;
        use tokio_tungstenite::tungstenite::Message;

        let player_dir = tempfile::tempdir()?;
        let controller_dir = tempfile::tempdir()?;
        let player = fixture(player_dir.path()).await?;
        let controller = fixture(controller_dir.path()).await?;
        let fingerprint = info(&controller).await?.fingerprint;
        let (_addr, socket) = controller_load_playing(&player, &controller).await?;
        drop(socket);
        // The server restarts; the persisted session holds the queue the controller loaded.
        let player = reopen_fixture(&player).await?;
        let addr = paired_full(&player, &controller).await?;
        // The user resumes the queue the restart restored; the playback is still the
        // controller's own, because a resume starts nothing new.
        let resume = PlayRequest { call_id: None };
        resume
            .handle(resume.extract_params(&player.global_params), None, &resume)
            .await?;
        wait_until("resumed playback after the restart", || async {
            let status = player.global_params.player.lock().await.get_status();
            Ok((status.state == PlayerPlaybackState::Playing).then_some(()))
        })
        .await?;
        let mut socket = connect_remote(
            "127.0.0.1",
            addr.port(),
            &controller.global_params.config_path,
            controller.global_params.cert_validator.read().await.clone(),
            &controller.global_params.node.sync_tokens,
        )
        .await?;
        player
            .global_params
            .permission_manager
            .read()
            .await
            .remove_user(&fingerprint)
            .await?;
        tokio_time::timeout(TEST_DEADLINE, async {
            loop {
                match socket
                    .next()
                    .await
                    .context("socket closed without a revocation close frame")??
                {
                    Message::Close(Some(frame))
                        if u16::from(frame.code) == 4403 && frame.reason == "revoked" =>
                    {
                        break;
                    }
                    Message::Close(frame) => {
                        anyhow::bail!("unexpected revocation close frame: {frame:?}");
                    }
                    _ => {}
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("revocation close frame was not received")??;
        wait_until(
            "playback the revoked controller started to stop",
            || async {
                let status = player.global_params.player.lock().await.get_status();
                Ok((status.state == PlayerPlaybackState::Paused).then_some(()))
            },
        )
        .await?;
        player.global_params.node.token.cancel();
        controller.global_params.node.token.cancel();
        Ok(())
    }

    /// An ownership change reaches the session store by itself, checked before any clean
    /// shutdown could flush it: the recording of the owner marks the store dirty even though
    /// the load issued no player command that would, and so does the revocation that releases
    /// the owner, which emits no player event at all.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn playback_owner_reaches_the_store_before_a_clean_shutdown() -> Result<()> {
        use discovery::access_policy::PeerIdentity;

        use crate::session::store::SessionState;

        let player_dir = tempfile::tempdir()?;
        let controller_dir = tempfile::tempdir()?;
        let player = fixture(player_dir.path()).await?;
        let controller = fixture(controller_dir.path()).await?;
        let fingerprint = info(&controller).await?.fingerprint;
        paired_full(&player, &controller).await?;
        let store_path =
            std::path::Path::new(&*player.global_params.config_path).join("sessions/local.json");
        let read_owner = |path: &std::path::Path| {
            SessionState::read(path)
                .ok()
                .and_then(|state| state.playback_owner.map(|owner| owner.fingerprint))
        };
        // The ownership is recorded without any load: the edit changes nothing and issues no
        // command, so no queue or status report can carry the owner to the store.
        player
            .global_params
            .node
            .session()?
            .edit_queue_with_load(
                None,
                Some(PeerIdentity {
                    fingerprint: fingerprint.clone(),
                    public_key: String::new(),
                }),
                |_player, _started_report| (false, true, false, None),
            )
            .await?;
        wait_until("the recorded owner to reach the session store", || async {
            Ok((read_owner(&store_path).as_deref() == Some(fingerprint.as_str())).then_some(()))
        })
        .await
        .context("recording the owner did not persist it before a clean shutdown")?;
        // The revocation releases the owner without any player event, and the release must
        // reach the store just the same.
        player
            .global_params
            .permission_manager
            .read()
            .await
            .remove_user(&fingerprint)
            .await?;
        wait_until("the released owner to leave the session store", || async {
            Ok((read_owner(&store_path).is_none()).then_some(()))
        })
        .await
        .context("releasing the owner did not persist before a clean shutdown")?;
        player.global_params.node.token.cancel();
        controller.global_params.node.token.cancel();
        Ok(())
    }

    /// A load whose explicit `EditPlaylist` completion fails hands the ownership back to the
    /// playback it interrupted: the owner is written before the load's commands go out, and
    /// only the command's own failure — not only the player thread's — may roll it back.
    /// Here the second controller's load asks for an index beyond the playlist it just
    /// replaced, so the player thread answers the command with `CannotApply` for certain.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn failed_load_hands_playback_ownership_back() -> Result<()> {
        use discovery::access_policy::PeerIdentity;
        use playback::player::{PlayingItem, PlaylistEdit, PlaylistLoad, PlaylistPlacement};

        let player_dir = tempfile::tempdir()?;
        let first_dir = tempfile::tempdir()?;
        let second_dir = tempfile::tempdir()?;
        let player = fixture(player_dir.path()).await?;
        let first = fixture(first_dir.path()).await?;
        let second = fixture(second_dir.path()).await?;
        let first_fingerprint = info(&first).await?.fingerprint;
        let second_fingerprint = info(&second).await?.fingerprint;
        paired_full(&player, &first).await?;
        // The player's server is already running, so the second controller is only granted
        // access both ways instead of pairing through the server start.
        pair(&player, &second).await?;
        pair(&second, &player).await?;
        let session = player.global_params.node.session()?;
        let owner_of = |fingerprint: &str| {
            Some(PeerIdentity {
                fingerprint: fingerprint.to_owned(),
                public_key: String::new(),
            })
        };
        // The first controller owns the playback.
        session
            .edit_queue_with_load(
                None,
                owner_of(&first_fingerprint),
                |_player, _started_report| (false, true, false, None),
            )
            .await?;
        assert_eq!(
            session
                .playback_owner_for_test()
                .await
                .map(|owner| owner.fingerprint),
            Some(first_fingerprint.clone())
        );
        // The second controller's load goes out and fails its own completion: the track list
        // is replaced with one entry while the load asks for index 99.
        let error = session
            .edit_queue_with_load(
                None,
                owner_of(&second_fingerprint),
                |player, _started_report| {
                    let completion = Some(player.command_with_completion(
                        playback::PlayerCommand::EditPlaylist(
                            PlaylistEdit {
                                tracks: vec![(
                                    PlayingItem::IndependentFile("missing-load-track.ogg".into()),
                                    playback::player::PlaybackSource::Local(
                                        "missing-load-track.ogg".into(),
                                    ),
                                )],
                                placement: PlaylistPlacement::Replace,
                                playback_mode: None,
                                load: Some(PlaylistLoad {
                                    start_index: 99,
                                    position: Duration::ZERO,
                                    play: true,
                                }),
                                block: None,
                                started_playback: None,
                            },
                            None,
                        ),
                    ));
                    (true, true, true, completion)
                },
            )
            .await
            .expect_err("a load past the end of the playlist cannot apply");
        assert!(
            error.chain().any(
                |cause| cause.downcast_ref::<ExpectedRequestError>().is_some_and(
                    |expected| matches!(expected, ExpectedRequestError::PlayerCommandCannotApply)
                )
            ),
            "the failing load should surface the player command error: {error:?}"
        );
        // The ownership is the first controller's again, not the failed load's.
        assert_eq!(
            session
                .playback_owner_for_test()
                .await
                .map(|owner| owner.fingerprint),
            Some(first_fingerprint.clone()),
            "the failed load must hand the ownership back"
        );
        player.global_params.node.token.cancel();
        first.global_params.node.token.cancel();
        second.global_params.node.token.cancel();
        Ok(())
    }

    /// A load whose controller loses control access while the load is still in flight must not
    /// leave playback running ownerless: the revocation pass over the owner has no owner to see
    /// yet when the load's commands go out, so the ownership write itself pauses the playback
    /// behind them. Here the revocation is applied inside the edit closure, after the load's
    /// command has gone out and before the ownership is written, which is exactly the landing
    /// the revocation pass cannot cover.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn playback_pauses_when_starting_controller_is_revoked_mid_load() -> Result<()> {
        use crate::access_policy::Action;
        use discovery::access_policy::AccessPolicy;
        use discovery::access_policy::PeerIdentity;
        use playback::player::{
            PlaybackState as PlayerPlaybackState, PlayingItem, PlaylistEdit, PlaylistLoad,
            PlaylistPlacement,
        };

        let player_dir = tempfile::tempdir()?;
        let controller_dir = tempfile::tempdir()?;
        let player = fixture(player_dir.path()).await?;
        let controller = fixture(controller_dir.path()).await?;
        let fingerprint = info(&controller).await?.fingerprint;
        paired_full(&player, &controller).await?;
        let session = player.global_params.node.session()?;
        let track_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");
        let track = (
            PlayingItem::IndependentFile(track_path.to_string_lossy().into_owned()),
            playback::player::PlaybackSource::Local(track_path),
        );
        let access_policy = player.global_params.node.access_policy.clone();
        let peer = PeerIdentity {
            fingerprint: fingerprint.clone(),
            public_key: String::new(),
        };
        // The controller still controls its playback when the load begins; the access is cut
        // after the load's command is already on its way to the player thread.
        assert!(
            access_policy.check(&peer, &Action::Control).is_ok(),
            "the paired controller must control the player before the load"
        );
        let revoke_fingerprint = fingerprint.clone();
        let edit_result = session
            .edit_queue_with_load(None, Some(peer), move |player, _started_report| {
                let completion = Some(player.command_with_completion(
                    playback::PlayerCommand::EditPlaylist(
                        PlaylistEdit {
                            tracks: vec![track],
                            placement: PlaylistPlacement::Replace,
                            playback_mode: None,
                            load: Some(PlaylistLoad {
                                start_index: 0,
                                position: Duration::ZERO,
                                play: true,
                            }),
                            block: None,
                            started_playback: None,
                        },
                        None,
                    ),
                ));
                let mut grants = access_policy
                    .grants(&revoke_fingerprint)
                    .expect("paired controller grants");
                grants.control = false;
                access_policy
                    .set(&revoke_fingerprint, grants)
                    .expect("revoking the controller mid-load");
                (true, true, true, completion)
            })
            .await;
        // Which answer the edit gets depends on where the revocation pass lands: a pause that
        // arrives while the track still loads supersedes the play-through completion, one that
        // arrives after it came up does not. Anything but those two answers is a failure.
        if let Err(error) = &edit_result {
            assert!(
                format!("{error:?}").contains("superseded"),
                "the edit may only fail superseded by the revocation pause, got: {error:?}"
            );
        }
        // Whichever side the pause landed on, the playback the revoked controller's load
        // started settles paused on the track the load put up — never playing, and never the
        // interrupted playback from before it — and the revoked controller owns nothing.
        wait_until(
            "in-flight revoked playback to pause on the loaded track",
            || async {
                let status = player.global_params.player.lock().await.get_status();
                Ok(
                    (status.state == PlayerPlaybackState::Paused && status.index == Some(0))
                        .then_some(()),
                )
            },
        )
        .await?;
        wait_until("the revoked controller to own nothing", || async {
            Ok((session.playback_owner_for_test().await.is_none()).then_some(()))
        })
        .await?;
        player.global_params.node.token.cancel();
        controller.global_params.node.token.cancel();
        Ok(())
    }

    /// A controller whose access is already revoked when its load reaches the session is
    /// refused before anything is dispatched: the revocation pass over the owner ran before
    /// the edit could write its ownership, so no pause after the fact could ever catch the
    /// playback the load would start. The edit must fail the request, send nothing to the
    /// player, and move neither the queue nor the ownership.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn revoked_controller_load_is_refused_before_dispatch() -> Result<()> {
        use crate::access_policy::Action;
        use discovery::access_policy::AccessPolicy;
        use discovery::access_policy::PeerIdentity;
        use playback::player::{PlayingItem, PlaylistEdit, PlaylistLoad, PlaylistPlacement};

        let player_dir = tempfile::tempdir()?;
        let controller_dir = tempfile::tempdir()?;
        let player = fixture(player_dir.path()).await?;
        let controller = fixture(controller_dir.path()).await?;
        let fingerprint = info(&controller).await?.fingerprint;
        paired_full(&player, &controller).await?;
        let session = player.global_params.node.session()?;
        let track_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");
        let track = (
            PlayingItem::IndependentFile(track_path.to_string_lossy().into_owned()),
            playback::player::PlaybackSource::Local(track_path),
        );
        let access_policy = player.global_params.node.access_policy.clone();
        let peer = PeerIdentity {
            fingerprint: fingerprint.clone(),
            public_key: String::new(),
        };
        assert!(
            access_policy.check(&peer, &Action::Control).is_ok(),
            "the paired controller must control the player before the revocation"
        );
        let mut grants = access_policy
            .grants(&fingerprint)
            .expect("paired controller grants");
        grants.control = false;
        access_policy
            .set(&fingerprint, grants)
            .expect("revoking the controller before its load");
        let queue_revision_before = session.queue_revision().await;
        let edit_result = session
            .edit_queue_with_load(None, Some(peer), move |player, _started_report| {
                let completion = Some(player.command_with_completion(
                    playback::PlayerCommand::EditPlaylist(
                        PlaylistEdit {
                            tracks: vec![track],
                            placement: PlaylistPlacement::Replace,
                            playback_mode: None,
                            load: Some(PlaylistLoad {
                                start_index: 0,
                                position: Duration::ZERO,
                                play: true,
                            }),
                            block: None,
                            started_playback: None,
                        },
                        None,
                    ),
                ));
                (true, true, true, completion)
            })
            .await;
        let error = edit_result.expect_err("a revoked controller's load must never dispatch");
        assert!(
            format!("{error:?}").contains("denied") || format!("{error:?}").contains("access"),
            "the refusal should be the access policy's: {error:?}"
        );
        assert_eq!(
            session.queue_revision().await,
            queue_revision_before,
            "a refused edit must not move the queue revision"
        );
        assert!(
            session.playback_owner_for_test().await.is_none(),
            "a refused edit must not write the ownership"
        );
        assert!(
            player
                .global_params
                .player
                .lock()
                .await
                .get_playlist()
                .is_empty(),
            "a refused edit must not reach the player's queue"
        );
        player.global_params.node.token.cancel();
        controller.global_params.node.token.cancel();
        Ok(())
    }

    /// How a controller that stopped reading ends its session.
    #[derive(Clone, Copy)]
    enum StalledControllerEnd {
        /// The controller sends its Close frame and still does not read.
        PeerClose,
        /// The server removes the controller's approval.
        Revocation,
    }

    /// A controller that never reads fills the connection's outgoing queue.
    /// Shutting the server down must still close that connection within the
    /// shutdown deadline, without the controller reading anything.
    async fn shutdown_releases_stalled_controller(end: StalledControllerEnd) -> Result<()> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Instant;

        // Frames the connection queues for its socket writer.
        const OUTGOING_QUEUE_CAPACITY: usize = 32;
        // Larger than the loopback send and receive buffers together, so the
        // socket writer cannot finish this frame while the controller does not read.
        const STALLING_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
        const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(8);

        let renderer_dir = tempfile::tempdir()?;
        let controller_dir = tempfile::tempdir()?;
        let renderer = fixture(renderer_dir.path()).await?;
        let controller = fixture(controller_dir.path()).await?;
        let service = &renderer.global_params.node.websocket_service;
        service
            .override_handler("StallSettle", |_, _, _| async {
                ("StallSettleResponse".into(), Ok(Vec::new()))
            })
            .await;
        service
            .override_handler("StallWriter", |_, _, _| async {
                (
                    "StallWriterResponse".into(),
                    Ok(vec![0; STALLING_RESPONSE_BYTES]),
                )
            })
            .await;
        let filled = Arc::new(AtomicUsize::new(0));
        let filled_handler = filled.clone();
        service
            .override_handler("StallFill", move |_, _, _| {
                filled_handler.fetch_add(1, Ordering::SeqCst);
                async { ("StallFillResponse".into(), Ok(Vec::new())) }
            })
            .await;

        let addr = paired_full(&renderer, &controller).await?;
        let fingerprint = info(&controller).await?.fingerprint;
        let mut socket = connect_remote(
            "127.0.0.1",
            addr.port(),
            &controller.global_params.config_path,
            controller.global_params.cert_validator.read().await.clone(),
            &controller.global_params.node.sync_tokens,
        )
        .await?;
        // One answered request flushes the frames sent when the connection opened.
        socket_request(&mut socket, "StallSettle", &[]).await?;

        socket
            .send(Message::Binary(
                encode_message("StallWriter", &[], Some(Uuid::new_v4())).into(),
            ))
            .await?;
        for _ in 0..OUTGOING_QUEUE_CAPACITY + 4 {
            socket
                .send(Message::Binary(
                    encode_message("StallFill", &[], Some(Uuid::new_v4())).into(),
                ))
                .await?;
        }
        // Requests are handled one at a time, and each response is queued before
        // the next request is read. Entering one more fill handler than the queue
        // holds proves the queue is full behind the frame the writer cannot finish.
        wait_until("the outgoing queue to fill", || {
            let entered = filled.load(Ordering::SeqCst);
            async move { Ok((entered > OUTGOING_QUEUE_CAPACITY).then_some(())) }
        })
        .await?;

        match end {
            StalledControllerEnd::PeerClose => socket.send(Message::Close(None)).await?,
            StalledControllerEnd::Revocation => {
                renderer
                    .global_params
                    .permission_manager
                    .read()
                    .await
                    .remove_user(&fingerprint)
                    .await?;
                wait_until("revoked controller to detach", || async {
                    Ok(renderer
                        .global_params
                        .node
                        .session()?
                        .attached_controllers()
                        .iter()
                        .all(|controller| controller.fingerprint != fingerprint)
                        .then_some(()))
                })
                .await?;
            }
        }

        let started = Instant::now();
        tokio::time::timeout(SHUTDOWN_DEADLINE, renderer.global_params.node.shutdown())
            .await
            .context("server shutdown did not finish within the deadline")??;
        let MaybeTlsStream::Rustls(tls) = socket.get_ref() else {
            anyhow::bail!("controller connection is not TLS");
        };
        let tcp = tls.get_ref().0;
        // A raw write bypasses the websocket, which refuses sends after its own
        // Close. Once the server closes its socket, the next write fails with a reset.
        let released = wait_until(
            "server to close the stalled connection",
            move || async move {
                match tcp.try_write(b"x") {
                    Ok(_) => Ok(None),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
                    Err(_) => Ok(Some(())),
                }
            },
        );
        tokio::time::timeout(
            SHUTDOWN_DEADLINE.saturating_sub(started.elapsed()),
            released,
        )
        .await
        .with_context(|| {
            format!(
                "server kept the stalled connection open {SHUTDOWN_DEADLINE:?} after shutdown \
                 began ({} fill requests handled)",
                filled.load(Ordering::SeqCst)
            )
        })??;

        drop(socket);
        controller.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shutdown_releases_stalled_controller_after_peer_close() -> Result<()> {
        shutdown_releases_stalled_controller(StalledControllerEnd::PeerClose).await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shutdown_releases_stalled_controller_after_revocation() -> Result<()> {
        shutdown_releases_stalled_controller(StalledControllerEnd::Revocation).await
    }

    async fn socket_request(
        socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
        name: &str,
        payload: &[u8],
    ) -> Result<(String, Vec<u8>)> {
        use crate::backends::remote::encode_message;
        use futures::SinkExt;
        use tokio_tungstenite::tungstenite::Message;

        let id = Uuid::new_v4();
        socket
            .send(Message::Binary(
                encode_message(name, payload, Some(id)).into(),
            ))
            .await?;
        next_socket_response(socket, id).await
    }

    async fn next_socket_response(
        socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
        request_id: Uuid,
    ) -> Result<(String, Vec<u8>)> {
        use crate::backends::remote::decode_message;
        use anyhow::Context;
        use futures::StreamExt;

        tokio_time::timeout(TEST_DEADLINE, async {
            loop {
                let frame = socket.next().await.context("socket closed")??;
                let (response_name, payload, response_id) =
                    decode_message(&frame.into_data()).context("invalid frame")?;
                if response_name == "RequestCapabilities" {
                    continue;
                }
                if response_id == request_id {
                    return Ok((response_name, payload));
                }
            }
        })
        .await?
    }

    async fn socket_request_for_library(
        socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
        name: &str,
        payload: &[u8],
        library_id: Uuid,
    ) -> Result<(String, Vec<u8>)> {
        use crate::backends::remote::encode_request;
        use futures::SinkExt;
        use tokio_tungstenite::tungstenite::Message;

        let id = Uuid::new_v4();
        socket
            .send(Message::Binary(
                encode_request(name, payload, id, library_id).into(),
            ))
            .await?;
        next_socket_response(socket, id).await
    }

    async fn socket_request_for_catalogue_scope(
        socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
        name: &str,
        payload: &[u8],
        library_id: Uuid,
    ) -> Result<(String, Vec<u8>)> {
        if request_scope(name) == Some(RequestScope::Library) {
            socket_request_for_library(socket, name, payload, library_id).await
        } else {
            socket_request(socket, name, payload).await
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn collections_by_id_bake_covers_only_when_requested() -> Result<()> {
        use crate::messages::{
            CollectionType, FetchCollectionByIdsRequest, FetchCollectionByIdsResponse,
        };
        use crate::rpc::dispatch_gui_request;
        use database::{
            actions::cover_art::assign_inspected, entities::portable_artists as artists,
        };

        let owner_dir = tempfile::tempdir()?;
        let peer_dir = tempfile::tempdir()?;
        let owner = fixture(owner_dir.path()).await?;
        let peer = fixture(peer_dir.path()).await?;
        let library = owner
            .global_params
            .library
            .as_ref()
            .context("owner library")?;
        let library_id = library.library_id;
        let track = scan_held_track(&owner, library_id, "covered.ogg").await?;
        let cover = include_bytes!("../../../../assets/demo_cover.webp");
        assign_inspected(
            &library.main_db,
            &library.library_sync.record_store(),
            &track,
            Some(cover_art::CoverArt {
                crc: String::new(),
                data: cover.to_vec(),
                primary_color: 42,
            }),
        )
        .await?;
        let artist = artists::Entity::find()
            .filter(artists::Column::Name.eq("Shara"))
            .one(&*library.main_db)
            .await?
            .context("artist from startup_0.ogg")?;

        let address = paired_full(&owner, &peer).await?;
        let mut socket = connect_remote(
            "127.0.0.1",
            address.port(),
            &peer.global_params.config_path,
            peer.global_params.cert_validator.read().await.clone(),
            &peer.global_params.node.sync_tokens,
        )
        .await?;
        for bake_cover_arts in [true, false] {
            let request = FetchCollectionByIdsRequest {
                collection_type: CollectionType::Artist,
                bake_cover_arts,
                ids: vec![artist.id],
                call_id: None,
            };
            let local = dispatch_gui_request(&owner.global_params, &request)
                .await?
                .context("local by-ID response")?;
            assert_eq!(local.result.len(), 1);
            let local_covers = &local.result[0].cover_art_map;
            if bake_cover_arts {
                assert_eq!(local_covers.len(), 1, "expected a baked local cover");
                assert_eq!(fs::read(&local_covers[&track.id])?.as_slice(), cover);
            } else {
                assert!(local_covers.is_empty());
            }

            let (name, payload) = socket_request_for_library(
                &mut socket,
                "FetchCollectionByIdsRequest",
                &rinf::serialize(&request)?,
                library_id,
            )
            .await?;
            assert_eq!(name, "FetchCollectionByIdsResponse");
            let remote: FetchCollectionByIdsResponse = rinf::deserialize(&payload)?;
            assert_eq!(remote.result.len(), 1);
            let remote_covers = &remote.result[0].cover_art_map;
            if bake_cover_arts {
                let digest = blake3::hash(cover).to_hex();
                assert_eq!(
                    remote_covers.get(&track.id),
                    Some(&format!(
                        "https://127.0.0.1:{}/files/cache/{digest}?library_id={library_id}",
                        address.port()
                    ))
                );
            } else {
                assert!(remote_covers.is_empty());
            }
        }
        socket.close(None).await?;
        owner.global_params.node.shutdown().await?;
        peer.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn controller_cover_urls_name_the_port_it_connected_to() -> Result<()> {
        use database::entities::media_cover_art;
        use sea_orm::{ActiveModelTrait, IntoActiveModel, Set};

        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let renderer = fixture(first.path()).await?;
        let controller = fixture(second.path()).await?;
        let renderer_id = renderer
            .global_params
            .library_id
            .context("renderer library")?;
        let track = scan_held_track(&renderer, renderer_id, "covered.ogg").await?;
        let library = renderer
            .global_params
            .library_pool
            .acquire(renderer_id)
            .await?;
        let cover_bytes = b"cover bytes for the port test".to_vec();
        let digest = blake3::hash(&cover_bytes).to_hex().to_string();
        let cover_uuid = catalog_identity::artwork(&digest).to_string();
        media_cover_art::ActiveModel {
            id: sea_orm::NotSet,
            file_hash: Set(String::new()),
            binary: Set(cover_bytes.clone()),
            primary_color: Set(None),
            content_hash: Set(Some(digest.clone())),
            content_size: Set(Some(i64::try_from(cover_bytes.len())?)),
            local_available: Set(true),
            source_uuid: Set(cover_uuid.clone()),
        }
        .insert(&*library.main_db)
        .await?;
        let mut file = track.clone().into_active_model();
        file.cover_art_uuid = Set(Some(cover_uuid));
        file.update(&*library.main_db).await?;

        let addr = paired_full(&renderer, &controller).await?;
        assert_ne!(
            addr.port(),
            endpoint::DEFAULT_DEVICE_PORT,
            "the test must run on a port other than the default"
        );
        let validator = controller.global_params.cert_validator.read().await.clone();
        let mut socket = connect_remote(
            "127.0.0.1",
            addr.port(),
            &controller.global_params.config_path,
            validator,
            &controller.global_params.node.sync_tokens,
        )
        .await?;
        let request = rinf::serialize(&FetchMediaFilesRequest {
            cursor: 0,
            page_size: 10,
            bake_cover_arts: true,
            call_id: None,
        })?;
        let (name, payload) = socket_request_for_library(
            &mut socket,
            "FetchMediaFilesRequest",
            &request,
            renderer_id,
        )
        .await?;
        assert_eq!(name, "FetchMediaFilesResponse");
        let response: FetchMediaFilesResponse = rinf::deserialize(&payload)?;
        assert_eq!(
            response.cover_art_map.get(&track.id),
            Some(&format!(
                "https://127.0.0.1:{}/files/cache/{digest}?library_id={renderer_id}",
                addr.port()
            ))
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_server_announces_itself_as_a_server_with_the_app_version() -> Result<()> {
        use crate::server::api::fetch_device_info_at_port;
        use crate::server::utils::device::APP_VERSION;
        use discovery::protocol::announcement_message;
        use discovery::utils::DeviceType;

        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let server = fixture_with_player(
            first.path(),
            Uuid::new_v4(),
            Duration::from_secs(300),
            Arc::new(Quiet),
            ReconnectConfig::default(),
            RunMode::Server,
            crate::session::PlayerSession::real_player(),
        )
        .await?;
        let viewer = fixture(second.path()).await?;
        let addr = paired_full(&server, &viewer).await?;

        // The device information the viewer fetches over its pinned connection.
        let validator = viewer.global_params.cert_validator.read().await.clone();
        let fetched = fetch_device_info_at_port(
            "127.0.0.1",
            addr.port(),
            Arc::new(Arc::new(validator).into_client_config()),
        )
        .await?;
        assert_eq!(fetched.device_type, DeviceType::Server);
        assert_eq!(fetched.version, APP_VERSION);

        // The multicast announcement the viewer lists the device from.
        let announced = announcement_message(&server.global_params.node.device_info().await?);
        viewer
            .global_params
            .device_scanner
            .observe_announcement(
                &serde_json::to_vec(&announced)?,
                "192.0.2.10:57863".parse()?,
                None,
            )
            .await?;
        let request = GetDiscoveredDeviceRequest { call_id: None };
        let response = request
            .handle(
                Arc::clone(&viewer.global_params.device_scanner),
                None,
                &request,
            )
            .await?
            .context("discovered devices")?;
        assert_eq!(response.devices.len(), 1);
        assert_eq!(response.devices[0].device_type, DeviceTypeMessage::Server);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn addressed_websocket_requests_select_and_authorize_the_named_library() -> Result<()> {
        use crate::Session;
        use crate::access_policy::{ConnectionLevel, DeviceGrants, LibraryGrants};
        use crate::backends::remote::connect_remote;
        use crate::messages::{
            CreatePlaylistRequest, RemotePlaylistRevisionRequest, SessionSnapshotRequest,
        };

        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let renderer = fixture(first.path()).await?;
        let controller = fixture(second.path()).await?;
        let renderer_id = renderer
            .global_params
            .library_id
            .context("renderer library")?;
        scan_held_track(&renderer, renderer_id, "renderer-one.ogg").await?;
        scan_held_track(&renderer, renderer_id, "renderer-two.ogg").await?;
        let other_id = held_owned_library(&renderer, &first.path().join("other-library")).await?;
        scan_held_track(&renderer, other_id, "other.ogg").await?;

        pair(&renderer, &controller).await?;
        let controller_fingerprint = info(&controller).await?.fingerprint;
        let addressed_grants = |library_id, connection| DeviceGrants {
            control: false,
            connection: LibraryGrants {
                all_libraries: None,
                libraries: [(library_id, connection)].into_iter().collect(),
            },
            sync: LibraryGrants {
                all_libraries: None,
                libraries: Default::default(),
            },
            administer: false,
        };
        let user = renderer
            .global_params
            .permission_manager
            .read()
            .await
            .verify_by_fingerprint(&controller_fingerprint)
            .await
            .context("paired controller user")?;
        let peer_session = Session {
            fingerprint: user.fingerprint,
            public_key: user.public_key,
            base_url: Url::parse("https://127.0.0.2:7863/").unwrap(),
        };
        renderer.global_params.node.access_policy.set(
            &controller_fingerprint,
            addressed_grants(other_id, ConnectionLevel::View),
        )?;
        renderer
            .global_params
            .node
            .authorize_request(
                &peer_session,
                "GetMediaFilesCountRequest",
                None,
                Some(other_id),
            )
            .await?;
        renderer.global_params.node.access_policy.set(
            &controller_fingerprint,
            addressed_grants(renderer_id, ConnectionLevel::View),
        )?;
        let denied = renderer
            .global_params
            .node
            .authorize_request(
                &peer_session,
                "GetMediaFilesCountRequest",
                None,
                Some(other_id),
            )
            .await
            .expect_err("a read grant on A must not authorize a read addressed to B");
        assert_eq!(classify(&denied), RequestErrorKind::PermissionDenied);

        renderer.global_params.node.access_policy.set(
            &controller_fingerprint,
            DeviceGrants {
                control: true,
                connection: LibraryGrants {
                    all_libraries: None,
                    libraries: [
                        (other_id, ConnectionLevel::View),
                        (renderer_id, ConnectionLevel::View),
                    ]
                    .into_iter()
                    .collect(),
                },
                sync: LibraryGrants {
                    all_libraries: None,
                    libraries: Default::default(),
                },
                administer: false,
            },
        )?;

        let addr = paired_full(&renderer, &controller).await?;
        let validator = controller.global_params.cert_validator.read().await.clone();
        let mut socket = connect_remote(
            "127.0.0.1",
            addr.port(),
            &controller.global_params.config_path,
            validator,
            &controller.global_params.node.sync_tokens,
        )
        .await?;
        let count_request = rinf::serialize(&GetMediaFilesCountRequest { call_id: None })?;

        // With the UI on the renderer's first library, the explicit address serves the other.
        let (name, payload) = socket_request_for_library(
            &mut socket,
            "GetMediaFilesCountRequest",
            &count_request,
            other_id,
        )
        .await?;
        assert_eq!(name, "GetMediaFilesCountResponse");
        let response: GetMediaFilesCountResponse = rinf::deserialize(&payload)?;
        assert_eq!(response.count, 1);

        // A library-scoped frame without an address is rejected before decoding or dispatch.
        let (name, payload) =
            socket_request(&mut socket, "GetMediaFilesCountRequest", &count_request).await?;
        assert_eq!(name, "RemoteRequestFailed");
        let failure: RemoteRequestFailed = rinf::deserialize(&payload)?;
        assert_eq!(failure.kind, RequestErrorKind::Unavailable);
        let (name, payload) = socket_request(&mut socket, "GetMediaFilesCountRequest", &[]).await?;
        assert_eq!(name, "RemoteRequestFailed");
        let failure: RemoteRequestFailed = rinf::deserialize(&payload)?;
        assert_eq!(failure.kind, RequestErrorKind::Unavailable);

        let session_request = rinf::serialize(&SessionSnapshotRequest { call_id: None })?;
        let (name, session_result) = dispatch_remote(
            "SessionSnapshotRequest",
            &session_request,
            Some(Session {
                fingerprint: peer_session.fingerprint.clone(),
                public_key: peer_session.public_key.clone(),
                base_url: peer_session.base_url.clone(),
            }),
            None,
            &renderer.global_params.node,
        )
        .await
        .context("session-scoped request dispatch")?;
        assert_eq!(name, "SessionSnapshotResponse");
        session_result?;
        let node_request = rinf::serialize(&RemotePlaylistRevisionRequest {
            fingerprint: "not-a-trusted-device".into(),
            call_id: None,
        })?;
        let (_, node_result) = dispatch_remote(
            "RemotePlaylistRevisionRequest",
            &node_request,
            None,
            None,
            &renderer.global_params.node,
        )
        .await
        .context("node-scoped request dispatch")?;
        let node_error = node_result.expect_err("unknown renderer must fail normally");
        assert_eq!(classify(&node_error), RequestErrorKind::NotFound);

        // A write against a read-only addressed library is denied.
        renderer.global_params.node.access_policy.set(
            &controller_fingerprint,
            DeviceGrants {
                control: true,
                connection: LibraryGrants {
                    all_libraries: None,
                    libraries: [(other_id, ConnectionLevel::View)].into_iter().collect(),
                },
                sync: LibraryGrants {
                    all_libraries: None,
                    libraries: Default::default(),
                },
                administer: false,
            },
        )?;
        let write = rinf::serialize(&CreatePlaylistRequest {
            call_id: None,
            name: "denied".into(),
            group: "test".into(),
        })?;
        let (name, payload) =
            socket_request_for_library(&mut socket, "CreatePlaylistRequest", &write, other_id)
                .await?;
        assert_eq!(name, "RemoteRequestFailed");
        let failure: RemoteRequestFailed = rinf::deserialize(&payload)?;
        assert_eq!(failure.kind, RequestErrorKind::PermissionDenied);

        let missing_id = Uuid::new_v4();
        let (name, payload) = socket_request_for_library(
            &mut socket,
            "GetMediaFilesCountRequest",
            &count_request,
            missing_id,
        )
        .await?;
        assert_eq!(name, "RemoteRequestFailed");
        let failure: RemoteRequestFailed = rinf::deserialize(&payload)?;
        // An ungranted address is rejected before the server reveals whether it exists.
        assert_eq!(failure.kind, RequestErrorKind::PermissionDenied);

        socket.close(None).await?;
        renderer.manager.stop().await?;
        renderer.global_params.node.token.cancel();
        controller.global_params.node.token.cancel();
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn access_policy_guest_websocket() -> Result<()> {
        use crate::access_policy::{ConnectionLevel, DeviceGrants};
        use crate::backends::remote::connect_remote;
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let original = fixture(first.path()).await?;
        let b = fixture(second.path()).await?;
        pair(&original, &b).await?;
        let fingerprint = info(&b).await?.fingerprint;
        fs::write(
            first.path().join("grants.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": 1, "devices": {fingerprint.clone(): {
                    "control": true, "all_libraries": null, "libraries": {}, "administer": false
                }}
            }))?,
        )?;
        let a = reopen_fixture(&original).await?;
        // An old-format grants file is reset to the least-privileged preset.
        assert_eq!(
            a.global_params.node.access_policy.grants(&fingerprint)?,
            DeviceGrants::guest()
        );
        a.global_params
            .node
            .access_policy
            .set(&fingerprint, DeviceGrants::guest())?;
        let addr = paired_full(&a, &b).await?;
        let validator = b.global_params.cert_validator.read().await.clone();
        let mut socket = connect_remote(
            "127.0.0.1",
            addr.port(),
            &b.global_params.config_path,
            validator,
            &b.global_params.node.sync_tokens,
        )
        .await?;
        let renderer_id = a.global_params.library_id.context("renderer library")?;
        let (name, payload) = socket_request_for_library(
            &mut socket,
            "GetMediaFilesCountRequest",
            &rinf::serialize(&GetMediaFilesCountRequest { call_id: None })?,
            renderer_id,
        )
        .await?;
        assert_eq!(name, "RemoteRequestFailed");
        let failure: RemoteRequestFailed = rinf::deserialize(&payload)?;
        assert_eq!(failure.kind, RequestErrorKind::PermissionDenied);

        let mut shared_guest = DeviceGrants::guest();
        shared_guest
            .connection
            .libraries
            .insert(renderer_id, ConnectionLevel::View);
        a.global_params
            .node
            .access_policy
            .set(&fingerprint, shared_guest)?;
        let (name, _) = socket_request_for_library(
            &mut socket,
            "GetMediaFilesCountRequest",
            &rinf::serialize(&GetMediaFilesCountRequest { call_id: None })?,
            renderer_id,
        )
        .await?;
        assert_eq!(name, "GetMediaFilesCountResponse");
        let (name, _) = socket_request(
            &mut socket,
            "PauseRequest",
            &rinf::serialize(&PauseRequest { call_id: None })?,
        )
        .await?;
        assert_eq!(name, "");
        for (request, payload) in [
            (
                "CreatePlaylistRequest",
                rinf::serialize(&CreatePlaylistRequest {
                    call_id: None,
                    name: "forbidden".into(),
                    group: "test".into(),
                })?,
            ),
            (
                "ScanAudioLibraryRequest",
                rinf::serialize(&ScanAudioLibraryRequest {
                    call_id: None,
                    force: false,
                })?,
            ),
        ] {
            let (name, payload) =
                socket_request_for_catalogue_scope(&mut socket, request, &payload, renderer_id)
                    .await?;
            assert_eq!(name, "RemoteRequestFailed");
            let failure: RemoteRequestFailed = rinf::deserialize(&payload)?;
            assert_eq!(
                failure.kind,
                RequestErrorKind::PermissionDenied,
                "{request}"
            );
        }
        socket.close(None).await?;
        a.global_params.node.shutdown().await?;
        b.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn missing_playlist_handler_is_not_found() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let manager = fixture(dir.path()).await?;
        let request = GetPlaylistByIdRequest {
            call_id: None,
            playlist_id: -1,
        };
        let result = request
            .handle(
                Some(manager.global_params.test_library().clone()),
                None,
                &request,
            )
            .await;
        manager.global_params.node.token.cancel();
        let error = result
            .err()
            .context("missing playlist unexpectedly succeeded")?;
        assert_eq!(classify(&error), RequestErrorKind::NotFound);
        Ok(())
    }

    #[tokio::test]
    async fn missing_mix_handler_is_not_found() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let manager = fixture(dir.path()).await?;
        let request = GetMixByIdRequest {
            call_id: None,
            mix_id: -1,
        };
        let result = request
            .handle(
                Some(manager.global_params.test_library().clone()),
                None,
                &request,
            )
            .await;
        manager.global_params.node.token.cancel();
        let error = result.err().context("missing mix unexpectedly succeeded")?;
        assert_eq!(classify(&error), RequestErrorKind::NotFound);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn remote_request_failures_end_to_end() -> Result<()> {
        use crate::backends::remote::connect_remote;

        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let a = fixture(first.path()).await?;
        let b = fixture(second.path()).await?;
        let addr = paired_full(&a, &b).await?;
        let validator = b.global_params.cert_validator.read().await.clone();
        let mut socket = connect_remote(
            "127.0.0.1",
            addr.port(),
            &b.global_params.config_path,
            validator,
            &b.global_params.node.sync_tokens,
        )
        .await?;
        let library_id = a.global_params.library_id.context("library")?;
        for (name, payload) in [
            ("FetchParsedMediaFileRequest", vec![]),
            (
                "GetPlaylistByIdRequest",
                rinf::serialize(&GetPlaylistByIdRequest {
                    call_id: None,
                    playlist_id: -1,
                })?,
            ),
            (
                "GetMediaFilesCountRequest",
                rinf::serialize(&GetMediaFilesCountRequest { call_id: None })?,
            ),
        ] {
            let (response_name, payload) =
                socket_request_for_catalogue_scope(&mut socket, name, &payload, library_id).await?;
            if name == "GetMediaFilesCountRequest" {
                assert_eq!(response_name, "GetMediaFilesCountResponse");
                let response: GetMediaFilesCountResponse = rinf::deserialize(&payload)?;
                assert_eq!(response.count, 0);
            } else {
                assert_eq!(response_name, "RemoteRequestFailed");
                let failure: RemoteRequestFailed = rinf::deserialize(&payload)?;
                assert_eq!(failure.request, name);
                if name == "GetPlaylistByIdRequest" {
                    assert_eq!(failure.kind, RequestErrorKind::NotFound);
                    assert!(failure.detail.contains("Playlist not found with id: -1"));
                } else {
                    assert_eq!(failure.kind, RequestErrorKind::Internal);
                    assert!(failure.detail.contains("Failed to deserialize request"));
                }
            }
        }
        socket.close(None).await?;
        a.stop().await?;
        a.global_params.node.token.cancel();
        b.global_params.node.token.cancel();
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn remote_control_and_file_fetch_end_to_end() -> Result<()> {
        use crate::backends::remote::{connect_remote, encode_message, encode_request};
        use crate::handlers::neighbors::authorized_get;
        use crate::request_error::classify;
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        use url::Url;

        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let a = fixture(first.path()).await?;
        let b = fixture(second.path()).await?;
        let addr = paired_full(&a, &b).await?;
        let validator = b.global_params.cert_validator.read().await.clone();
        let config = &b.global_params.config_path;
        let mut socket = connect_remote(
            "127.0.0.1",
            addr.port(),
            config,
            validator.clone(),
            &b.global_params.node.sync_tokens,
        )
        .await?;
        let library_id = a.global_params.library_id.context("library")?;
        async fn count(
            socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
            library_id: Uuid,
        ) -> Result<()> {
            use crate::backends::remote::decode_message;
            use futures::{SinkExt, StreamExt};
            use tokio_tungstenite::tungstenite::Message;

            let payload = rinf::serialize(&GetMediaFilesCountRequest { call_id: None })?;
            let request_id = Uuid::new_v4();
            socket
                .send(Message::Binary(
                    encode_request(
                        "GetMediaFilesCountRequest",
                        &payload,
                        request_id,
                        library_id,
                    )
                    .into(),
                ))
                .await?;
            let frame = tokio_time::timeout(TEST_DEADLINE, socket.next())
                .await?
                .context("socket closed")??;
            let (name, payload, response_id) =
                decode_message(&frame.into_data()).context("invalid response frame")?;
            assert_eq!(
                response_id, request_id,
                "unexpected response before count barrier"
            );
            assert_eq!(name, "GetMediaFilesCountResponse");
            let response: GetMediaFilesCountResponse = rinf::deserialize(&payload)?;
            assert_eq!(response.count, 0);
            Ok(())
        }
        count(&mut socket, library_id).await?;
        let request = FetchRemoteFileRequest {
            call_id: None,
            url: "http://example.invalid/file".into(),
        };
        socket
            .send(Message::Binary(
                encode_message(
                    "FetchRemoteFileRequest",
                    &rinf::serialize(&request)?,
                    Some(Uuid::new_v4()),
                )
                .into(),
            ))
            .await?;
        // A successful response to the next valid request is the ordering barrier. The count
        // helper checks the very next frame, so an invalid fetch response cannot be discarded.
        count(&mut socket, library_id).await?;
        assert!(
            tokio_time::timeout(Duration::from_millis(500), socket.next())
                .await
                .is_err()
        );

        let name = format!("{}.txt", Uuid::new_v4());
        let bytes = b"remote file contents";
        fs::write(first.path().join(&name), bytes)?;
        let url = Url::parse(&format!(
            "https://127.0.0.1:{}/files/library/{name}",
            addr.port()
        ))?;
        let response = authorized_get(
            config,
            validator.clone(),
            &b.global_params.node.sync_tokens,
            "127.0.0.1",
            addr.port(),
            url,
        )
        .await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.bytes().await?.as_ref(), bytes);
        for (url, kind) in [
            (
                format!("http://127.0.0.1:{}/files/library/{name}", addr.port()),
                RequestErrorKind::InvalidInput,
            ),
            (
                format!("https://untrusted.invalid/files/library/{name}"),
                RequestErrorKind::InvalidInput,
            ),
        ] {
            let request = FetchRemoteFileRequest { call_id: None, url };
            let error = match request
                .handle(request.extract_params(&b.global_params), None, &request)
                .await
            {
                Ok(_) => anyhow::bail!("unexpected success for {}", request.url),
                Err(error) => error,
            };
            assert_eq!(classify(&error), kind, "{}: {error:#}", request.url);
        }
        a.global_params
            .permission_manager
            .write()
            .await
            .change_user_status(&info(&b).await?.fingerprint, UserStatus::Blocked)
            .await?;
        assert!(
            connect_remote(
                "127.0.0.1",
                addr.port(),
                config,
                validator,
                &b.global_params.node.sync_tokens
            )
            .await
            .is_err()
        );
        let frame = tokio_time::timeout(TEST_DEADLINE, socket.next())
            .await?
            .context("revocation close frame missing")??;
        assert!(
            matches!(&frame, Message::Close(Some(frame)) if u16::from(frame.code) == 4403),
            "expected the server's revocation close frame, got {frame:?}"
        );
        a.stop().await?;
        a.global_params.node.token.cancel();
        b.global_params.node.token.cancel();
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn file_endpoint_serves_the_requested_file_for_encoded_names() -> Result<()> {
        use crate::handlers::neighbors::authorized_get;
        use url::Url;

        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let a = fixture(first.path()).await?;
        let b = fixture(second.path()).await?;
        let addr = paired_full(&a, &b).await?;
        let validator = b.global_params.cert_validator.read().await.clone();
        let config = &b.global_params.config_path;

        let spaced = b"file named with a space";
        let escaped = b"file named with a literal percent escape";
        fs::write(first.path().join("a b.txt"), spaced)?;
        fs::write(first.path().join("a%20b.txt"), escaped)?;

        for (encoded, expected) in [("a%20b.txt", spaced.as_slice()), ("a%2520b.txt", escaped)] {
            let url = Url::parse(&format!(
                "https://127.0.0.1:{}/files/library/{encoded}",
                addr.port()
            ))?;
            let response = authorized_get(
                config,
                validator.clone(),
                &b.global_params.node.sync_tokens,
                "127.0.0.1",
                addr.port(),
                url,
            )
            .await?;
            assert_eq!(response.status(), StatusCode::OK, "{encoded}");
            assert_eq!(response.bytes().await?.as_ref(), expected, "{encoded}");
        }

        let url = Url::parse(&format!(
            "https://127.0.0.1:{}/files/library/..%2Fa%20b.txt",
            addr.port()
        ))?;
        let response = authorized_get(
            config,
            validator.clone(),
            &b.global_params.node.sync_tokens,
            "127.0.0.1",
            addr.port(),
            url,
        )
        .await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        fs::create_dir(first.path().join("albums"))?;
        let url = Url::parse(&format!(
            "https://127.0.0.1:{}/files/library/albums",
            addr.port()
        ))?;
        let response = authorized_get(
            config,
            validator,
            &b.global_params.node.sync_tokens,
            "127.0.0.1",
            addr.port(),
            url,
        )
        .await?;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        a.stop().await?;
        a.global_params.node.token.cancel();
        b.global_params.node.token.cancel();
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn library_namespace_requests_authenticate_before_rejecting_other_ids() -> Result<()> {
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let a = fixture(first.path()).await?;
        let b = fixture(second.path()).await?;
        let addr = paired_full(&a, &b).await?;
        a.stop_worker().await?;
        let (base, client) = authenticated_client(
            &b.global_params.node,
            &info(&b).await?.fingerprint,
            &b.private_key,
            &info(&a).await?.fingerprint,
            "127.0.0.1",
            addr.port(),
        )
        .await?;
        let operation = serde_json::json!({"operation": "identity"});
        let wrong = Uuid::new_v4();
        let url = format!("{base}/libraries/{wrong}/sync/metadata");
        let response = client.post(&url).json(&operation).send().await?;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response.json::<serde_json::Value>().await?,
            serde_json::json!({"error": "library_not_served"})
        );
        let unauthenticated = Client::builder()
            .use_preconfigured_tls(
                Arc::new(b.global_params.cert_validator.read().await.clone()).into_client_config(),
            )
            .build()?;
        assert_eq!(
            unauthenticated.post(&url).send().await?.status(),
            StatusCode::UNAUTHORIZED
        );
        let served = a.global_params.library_id.unwrap();
        let response = client
            .post(format!("{base}/libraries/{served}/sync/metadata"))
            .json(&operation)
            .send()
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(body["Ok"]["namespace"], served.to_string());
        a.stop().await?;
        a.global_params.node.token.cancel();
        b.global_params.node.token.cancel();
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn library_namespace_different_libraries_skip_paired_worker_passes() -> Result<()> {
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let a = fixture(first.path()).await?;
        let b = fixture(second.path()).await?;
        let a_track = Uuid::new_v4();
        let b_track = Uuid::new_v4();
        for (manager, id) in [(&a, a_track), (&b, b_track)] {
            manager
                .global_params
                .test_library()
                .library_sync
                .write_local(
                    ObjectKey {
                        collection: Collection::Tracks,
                        id,
                    },
                    Some(Payload::Track(Track {
                        extension: "ogg".into(),
                        content_hash: None,
                        content_size: None,
                        duration: "1".to_owned().try_into().unwrap(),
                        sample_rate: 44100,
                        cover_art_id: None,
                        metadata: Vec::new(),
                    })),
                )
                .await?;
        }
        let a_addr = paired_full(&a, &b).await?;
        a.stop_worker().await?;
        let b_addr = paired_full(&b, &a).await?;
        b.stop_worker().await?;
        for _ in 0..3 {
            // Repeated forced passes still respect the challenge endpoint's rate limit.
            tokio_time::timeout(TEST_DEADLINE, tokio_time::sleep(Duration::from_secs(1))).await?;
            for (local, remote, addr) in [(&a, &b, b_addr), (&b, &a, a_addr)] {
                let endpoint = PeerEndpoint {
                    fingerprint: info(remote).await?.fingerprint,
                    trusted_host: "127.0.0.1".into(),
                    host: "127.0.0.1".into(),
                    port: addr.port(),
                };
                for metadata in [true, false] {
                    let outcome = test_peer_pass(
                        local.global_params.clone(),
                        &local.private_key,
                        endpoint.clone(),
                        metadata,
                    )
                    .await?;
                    assert_eq!(outcome, PeerPass::LibraryNotServed);
                    assert_eq!(outcome.retry_delay(), Duration::from_secs(30));
                }
            }
        }
        for (manager, other) in [(&a, b_track), (&b, a_track)] {
            assert_eq!(
                media_files::Entity::find()
                    .count(&*manager.global_params.test_library().main_db)
                    .await?,
                1
            );
            assert!(
                manager
                    .global_params
                    .test_library()
                    .library_sync
                    .get(&ObjectKey {
                        collection: Collection::Tracks,
                        id: other
                    })
                    .await?
                    .is_none()
            );
        }
        a.stop().await?;
        b.stop().await?;
        a.global_params.node.token.cancel();
        b.global_params.node.token.cancel();
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn sync_tokens_are_reused_across_authenticated_clients_and_peer_passes() -> Result<()> {
        use super::super::libraries::list_libraries;
        use super::super::sync::pool_router;
        use axum::extract::{Request as AxumRequest, State as AxumState};
        use axum::middleware::Next;
        use axum::response::Response;

        type AuthRequests = Arc<StdMutex<Vec<(String, StatusCode)>>>;

        #[derive(Clone, Default)]
        struct AuthRecorder {
            requests: AuthRequests,
        }

        async fn record_auth(
            AxumState(recorder): AxumState<AuthRecorder>,
            request: AxumRequest,
            next: Next,
        ) -> Response {
            let path = request.uri().path().to_owned();
            let response = next.run(request).await;
            if path == "/sync/challenge" || path == "/sync/authorize" {
                recorder
                    .requests
                    .lock()
                    .unwrap()
                    .push((path, response.status()));
            }
            response
        }

        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let host = fixture(first.path()).await?;
        let client = fixture(second.path()).await?;
        let _host_address = paired_full(&host, &client).await?;
        host.stop_worker().await?;
        client.stop_worker().await?;
        let own = info(&client).await?.fingerprint;
        let peer = info(&host).await?.fingerprint;
        let node = host.node()?;
        let node_id = Uuid::parse_str(&node.node_id)?;
        let auth = {
            let sync_auth = host.manager.sync_auth.lock().await;
            sync_auth
                .clone()
                .context("host sync auth not initialized")?
        };
        let recorder = AuthRecorder::default();
        let app = Router::new()
            .route("/libraries", get(list_libraries))
            .nest("/libraries/{library_id}", pool_router(node, node_id))
            .layer(middleware::from_fn_with_state(
                auth.clone(),
                super::super::sync_auth::require_auth,
            ))
            .merge(auth.router())
            .layer(Extension(host.manager.clone()))
            .layer(middleware::from_fn_with_state(
                recorder.clone(),
                record_auth,
            ));
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let peer_address = listener.local_addr()?;
        let tls = RustlsConfig::from_pem(
            host.certificate.as_bytes().to_vec(),
            host.private_key.as_bytes().to_vec(),
        )
        .await?;
        let handle = Handle::new();
        let peer_server = tokio::spawn(async move {
            axum_server::from_tcp_rustls(listener, tls)
                .handle(handle)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                .await
                .unwrap();
        });
        client
            .global_params
            .cert_validator
            .read()
            .await
            .replace_hosts_for_fingerprint(
                &peer,
                vec![format!("127.0.0.1:{}", peer_address.port())],
            )
            .await?;

        let base = sync_worker::peer_base("127.0.0.1", peer_address.port())?;
        let token_client = pinned_anonymous_client(&client.global_params, None).await?;
        let first_token = sync_worker::obtain_or_reuse_token(
            &client.global_params.node.sync_tokens,
            &token_client,
            &base,
            &own,
            &client.private_key,
            &peer,
        )
        .await?;
        let handshake = || recorder.requests.lock().unwrap().clone();
        let initial = handshake();
        assert_eq!(
            initial
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>(),
            ["/sync/challenge", "/sync/authorize"],
            "unexpected initial handshake: {initial:?}"
        );
        assert!(
            initial.iter().all(|(_, status)| *status == StatusCode::OK),
            "initial handshake failed: {initial:?}"
        );

        for _ in 0..2 {
            authenticated_client(
                &client.global_params.node,
                &own,
                &client.private_key,
                &peer,
                "127.0.0.1",
                peer_address.port(),
            )
            .await?;
        }
        assert_eq!(
            sync_worker::obtain_or_reuse_token(
                &client.global_params.node.sync_tokens,
                &token_client,
                &base,
                &own,
                &client.private_key,
                &peer,
            )
            .await?,
            first_token,
            "authenticated clients did not reuse the cached token"
        );
        let after_clients = handshake();
        assert_eq!(
            after_clients, initial,
            "authenticated clients performed another handshake: {after_clients:?}"
        );

        let endpoint = PeerEndpoint {
            fingerprint: peer.clone(),
            trusted_host: format!("127.0.0.1:{}", peer_address.port()),
            host: "127.0.0.1".into(),
            port: peer_address.port(),
        };
        for _ in 0..30 {
            let result = test_peer_pass(
                client.global_params.clone(),
                &client.private_key,
                endpoint.clone(),
                true,
            )
            .await?;
            assert_eq!(result, PeerPass::LibraryNotServed);
        }
        assert_eq!(
            sync_worker::obtain_or_reuse_token(
                &client.global_params.node.sync_tokens,
                &token_client,
                &base,
                &own,
                &client.private_key,
                &peer,
            )
            .await?,
            first_token,
            "peer passes did not reuse the cached token"
        );
        let after_passes = handshake();
        assert_eq!(
            after_passes, initial,
            "peer passes performed another handshake: {after_passes:?}"
        );

        peer_server.abort();
        let _ = peer_server.await;
        host.global_params.node.shutdown().await?;
        client.global_params.node.shutdown().await?;
        Ok(())
    }

    async fn request_peer_libraries(
        manager: &Fixture,
        fingerprint: &str,
    ) -> Result<ListPeerLibrariesResponse> {
        // Forced passes and listings share the challenge endpoint's rate limit.
        tokio_time::timeout(TEST_DEADLINE, tokio_time::sleep(Duration::from_secs(1))).await?;
        let request = ListPeerLibrariesRequest {
            call_id: None,
            fingerprint: fingerprint.to_owned(),
        };
        request
            .handle(
                request.extract_params(&manager.global_params),
                None,
                &request,
            )
            .await?
            .context("list response")
    }

    /// The typed cause of a peer library listing that must fail.
    async fn refused_peer_libraries(
        manager: &Fixture,
        fingerprint: &str,
    ) -> Result<Option<RequestFailureCause>> {
        use crate::request_error::request_failure_cause;

        match request_peer_libraries(manager, fingerprint).await {
            Ok(listed) => anyhow::bail!("peer libraries were listed: {:?}", listed.libraries),
            Err(error) => Ok(request_failure_cause(&error)),
        }
    }

    async fn request_add_library(
        manager: &Fixture,
        fingerprint: &str,
        library: &PeerLibrary,
    ) -> Result<String> {
        let request = AddRemoteLibraryRequest {
            call_id: None,
            fingerprint: fingerprint.to_owned(),
            library_id: library.library_id.clone(),
            alias: library.alias.clone(),
            connection: library.connection,
            sync: library.sync,
        };
        let response = request
            .handle(
                request.extract_params(&manager.global_params),
                None,
                &request,
            )
            .await?
            .context("add response")?;
        Ok(response.record_id)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn peer_library_replica_lifecycle() -> Result<()> {
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let library_id = Uuid::new_v4();
        let a = fixture_with_library(first.path(), library_id).await?;
        let b = fixture(second.path()).await?;
        let fingerprint = info(&a).await?.fingerprint;
        // A also holds somebody else's library, which must never be advertised.
        let mut registry = LibraryRegistry::load(first.path())?;
        create_replica(
            first.path(),
            &mut registry,
            Uuid::new_v4(),
            "Third party",
            "third",
        )
        .await?;
        let remote_control_id = LibraryRecordId(Uuid::new_v4());
        let subsonic_id = LibraryRecordId(Uuid::new_v4());
        LibraryRegistry::update(first.path(), |registry| {
            registry.libraries.extend([
                LibraryRecord {
                    record_id: remote_control_id,
                    alias: "Remote control".into(),
                    library_id: None,
                    protocol: ProtocolAdapter::RuneForward {
                        owner_fingerprint: "remote".into(),
                        mode: RuneForwardMode::RemoteControl,
                    },
                    fs: Vec::new(),
                    catalog: CatalogPlacement::None,
                    background_sync: false,
                },
                LibraryRecord {
                    record_id: subsonic_id,
                    alias: "Subsonic".into(),
                    library_id: None,
                    protocol: ProtocolAdapter::SubsonicTranslate {
                        endpoint: "https://music.example".into(),
                        credential: CredentialRef(Uuid::new_v4()),
                        mode: SubsonicOpenMode::RemoteControl,
                    },
                    fs: Vec::new(),
                    catalog: CatalogPlacement::None,
                    background_sync: false,
                },
            ]);
            Ok(())
        })
        .await?;
        let held_ids = a
            .global_params
            .node
            .library_pool
            .served_library_ids()
            .await?;
        let registry = LibraryRegistry::load(first.path())?;
        anyhow::ensure!(registry.record(remote_control_id).is_ok());
        anyhow::ensure!(registry.record(subsonic_id).is_ok());
        let catalog_ids = registry
            .catalogs()
            .filter_map(LibraryRecord::catalog_library_id)
            .collect::<Vec<_>>();
        anyhow::ensure!(catalog_ids.len() == 2);
        anyhow::ensure!(held_ids.iter().all(|id| catalog_ids.contains(id)));
        let source = first.path().join("source.ogg");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg"),
            &source,
        )?;
        scan_audio_file(
            &a.global_params.test_library().fsio,
            &a.global_params.test_library().main_db,
            &a.global_params.node_id,
            first.path(),
            &source,
            &a.global_params.test_library().library_sync.record_store(),
        )
        .await?;
        let original = media_files::Entity::find()
            .one(&*a.global_params.test_library().main_db)
            .await?
            .context("owner track")?;
        let addr = paired_full(&a, &b).await?;
        a.stop_worker().await?;
        b.global_params
            .cert_validator
            .read()
            .await
            .replace_hosts_for_fingerprint(&fingerprint, vec![format!("127.0.0.1:{}", addr.port())])
            .await?;
        assert_eq!(
            refused_peer_libraries(&b, "unknown").await?,
            Some(RequestFailureCause::RemoteOpenFailed {
                kind: LibraryOpenErrorKind::RemoteUntrusted
            })
        );
        let unauthed = Client::builder()
            .use_preconfigured_tls(sync_worker::tls_config(
                b.global_params.cert_validator.read().await.clone(),
                &fingerprint,
            ))
            .build()?;
        assert_eq!(
            unauthed
                .get(format!("https://127.0.0.1:{}/libraries", addr.port()))
                .send()
                .await?
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let listed = request_peer_libraries(&b, &fingerprint).await?;
        let libraries = listed.libraries;
        assert_eq!(libraries.len(), 1);
        assert_eq!(libraries[0].library_id, library_id.to_string());
        assert_eq!(libraries[0].alias, "Owned");
        assert!(!libraries[0].added);
        assert_ne!(b.global_params.library_id, Some(library_id));
        let record_id = request_add_library(&b, &fingerprint, &libraries[0]).await?;
        assert_eq!(
            record_id,
            request_add_library(&b, &fingerprint, &libraries[0]).await?
        );
        let replica = fixture_open_replica(&b, library_id).await?;
        assert_eq!(replica.global_params.library_id, Some(library_id));
        let endpoint = PeerEndpoint {
            fingerprint: fingerprint.clone(),
            trusted_host: format!("127.0.0.1:{}", addr.port()),
            host: "127.0.0.1".into(),
            port: addr.port(),
        };
        let session = TestPeerSession::default();
        for _ in 0..3 {
            tokio_time::timeout(TEST_DEADLINE, tokio_time::sleep(Duration::from_secs(1))).await?;
            if session
                .pass(
                    replica.global_params.clone(),
                    &replica.private_key,
                    endpoint.clone(),
                    true,
                )
                .await?
                == PeerPass::Complete
            {
                break;
            }
        }
        let remote = media_files::Entity::find()
            .filter(Column::SourceUuid.eq(&original.source_uuid))
            .one(&*replica.global_params.test_library().main_db)
            .await?
            .context("replicated track")?;
        assert!(!remote.local_available);
        scan_audio_library(
            &replica.global_params.test_library().fsio,
            &replica.global_params.test_library().main_db,
            &replica.global_params.node_id,
            Path::new(&replica.global_params.test_library().media_root),
            ScanOptions {
                cleanup: true,
                force: false,
                missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
                cancel_token: None,
            },
            |_| {},
            &replica
                .global_params
                .test_library()
                .library_sync
                .record_store(),
        )
        .await?;
        assert_eq!(
            media_files::Entity::find_by_id(remote.id)
                .one(&*replica.global_params.test_library().main_db)
                .await?,
            Some(remote)
        );
        let listed = request_peer_libraries(&replica, &fingerprint).await?;
        let libraries = listed.libraries;
        assert_eq!(libraries.len(), 1);
        assert!(libraries[0].added);
        let replica_record_id = request_add_library(&replica, &fingerprint, &libraries[0]).await?;
        assert_eq!(
            replica_record_id,
            request_add_library(&replica, &fingerprint, &libraries[0]).await?
        );
        let registry = LibraryRegistry::load(second.path())?;
        assert_eq!(registry.libraries.len(), 2);
        assert_eq!(
            registry.catalog(library_id).unwrap().owner_fingerprint(),
            Some(fingerprint.as_str())
        );
        let registry_path = first.path().join("libraries.json");
        let saved = fs::read(&registry_path)?;
        fs::write(&registry_path, b"invalid registry")?;
        let malformed = refused_peer_libraries(&replica, &fingerprint).await?;
        fs::write(&registry_path, saved)?;
        assert_eq!(
            malformed,
            Some(RequestFailureCause::RemoteOpenFailed {
                kind: LibraryOpenErrorKind::RemoteProtocol
            })
        );
        a.global_params
            .permission_manager
            .write()
            .await
            .change_user_status(&info(&b).await?.fingerprint, UserStatus::Pending)
            .await?;
        assert_eq!(
            refused_peer_libraries(&replica, &fingerprint).await?,
            Some(RequestFailureCause::RemoteOpenFailed {
                kind: LibraryOpenErrorKind::RemoteNotAuthorized
            })
        );
        a.stop().await?;
        assert_eq!(
            refused_peer_libraries(&replica, &fingerprint).await?,
            Some(RequestFailureCause::RemoteOpenFailed {
                kind: LibraryOpenErrorKind::RemoteUnreachable
            })
        );
        for manager in [&a, &b, &replica] {
            manager.stop_worker().await?;
            manager.global_params.node.token.cancel();
        }
        Ok(())
    }

    /// A replica needs pull access. A guest that may browse a library but not
    /// sync it must not be left with a record whose database can never open.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn guest_replica_registration_requires_pull_access() -> Result<()> {
        use crate::access_policy::{ConnectionLevel, SyncLevel};

        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let library_id = Uuid::new_v4();
        let a = fixture_with_library(first.path(), library_id).await?;
        let b = fixture(second.path()).await?;
        let owner_fingerprint = info(&a).await?.fingerprint;
        let guest_fingerprint = info(&b).await?.fingerprint;
        let addr = paired_full(&a, &b).await?;
        a.stop_worker().await?;

        // The guest may browse the library through the connection, but has no
        // sync grant, so it cannot pull the owner's metadata.
        let mut grants = a
            .global_params
            .node
            .access_policy
            .grants(&guest_fingerprint)?;
        grants
            .connection
            .libraries
            .insert(library_id, ConnectionLevel::View);
        grants.sync.libraries.insert(library_id, SyncLevel::None);
        a.global_params
            .node
            .access_policy
            .set(&guest_fingerprint, grants)?;

        let listed = request_peer_libraries(&b, &owner_fingerprint).await?;
        let guest_view = listed
            .libraries
            .into_iter()
            .next()
            .context("peer library")?;
        assert!(matches!(guest_view.connection, DeviceConnectionLevel::View));
        assert!(matches!(guest_view.sync, DeviceSyncLevel::None));

        let (_, client) = authenticated_client(
            &b.global_params.node,
            &guest_fingerprint,
            &b.private_key,
            &owner_fingerprint,
            "127.0.0.1",
            addr.port(),
        )
        .await?;
        let pull = client
            .post(format!(
                "https://127.0.0.1:{}/libraries/{library_id}/sync/metadata",
                addr.port()
            ))
            .json(&serde_json::json!({"operation": "identity"}))
            .send()
            .await?;
        assert_eq!(pull.status(), StatusCode::FORBIDDEN);

        // Registration must be refused before it leaves any replica state.
        let replica_root = second.path().join("replicas").join(library_id.to_string());
        let replica_descriptor = replica_root.join(".rune").join("library.json");
        let replica_db = replica_root.join(".rune").join(".0.db");
        let registration = request_add_library(&b, &owner_fingerprint, &guest_view).await;
        let registry = LibraryRegistry::load(second.path())?;
        match registration {
            Ok(record_id) => anyhow::bail!(
                "Sync None registration succeeded with record {record_id}: \
                 registry record {}, replica descriptor {}, replica database {}",
                registry.catalog(library_id).is_ok(),
                replica_descriptor.is_file(),
                replica_db.is_file()
            ),
            Err(error) => {
                assert_eq!(
                    classify(&error),
                    RequestErrorKind::PermissionDenied,
                    "{error:#}"
                );
                assert_eq!(
                    request_failure_cause(&error),
                    Some(RequestFailureCause::LibraryRegistrationFailed {
                        kind: RegisterLibraryFailureKind::SyncNotShared,
                    })
                );
            }
        }
        assert!(
            registry.catalog(library_id).is_err(),
            "a refused registration must not leave a replica record"
        );
        assert!(
            !replica_root.exists(),
            "a refused registration must not leave a replica folder"
        );

        // Pull access makes the same library a real, initialized replica.
        let mut grants = a
            .global_params
            .node
            .access_policy
            .grants(&guest_fingerprint)?;
        grants.sync.libraries.insert(library_id, SyncLevel::Pull);
        a.global_params
            .node
            .access_policy
            .set(&guest_fingerprint, grants)?;
        let listed = request_peer_libraries(&b, &owner_fingerprint).await?;
        let library = listed
            .libraries
            .into_iter()
            .next()
            .context("peer library")?;
        assert!(matches!(library.sync, DeviceSyncLevel::Pull));
        request_add_library(&b, &owner_fingerprint, &library).await?;

        let replica = fixture_open_replica(&b, library_id).await?;
        assert!(
            replica_db.is_file(),
            "opening an authorized replica must create its database"
        );
        let original = scan_held_track(&a, library_id, "replica.ogg").await?;
        let endpoint = PeerEndpoint {
            fingerprint: owner_fingerprint.clone(),
            trusted_host: format!("127.0.0.1:{}", addr.port()),
            host: "127.0.0.1".into(),
            port: addr.port(),
        };
        let session = TestPeerSession::default();
        wait_until("the authorized replica to pull its first track", || {
            let params = replica.global_params.clone();
            let private_key = replica.private_key.clone();
            let endpoint = endpoint.clone();
            let session = &session;
            let original = &original;
            async move {
                if session
                    .pass(params.clone(), &private_key, endpoint, true)
                    .await?
                    == PeerPass::LibraryNotServed
                {
                    anyhow::bail!("the owner stopped serving the replica's library");
                }
                Ok(media_files::Entity::find()
                    .filter(Column::SourceUuid.eq(&original.source_uuid))
                    .one(&*params.test_library().main_db)
                    .await?
                    .map(|_| ()))
            }
        })
        .await?;

        a.stop().await?;
        for manager in [&a, &b] {
            manager.stop_worker().await?;
            manager.global_params.node.token.cancel();
        }
        Ok(())
    }

    async fn held_owned_library(manager: &Fixture, root: &Path) -> Result<Uuid> {
        let mut registry = LibraryRegistry::load(Path::new(&*manager.global_params.config_path))?;
        let descriptor = resolve_library(
            &manager.global_params.test_library().fsio,
            &mut registry,
            root.to_str().context("held library path")?,
            "Held",
            StorageMode::Portable,
        )
        .await?;
        descriptor
            .catalog_library_id()
            .context("held library catalog")
    }

    async fn scan_held_track(
        manager: &Fixture,
        id: Uuid,
        name: &str,
    ) -> Result<media_files::Model> {
        let library = manager.global_params.library_pool.acquire(id).await?;
        let root = Path::new(&library.media_root);
        let source = root.join(name);
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg"),
            &source,
        )?;
        scan_audio_file(
            &library.fsio,
            &library.main_db,
            &manager.global_params.node_id,
            root,
            &source,
            &library.library_sync.record_store(),
        )
        .await?;
        media_files::Entity::find()
            .filter(Column::FileName.eq(name))
            .one(&*library.main_db)
            .await?
            .context("held track")
    }

    async fn set_library_background(manager: &Fixture, id: Uuid, enabled: bool) -> Result<()> {
        let request = SetLibraryBackgroundSyncRequest {
            call_id: None,
            library_id: id.to_string(),
            enabled,
        };
        request
            .handle(
                request.extract_params(&manager.global_params),
                None,
                &request,
            )
            .await?
            .context("option response")?;
        Ok(())
    }

    async fn write_album(library: &LibraryHandle, name: &str) -> Result<ObjectKey> {
        let key = ObjectKey {
            collection: Collection::Albums,
            id: parent(ParentKind::Album, name),
        };
        library
            .library_sync
            .write_local(key.clone(), Some(album_payload(name)))
            .await?;
        Ok(key)
    }

    async fn wait_for_album(library: &LibraryHandle, key: &ObjectKey) -> Result<()> {
        wait_until("album to replicate without a local UI library", || async {
            Ok(library.library_sync.get(key).await?.map(|_| ()))
        })
        .await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn node_without_ui_serves_and_syncs_held_libraries_bidirectionally() -> Result<()> {
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let a = fixture(first.path()).await?;
        let b = fixture(second.path()).await?;
        let id = held_owned_library(&a, &first.path().join("Held")).await?;
        peer_step(
            "add held library peer",
            request_add_library(
                &b,
                &info(&a).await?.fingerprint,
                &PeerLibrary {
                    library_id: id.to_string(),
                    alias: "Held".into(),
                    added: false,
                    connection: DeviceConnectionLevel::ViewEdit,
                    sync: DeviceSyncLevel::PullPush,
                },
            ),
        )
        .await?;
        let track = scan_held_track(&a, id, "source.ogg").await?;
        let left = a.global_params.library_pool.acquire(id).await?;
        let right = b.global_params.library_pool.acquire(id).await?;
        let forward = write_album(&left, "forward").await?;
        let reverse = write_album(&right, "reverse").await?;
        close_ui(&a.global_params).await?;
        close_ui(&b.global_params).await?;
        assert!(a.global_params.node.current_ui().await.is_err());
        assert!(b.global_params.node.current_ui().await.is_err());
        let a_addr = peer_step("pair and start first listener", paired_full(&a, &b)).await?;
        let b_addr = peer_step("pair and start second listener", paired_full(&b, &a)).await?;
        for (local, remote, addr) in [(&a, &b, b_addr), (&b, &a, a_addr)] {
            local
                .global_params
                .cert_validator
                .read()
                .await
                .replace_hosts_for_fingerprint(
                    &info(remote).await?.fingerprint,
                    vec![format!("127.0.0.1:{}", addr.port())],
                )
                .await?;
        }
        let (_, client) = peer_step(
            "authenticate held library client",
            authenticated_client(
                &b.global_params.node,
                &info(&b).await?.fingerprint,
                &b.private_key,
                &info(&a).await?.fingerprint,
                "127.0.0.1",
                a_addr.port(),
            ),
        )
        .await?;
        let response = peer_step("fetch held library file", async {
            Ok(client
                .get(format!(
                    "https://{a_addr}/files/library/source.ogg?library_id={id}"
                ))
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await?)
        })
        .await?;
        assert_eq!(
            blake3::hash(&response).to_hex().as_str(),
            track.content_hash.as_ref().context("digest")?
        );
        peer_step("request held library metadata sync", async {
            client
                .post(format!("https://{a_addr}/libraries/{id}/sync/metadata"))
                .json(&serde_json::json!({"operation": "identity"}))
                .send()
                .await?
                .error_for_status()?;
            Ok(())
        })
        .await?;
        tokio::try_join!(
            wait_for_album(&left, &reverse),
            wait_for_album(&right, &forward)
        )?;
        let worker = b.sync_handle.lock().await.as_ref().context("worker")?.id();
        let remote_fingerprint = info(&a).await?.fingerprint;
        let remote_ui = peer_step(
            "open headless controller library",
            open_remote_control_record(b.global_params.node.clone(), &remote_fingerprint),
        )
        .await?;
        assert!(remote_ui.library_id.is_none());
        assert!(Arc::ptr_eq(
            &remote_ui.permission_manager,
            &b.global_params.permission_manager
        ));
        assert_eq!(b.get_address().await, Some(b_addr));
        assert_eq!(
            b.sync_handle.lock().await.as_ref().context("worker")?.id(),
            worker
        );
        let later = write_album(&left, "while controlling another node").await?;
        wait_for_album(&right, &later).await?;
        close_ui(&remote_ui).await?;
        b.global_params.node.shutdown().await?;
        a.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn scenario_headless_controller_receives_only_allowed_status() -> Result<()> {
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let original = fixture(first.path()).await?;
        original.global_params.node.shutdown().await?;
        let node = NodeContext::new(
            first.path().to_str().context("config path")?,
            first.path().to_str().context("db path")?,
            Arc::new(WebSocketService::new()),
            false,
            RunMode::App,
            Arc::new(Visualizer::new()),
        )
        .await?;
        let library = node
            .library_pool
            .acquire_ui(original.global_params.library_id.context("library")?)
            .await?;
        let a = fixture_from_library(node, library).await?;
        let b = fixture(second.path()).await?;
        let addr = paired_full(&a, &b).await?;
        let mut socket = connect_remote(
            "127.0.0.1",
            addr.port(),
            &b.global_params.config_path,
            b.global_params.cert_validator.read().await.clone(),
            &b.global_params.node.sync_tokens,
        )
        .await?;
        assert!(
            a.global_params
                .node
                .websocket_service
                .broadcast_tx
                .receiver_count()
                > 0,
            "controller connection returned before broadcast subscription"
        );
        let broadcaster = &a.global_params.broadcaster;
        broadcaster.broadcast(&TrustListUpdated {
            certificates: vec![],
        });
        broadcaster.broadcast(&PlaybackStatus {
            session_id: a.global_params.node.session()?.session_id().await,
            state: PlaybackState::Playing,
            progress_seconds: 12.0,
            progress_percentage: 0.5,
            artist: None,
            album: None,
            title: None,
            duration: 24.0,
            index: None,
            item: None,
            item_request: None,
            playback_mode: PlaybackMode::Sequential,
            ready: true,
            cover_art_path: None,
            lib_path: a.global_params.test_library().media_root.clone(),
        });
        let frame =
            peer_step_with_timeout("controller playback status frame", TEST_DEADLINE, async {
                Ok(socket.next().await.context("controller socket closed")??)
            })
            .await?;
        let (name, payload, _) = decode_message(&frame.into_data()).context("status frame")?;
        assert_eq!(name, "PlaybackStatus");
        let status: PlaybackStatus = rinf::deserialize(&payload)?;
        assert_eq!(status.progress_seconds, 12.0);
        let unexpected_frame = tokio_time::timeout(Duration::from_millis(200), socket.next()).await;
        assert!(
            unexpected_frame.is_err(),
            "controller received a disallowed frame"
        );
        a.global_params.node.shutdown().await?;
        b.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn scenario_idle_held_library_reopens_for_sync_and_preserves_cursor() -> Result<()> {
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let a =
            fixture_with_library_timeout(first.path(), Uuid::new_v4(), Duration::from_millis(100))
                .await?;
        let b = fixture(second.path()).await?;
        let id = held_owned_library(&a, &first.path().join("Held")).await?;
        let fingerprint = info(&a).await?.fingerprint;
        request_add_library(
            &b,
            &fingerprint,
            &PeerLibrary {
                library_id: id.to_string(),
                alias: "Held".into(),
                added: false,
                connection: DeviceConnectionLevel::ViewEdit,
                sync: DeviceSyncLevel::PullPush,
            },
        )
        .await?;
        let addr = paired_full(&a, &b).await?;
        a.stop_worker().await?;
        b.stop_worker().await?;
        set_library_background(&a, id, false).await?;
        let source = a.global_params.library_pool.acquire(id).await?;
        let key = write_album(&source, "before idle close").await?;
        let identity = source.library_sync.identity().await?;
        let connection = Arc::downgrade(&source.main_db);
        drop(source);
        wait_until("held library to idle close", || async {
            Ok(connection.upgrade().is_none().then_some(()))
        })
        .await?;
        assert!(
            !a.global_params
                .library_pool
                .served_library_ids()
                .await?
                .contains(&id)
        );
        set_library_background(&a, id, true).await?;
        assert!(connection.upgrade().is_none());
        let (base, client) = authenticated_client(
            &b.global_params.node,
            &info(&b).await?.fingerprint,
            &b.private_key,
            &fingerprint,
            "127.0.0.1",
            addr.port(),
        )
        .await?;
        let replica = b.global_params.library_pool.acquire(id).await?;
        let url = format!("{base}/libraries/{id}");
        let node_id = Uuid::parse_str(&b.global_params.node_id)?;
        let first_pass =
            sync_library((*replica.main_db).clone(), node_id, &url, client.clone()).await?;
        assert!(first_pass.pulled.sequence > 0);
        assert!(replica.library_sync.get(&key).await?.is_some());
        let checkpoint = CheckpointKey {
            namespace: id,
            source_replica: identity.replica,
            source_generation: identity.generation,
            scope_id: scope_id(&Scope::<RuneModel> {
                collections: RuneModel::collections().into_iter().collect(),
            })?,
        };
        assert_eq!(
            replica
                .library_sync
                .endpoint()
                .checkpoint(&checkpoint)
                .await?,
            Some(first_pass.pulled.clone())
        );
        let second_pass = sync_library((*replica.main_db).clone(), node_id, &url, client).await?;
        assert!(second_pass.pulled.sequence >= first_pass.pulled.sequence);
        assert_eq!(
            replica
                .library_sync
                .endpoint()
                .checkpoint(&checkpoint)
                .await?,
            Some(second_pass.pulled)
        );
        a.global_params.node.shutdown().await?;
        b.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn held_libraries_sync_without_ui_and_respect_background_option() -> Result<()> {
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let a = fixture_with_library(first.path(), Uuid::new_v4()).await?;
        let b =
            fixture_with_library_timeout(second.path(), Uuid::new_v4(), Duration::from_millis(100))
                .await?;
        let library_id = held_owned_library(&a, &first.path().join("L2")).await?;
        let fingerprint = info(&a).await?.fingerprint;
        let peer_library = PeerLibrary {
            library_id: library_id.to_string(),
            alias: "L2".into(),
            added: false,
            connection: DeviceConnectionLevel::ViewEdit,
            sync: DeviceSyncLevel::PullPush,
        };
        request_add_library(&b, &fingerprint, &peer_library).await?;
        let original = scan_held_track(&a, library_id, "first.ogg").await?;
        let replica = b.global_params.library_pool.acquire(library_id).await?;
        let ui_id = b.global_params.library_id.context("UI library")?;
        assert_ne!(a.global_params.library_id, Some(library_id));
        assert_ne!(ui_id, library_id);
        assert!(Arc::ptr_eq(
            &b.global_params.test_library().main_db,
            &b.global_params.library_pool.acquire(ui_id).await?.main_db,
        ));
        let addr = paired_full(&a, &b).await?;
        a.stop_worker().await?;
        b.global_params
            .cert_validator
            .read()
            .await
            .replace_hosts_for_fingerprint(&fingerprint, vec![format!("127.0.0.1:{}", addr.port())])
            .await?;
        b.start_worker().await?;
        let received = wait_until("unopened L2 track to synchronize", || async {
            Ok(media_files::Entity::find()
                .filter(Column::SourceUuid.eq(&original.source_uuid))
                .one(&*replica.main_db)
                .await?)
        })
        .await?;
        assert!(!received.local_available);
        assert_eq!(
            media_files::Entity::find()
                .count(&*b.global_params.test_library().main_db)
                .await?,
            0
        );

        set_download_rule(
            &replica.main_db,
            vec![("lib::track".into(), received.id.to_string())],
            None,
            true,
        )
        .await?;
        // A fresh worker pass should evaluate L2's rule, while M has no rules or tracks.
        b.stop_worker().await?;
        b.start_worker().await?;
        wait_until("unopened L2 download rule to apply", || async {
            Ok(is_materialized(
                &*replica.main_db,
                &replica.fsio,
                Path::new(&replica.media_root),
                received.id,
            )
            .await?
            .then_some(()))
        })
        .await?;

        // The restarted worker shares the challenge quota with this direct request.
        let (_, client) = authenticated_client(
            &b.global_params.node,
            &info(&b).await?.fingerprint,
            &b.private_key,
            &fingerprint,
            "127.0.0.1",
            addr.port(),
        )
        .await?;
        let base = format!("https://{addr}/libraries/{library_id}");
        let digest = original.content_hash.as_ref().context("track digest")?;
        let blob_url = format!("{base}/sync/blobs/{digest}");
        assert_eq!(
            client
                .head(&blob_url)
                .query(&[("track_id", &original.source_uuid)])
                .send()
                .await?
                .status(),
            StatusCode::OK
        );
        let bytes = client
            .get(&blob_url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), digest);

        let a_ui = a.global_params.library_id.context("A UI library")?;
        set_library_background(&a, a_ui, false).await?;
        let response = client
            .post(format!("https://{addr}/libraries/{a_ui}/sync/metadata"))
            .json(&serde_json::json!({"operation": "identity"}))
            .send()
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.json::<serde_json::Value>().await?["Ok"]["namespace"],
            a_ui.to_string()
        );

        set_library_background(&a, library_id, false).await?;
        for response in [
            client.post(format!("{base}/sync/metadata")).send().await?,
            client.get(&blob_url).send().await?,
        ] {
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert_eq!(
                response.json::<serde_json::Value>().await?["error"],
                "library_not_served"
            );
        }
        set_library_background(&a, library_id, true).await?;
        set_library_background(&b, library_id, false).await?;
        let connection = Arc::downgrade(&replica.main_db);
        drop(replica);
        // Leave the worker running across a complete normal retry interval.
        let later = scan_held_track(&a, library_id, "second.ogg").await?;
        tokio_time::timeout(TEST_DEADLINE, tokio_time::sleep(Duration::from_secs(32))).await?;
        let replica = b.global_params.library_pool.acquire(library_id).await?;
        assert!(
            media_files::Entity::find()
                .filter(Column::SourceUuid.eq(&later.source_uuid))
                .one(&*replica.main_db)
                .await?
                .is_none(),
            "disabled L2 received a later track"
        );
        wait_until("disabled L2 idle pool connection to close", || async {
            Ok(connection.upgrade().is_none().then_some(()))
        })
        .await?;
        assert!(Arc::ptr_eq(
            &b.global_params.test_library().main_db,
            &b.global_params.library_pool.acquire(ui_id).await?.main_db,
        ));
        a.stop().await?;
        b.stop_worker().await?;
        a.global_params.node.token.cancel();
        b.global_params.node.token.cancel();
        Ok(())
    }

    fn album_payload(name: &str) -> Payload {
        Payload::Album(Album {
            name: name.into(),
            group: "test".into(),
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn paired_background_sync_and_listener_lifecycle() -> Result<()> {
        let _ = tracing_subscriber::fmt()
            .with_max_level(LevelFilter::WARN)
            .with_test_writer()
            .try_init();
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let library_id = Uuid::new_v4();
        let a = peer_step_with_timeout(
            "create first fixture",
            TEST_DEADLINE,
            fixture_with_library(first.path(), library_id),
        )
        .await?;
        let b = peer_step_with_timeout(
            "create second fixture",
            TEST_DEADLINE,
            fixture_with_library(second.path(), library_id),
        )
        .await?;
        let a_info = peer_step("read first peer identity", info(&a)).await?;
        let b_info = peer_step("read second peer identity", info(&b)).await?;
        peer_step("pair library peers", pair(&a, &b)).await?;
        peer_step("pair library peers in reverse", pair(&b, &a)).await?;
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg"),
            second.path().join("source.ogg"),
        )?;
        peer_step_with_timeout("scan and publish source audio", TEST_DEADLINE, async {
            scan_audio_library(
                &b.global_params.test_library().fsio,
                &b.global_params.test_library().main_db,
                &b.global_params.node_id,
                second.path(),
                ScanOptions {
                    cleanup: false,
                    force: false,
                    missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
                    cancel_token: None,
                },
                |_| {},
                &b.global_params.test_library().library_sync.record_store(),
            )
            .await?;
            let source_track = media_files::Entity::find()
                .filter(Column::LocalAvailable.eq(true))
                .one(&*b.global_params.test_library().main_db)
                .await?
                .context("fixture scanner did not create the source audio row")?;
            let track_key = ObjectKey {
                collection: Collection::Tracks,
                id: source_track.source_uuid.parse()?,
            };
            anyhow::ensure!(
                b.global_params
                    .test_library()
                    .library_sync
                    .get(&track_key)
                    .await?
                    .is_some(),
                "fixture scanner did not publish source track metadata"
            );
            Ok(())
        })
        .await?;
        let key = ObjectKey {
            collection: Collection::Albums,
            id: parent(ParentKind::Album, "background album"),
        };
        let reverse_key = ObjectKey {
            collection: Collection::Albums,
            id: parent(ParentKind::Album, "first node album"),
        };
        peer_step("seed bidirectional metadata", async {
            a.global_params
                .test_library()
                .library_sync
                .write_local(reverse_key.clone(), Some(album_payload("first node album")))
                .await?;
            let store = LibrarySyncStore::open(
                (*b.global_params.test_library().main_db).clone(),
                b.global_params.node_id.parse()?,
                None,
            )
            .await?;
            store
                .import_existing(b.global_params.node_id.parse()?)
                .await?;
            store
                .write_local(key.clone(), Some(album_payload("background album")))
                .await?;
            Ok(())
        })
        .await?;
        peer_step(
            "start first sync listener",
            a.clone().start_sync(
                ([127, 0, 0, 1], 0).into(),
                DiscoveryParams {
                    device_info: a_info.clone(),
                },
            ),
        )
        .await?;
        let a_addr = a
            .get_address()
            .await
            .context("first sync listener address")?;
        // A bound socket that has not called listen refuses connections and
        // keeps another fixture from claiming the offline peer's port.
        let offline_socket = TcpSocket::new_v4()?;
        offline_socket.bind(([127, 0, 0, 1], 0).into())?;
        let offline_addr = offline_socket.local_addr()?;
        peer_step(
            "announce peer before second listener starts",
            announce_peer(&a, &b_info, offline_addr),
        )
        .await?;
        assert_eq!(
            TcpStream::connect(offline_addr)
                .await
                .expect_err("offline peer unexpectedly accepted a connection")
                .kind(),
            ErrorKind::ConnectionRefused
        );
        peer_step(
            "start second sync listener",
            b.clone().start_sync(
                ([127, 0, 0, 1], 0).into(),
                DiscoveryParams {
                    device_info: b_info.clone(),
                },
            ),
        )
        .await?;
        let b_addr = b
            .get_address()
            .await
            .context("second sync listener address")?;
        assert_ne!(a_addr, b_addr);
        peer_step(
            "announce the second listener's bound address",
            announce_peer(&a, &b_info, b_addr),
        )
        .await?;
        drop(offline_socket);
        peer_step(
            "announce first listener to second peer",
            announce_peer(&b, &a_info, a_addr),
        )
        .await?;
        wait_until("peers to learn advertised ports", || async {
            let seen_a = b
                .global_params
                .device_scanner
                .get_all_devices()
                .iter()
                .any(|device| {
                    device.fingerprint == a_info.fingerprint
                        && device.api_port == Some(a_addr.port())
                });
            let seen_b = a
                .global_params
                .device_scanner
                .get_all_devices()
                .iter()
                .any(|device| {
                    device.fingerprint == b_info.fingerprint
                        && device.api_port == Some(b_addr.port())
                });
            Ok((seen_a && seen_b).then_some(()))
        })
        .await
        .context("peers did not learn advertised ports")?;
        for (receiver, fingerprint, expected_address) in [
            (
                &a,
                &b_info.fingerprint,
                format!("127.0.0.1:{}", b_addr.port()),
            ),
            (
                &b,
                &a_info.fingerprint,
                format!("127.0.0.1:{}", a_addr.port()),
            ),
        ] {
            receiver
                .global_params
                .cert_validator
                .read()
                .await
                .remove_fingerprint(fingerprint)
                .await?;
            let request = GetDiscoveredDeviceRequest { call_id: None };
            let response = request
                .handle(
                    Arc::clone(&receiver.global_params.device_scanner),
                    None,
                    &request,
                )
                .await?
                .context("discovered device response")?;
            let device = response
                .devices
                .iter()
                .find(|device| device.fingerprint == *fingerprint)
                .context("announced peer missing from discovered devices")?;
            assert!(
                device.addresses.contains(&expected_address),
                "discovered addresses {addresses:?} omitted expected loopback address {expected_address}",
                addresses = device.addresses
            );
            add_trusted_server(
                &*receiver.global_params.cert_validator.read().await,
                Some(&TrustedServerCertificate {
                    fingerprint: fingerprint.clone(),
                    hosts: device.addresses.clone(),
                }),
            )
            .await?;
        }
        let pinned = peer_step(
            "build pinned anonymous client",
            pinned_anonymous_client(&b.global_params, None),
        )
        .await?;
        let base = format!("https://{a_addr}");
        let unauthorized_metadata = peer_step("unauthorized metadata request", async {
            Ok(pinned
                .post(format!("{base}/libraries/{library_id}/sync/metadata"))
                .send()
                .await?
                .status())
        })
        .await?;
        assert_eq!(unauthorized_metadata, StatusCode::UNAUTHORIZED);
        let unauthorized_file = peer_step("unauthorized file request", async {
            Ok(pinned
                .get(format!("{base}/files/anything"))
                .send()
                .await?
                .status())
        })
        .await?;
        assert_eq!(unauthorized_file, StatusCode::UNAUTHORIZED);
        let remote_id = wait_until(
            "background sync to transfer album and track metadata",
            || async {
                let receiver = a.global_params.test_library().library_sync.clone();
                if receiver.get(&key).await?.is_some()
                    && b.global_params
                        .test_library()
                        .library_sync
                        .get(&reverse_key)
                        .await?
                        .is_some()
                    && let Some(track) = media_files::Entity::find()
                        .filter(Column::LocalAvailable.eq(false))
                        .one(&*a.global_params.test_library().main_db)
                        .await?
                {
                    Ok(Some(track.id))
                } else {
                    Ok(None)
                }
            },
        )
        .await?;
        peer_step("configure audio peer permissions", async {
            b.global_params
                .permission_manager
                .write()
                .await
                .add_user(
                    a.certificate.clone(),
                    a_info.fingerprint.clone(),
                    "paired".into(),
                    "test".into(),
                    DeviceType::Desktop,
                    "127.0.0.1".into(),
                )
                .await?;
            b.global_params
                .permission_manager
                .write()
                .await
                .change_user_status(&a_info.fingerprint, UserStatus::Approved)
                .await?;
            a.global_params
                .cert_validator
                .read()
                .await
                .replace_hosts_for_fingerprint(
                    &b_info.fingerprint,
                    vec![
                        format!("127.0.0.1:{}", b_addr.port()),
                        format!("localhost:{}", b_addr.port()),
                    ],
                )
                .await?;
            Ok(())
        })
        .await?;
        let audio_library = peer_step(
            "acquire audio library",
            a.global_params
                .library_pool
                .acquire(a.global_params.library_id.context("library")?),
        )
        .await?;
        let source = wait_until("remote audio source to become available", || async {
            match a
                .resolve_remote_audio(
                    &audio_library,
                    a.global_params.main_token.child_token(),
                    remote_id,
                )
                .await
            {
                Ok(source) => Ok(Some(source)),
                Err(_) => Ok(None),
            }
        })
        .await?;
        let expected = fs::read(second.path().join("source.ogg"))?;
        let source_path = source
            .url
            .strip_prefix(&format!("https://127.0.0.1:{}", b_addr.port()))
            .context("remote audio URL did not use the announced loopback address")?;
        let full_bytes = peer_step("full audio GET", async {
            let full = source.request.get(None).await?;
            anyhow::ensure!(full.status() == StatusCode::OK, "full audio GET status");
            Ok(full.bytes().await?)
        })
        .await?;
        assert_eq!(full_bytes.as_ref(), expected.as_slice());
        let ranged_reads = tokio::join!(
            peer_step("first concurrent ranged audio GET", async {
                let response = source.request.get(Some((0, Some(15)))).await?;
                anyhow::ensure!(
                    response.status() == StatusCode::PARTIAL_CONTENT,
                    "ranged audio GET status"
                );
                Ok(response.bytes().await?)
            }),
            peer_step("second concurrent ranged audio GET", async {
                let response = source.request.get(Some((16, Some(31)))).await?;
                anyhow::ensure!(
                    response.status() == StatusCode::PARTIAL_CONTENT,
                    "ranged audio GET status"
                );
                Ok(response.bytes().await?)
            }),
        );
        let (first_range, second_range) = ranged_reads;
        assert_eq!(first_range?.as_ref(), &expected[..16]);
        assert_eq!(second_range?.as_ref(), &expected[16..32]);
        anyhow::ensure!(
            !source.cancellation.is_cancelled(),
            "concurrent ranged reads cancelled an authorized stream"
        );
        let stream_cancel = source.cancellation.clone();
        let stream = peer_step("create authorized audio stream", async {
            create_stream_with_authorizer(source.request.clone(), &source.url).await
        })
        .await?;
        let stream = ManagedStream::new(stream, stream_cancel.clone());
        let extension = source.extension;
        let expected_tail = expected[expected.len() - 12..].to_vec();
        let _cancel_stream_on_timeout = stream_cancel.clone().drop_guard();
        let decoded = peer_step("stream seek and decode", async move {
            task::spawn_blocking(move || -> Result<bool> {
                use std::io::{Read, Seek, SeekFrom};
                let mut stream = stream;
                let mut tail = [0u8; 12];
                stream.seek(SeekFrom::End(-12))?;
                stream.read_exact(&mut tail)?;
                anyhow::ensure!(
                    tail.as_slice() == expected_tail.as_slice(),
                    "stream seek returned wrong bytes"
                );
                stream.seek(SeekFrom::Start(0))?;
                let mut decoder = RuneDecoder::new(stream, Some(&extension))?;
                Ok(decoder.next().is_some())
            })
            .await?
        })
        .await?;
        assert!(decoded, "remote audio must decode without materializing it");
        peer_step("revoke remote audio permission", async {
            b.global_params
                .permission_manager
                .write()
                .await
                .change_user_status(&a_info.fingerprint, UserStatus::Blocked)
                .await?;
            Ok(())
        })
        .await?;
        let denied = peer_step("revoked ranged audio GET", async {
            Ok(source.request.get(Some((16, Some(31)))).await)
        })
        .await?;
        assert!(denied.is_err());
        tokio_time::timeout(TEST_DEADLINE, stream_cancel.cancelled())
            .await
            .context("revocation did not cancel the remote stream")?;
        peer_step("stop paired workers", async {
            a.stop_worker().await?;
            b.stop_worker().await
        })
        .await?;
        peer_step("restore remote audio permission", async {
            b.global_params
                .permission_manager
                .write()
                .await
                .change_user_status(&a_info.fingerprint, UserStatus::Approved)
                .await?;
            Ok(())
        })
        .await?;
        // Blocking dropped A's token on B, so this renews it once on 401.
        let suffix_bytes = peer_step("raw suffix audio GET", async {
            let suffix = send_authenticated_request(
                &a.global_params.node,
                &a_info.fingerprint,
                &a.private_key,
                &b_info.fingerprint,
                "localhost",
                b_addr.port(),
                |base, client| {
                    client
                        .get(format!("{base}{source_path}"))
                        .header(header::RANGE, "bytes=-12")
                },
            )
            .await?;
            anyhow::ensure!(
                suffix.status() == StatusCode::PARTIAL_CONTENT,
                "suffix audio GET status"
            );
            Ok(suffix.bytes().await?)
        })
        .await?;
        assert_eq!(suffix_bytes.as_ref(), &expected[expected.len() - 12..]);
        let invalid = peer_step("invalid range audio GET", async {
            send_authenticated_request(
                &a.global_params.node,
                &a_info.fingerprint,
                &a.private_key,
                &b_info.fingerprint,
                "localhost",
                b_addr.port(),
                |base, client| {
                    client
                        .get(format!("{base}{source_path}"))
                        .header(header::RANGE, format!("bytes={}-", expected.len()))
                },
            )
            .await
        })
        .await?;
        assert_eq!(invalid.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            invalid.headers().get(header::CONTENT_RANGE).unwrap(),
            format!("bytes */{}", expected.len()).as_str()
        );
        assert!(
            !peer_step(
                "check audio remains unmaterialized",
                is_materialized(
                    a.global_params.test_library().main_db.as_ref(),
                    &a.global_params.test_library().fsio,
                    first.path(),
                    remote_id
                )
            )
            .await?
        );
        let (_, authorized) = peer_step("authenticate sync-only client", async {
            sync_worker::authenticated_client(
                &b.global_params.node,
                &b_info.fingerprint,
                &b.private_key,
                &a_info.fingerprint,
                "127.0.0.1",
                a_addr.port(),
            )
            .await
        })
        .await?;
        // Pool-backed data routes are available on the sync listener.
        for (path, status) in [
            ("/files/anything", StatusCode::BAD_REQUEST),
            ("/ws", StatusCode::NOT_FOUND),
            ("/panel/users", StatusCode::NOT_FOUND),
        ] {
            let actual = peer_step("sync-only listener route", async {
                Ok(authorized
                    .get(format!("{base}{path}"))
                    .send()
                    .await?
                    .status())
            })
            .await?;
            assert_eq!(actual, status);
        }
        peer_step(
            "upgrade to full listener",
            a.manager.clone().start(
                a_addr,
                DiscoveryParams {
                    device_info: a_info.clone(),
                },
            ),
        )
        .await?;
        peer_step(
            "stop background worker before direct full-listener auth",
            async { a.stop_worker().await },
        )
        .await?;
        let (_, full_authorized) = peer_step("authenticate full listener client", async {
            authenticated_client(
                &b.global_params.node,
                &b_info.fingerprint,
                &b.private_key,
                &a_info.fingerprint,
                "127.0.0.1",
                a_addr.port(),
            )
            .await
        })
        .await?;
        for path in [
            "/files/anything",
            "/media/cover/1",
            "/media/metadata/1",
            "/ws",
        ] {
            let request = |client: &reqwest::Client| {
                let request = client.get(format!("{base}{path}"));
                if path == "/ws" {
                    request
                        .header("Connection", "upgrade")
                        .header("Upgrade", "websocket")
                        .header("Sec-WebSocket-Version", "13")
                        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
                } else {
                    request
                }
            };
            let anonymous_status = peer_step("anonymous full-listener request", async {
                Ok(request(&pinned).send().await?.status())
            })
            .await?;
            assert_eq!(anonymous_status, StatusCode::UNAUTHORIZED, "{path}");
            let authorized_status = peer_step("authorized full-listener request", async {
                Ok(request(&full_authorized).send().await?.status())
            })
            .await?;
            assert_ne!(authorized_status, StatusCode::UNAUTHORIZED, "{path}");
        }
        let query = urlencoding::encode(&b_info.fingerprint);
        let anonymous_ws_status = peer_step("anonymous websocket request", async {
            Ok(pinned
                .get(format!("{base}/ws?fingerprint={query}"))
                .header("Connection", "upgrade")
                .header("Upgrade", "websocket")
                .header("Sec-WebSocket-Version", "13")
                .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
                .send()
                .await?
                .status())
        })
        .await?;
        assert_eq!(anonymous_ws_status, StatusCode::UNAUTHORIZED);
        let full = peer_step("panel login route", async {
            Ok(pinned
                .post(format!("{base}/panel/auth/login"))
                .send()
                .await?
                .status())
        })
        .await?;
        assert_ne!(full, StatusCode::NOT_FOUND);
        peer_step("stop full listener", a.stop()).await?;
        peer_step(
            "restart sync listener",
            a.clone().start_sync(
                a_addr,
                DiscoveryParams {
                    device_info: a_info,
                },
            ),
        )
        .await?;
        let restarted_client = peer_step(
            "build restarted listener client",
            pinned_anonymous_client(&b.global_params, Some(0)),
        )
        .await?;
        wait_until("restarted listener readiness", || async {
            let ready = if let Ok(response) = restarted_client
                .get(format!("{base}/ping"))
                .header(CONNECTION, "close")
                .send()
                .await
                && response.status().is_success()
            {
                true
            } else {
                false
            };
            Ok(ready.then_some(()))
        })
        .await?;
        let restarted_status = peer_step("restarted listener request", async {
            Ok(restarted_client
                .get(format!("{base}/files/anything"))
                .send()
                .await?
                .status())
        })
        .await?;
        assert_eq!(restarted_status, StatusCode::UNAUTHORIZED);
        b.global_params.node.token.cancel();
        let rebound = wait_until("cancelled listener to release its port", || async {
            Ok(TcpListener::bind(b_addr).ok())
        })
        .await?;
        drop(rebound);
        peer_step("stop cancelled second listener", b.stop()).await?;
        peer_step("stop restarted first listener", a.stop()).await?;
        a.global_params.node.token.cancel();
        let rebound = TcpListener::bind(a_addr)?;
        drop(rebound);
        Ok(())
    }

    /// A pinned TLS audio server whose blob endpoint answers correct ranges unless its faults
    /// say otherwise, with a client node authorized against it and a `RenewableAudio` pointed at
    /// its audio blob: the production client side of peer audio, validation included.
    struct TlsAudioFixture {
        client: Fixture,
        audio: super::super::sync_worker::RenewableAudio,
        faults: TlsAudioFaults,
        server: tokio::task::JoinHandle<()>,
        _server_dir: tempfile::TempDir,
    }

    #[derive(Clone)]
    struct TlsAudioFaults {
        reject_next_authorize: std::sync::Arc<AtomicBool>,
        corrupt_blob_ranges: std::sync::Arc<AtomicBool>,
    }

    async fn tls_audio_fixture(
        failure_reason: Option<std::sync::Arc<StdMutex<Option<UnavailableReason>>>>,
    ) -> Result<TlsAudioFixture> {
        use crate::server::sync_auth::require_auth;
        use crate::server::sync_worker::{AudioCredentials, RenewableAudio};
        use axum::extract::State as AxumState;
        use axum::http::HeaderMap;
        use axum::response::{IntoResponse, Response};
        use discovery::ssl;
        use std::sync::atomic::AtomicBool;

        #[derive(Clone)]
        struct Faults {
            reject_next_authorize: std::sync::Arc<AtomicBool>,
            corrupt_blob_ranges: std::sync::Arc<AtomicBool>,
        }

        async fn blob(AxumState(faults): AxumState<Faults>, headers: HeaderMap) -> Response {
            const BODY: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ!?";
            let Some(range) = headers
                .get(header::RANGE)
                .and_then(|value| value.to_str().ok())
            else {
                return (StatusCode::OK, BODY).into_response();
            };
            let spec = range.strip_prefix("bytes=").unwrap_or(range);
            let (start, end) = spec.split_once('-').unwrap_or((spec, ""));
            let start: usize = start.parse().unwrap();
            let end = if end.is_empty() {
                BODY.len() - 1
            } else {
                end.parse::<usize>().unwrap().min(BODY.len() - 1)
            };
            // The fault of a broken peer: the body answers the asked range, but the
            // Content-Range header names a different one. TLS cannot corrupt an answer this
            // way, so the only place such a response can exist is the serving side itself.
            let header_start = if faults.corrupt_blob_ranges.load(Ordering::SeqCst) {
                start.saturating_sub(1)
            } else {
                start
            };
            let mut response =
                (StatusCode::PARTIAL_CONTENT, BODY[start..=end].to_vec()).into_response();
            response.headers_mut().insert(
                header::CONTENT_RANGE,
                format!("bytes {header_start}-{end}/{}", BODY.len())
                    .parse()
                    .unwrap(),
            );
            response
        }

        // The gate that lets a test reject one re-authorization: the first authorize request
        // after the flag is raised answers 401, and the flag falls so the renewal that follows
        // goes through. A test that raises it must see it consumed, or its rejection never
        // reached a request.
        async fn reject_one_authorize(
            AxumState(faults): AxumState<Faults>,
            request: axum::extract::Request,
            next: axum::middleware::Next,
        ) -> Response {
            if request.uri().path() == "/sync/authorize"
                && faults.reject_next_authorize.swap(false, Ordering::SeqCst)
            {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            next.run(request).await
        }

        let client_dir = tempfile::tempdir()?;
        let client = fixture(client_dir.path()).await?;
        client.stop_worker().await?;
        let client_info = info(&client).await?;
        let server_cert = ssl::generate_self_signed_cert("test", "rune", "CA", 1)?;
        let server_fingerprint = server_cert.public_key_fingerprint.clone();
        let server_dir = tempfile::tempdir()?;
        let server_permissions = Arc::new(RwLock::new(PermissionManager::new(server_dir.path())?));
        server_permissions
            .write()
            .await
            .add_user(
                client.certificate.clone(),
                client_info.fingerprint.clone(),
                "client".into(),
                "test".into(),
                DeviceType::Desktop,
                "127.0.0.1".into(),
            )
            .await?;
        server_permissions
            .write()
            .await
            .change_user_status(&client_info.fingerprint, UserStatus::Approved)
            .await?;
        let auth = SyncAuth::new(server_permissions, server_fingerprint.clone());
        let faults = Faults {
            reject_next_authorize: std::sync::Arc::new(AtomicBool::new(false)),
            corrupt_blob_ranges: std::sync::Arc::new(AtomicBool::new(false)),
        };
        let blob_router = Router::new()
            .route("/sync/blobs/{digest}", get(blob))
            .with_state(faults.clone())
            .layer(middleware::from_fn_with_state(auth.clone(), require_auth));
        let app = auth
            .router()
            .merge(blob_router)
            .layer(middleware::from_fn_with_state(
                faults.clone(),
                reject_one_authorize,
            ));
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        client
            .global_params
            .permission_manager
            .write()
            .await
            .add_user(
                server_cert.certificate.clone(),
                server_fingerprint.clone(),
                "server".into(),
                "test".into(),
                DeviceType::Desktop,
                "127.0.0.1".into(),
            )
            .await?;
        client
            .global_params
            .permission_manager
            .write()
            .await
            .change_user_status(&server_fingerprint, UserStatus::Approved)
            .await?;
        client
            .global_params
            .cert_validator
            .read()
            .await
            .replace_hosts_for_fingerprint(
                &server_fingerprint,
                vec![format!("127.0.0.1:{}", address.port())],
            )
            .await?;
        let tls = RustlsConfig::from_pem(
            server_cert.certificate.as_bytes().to_vec(),
            server_cert.private_key.as_bytes().to_vec(),
        )
        .await?;
        let handle = Handle::new();
        let server = tokio::spawn(async move {
            axum_server::from_tcp_rustls(listener, tls)
                .handle(handle)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                .await
                .unwrap();
        });

        let (_, http_client) = authenticated_client(
            &client.global_params.node,
            &client_info.fingerprint,
            &client.private_key,
            &server_fingerprint,
            "127.0.0.1",
            address.port(),
        )
        .await?;
        let url = format!("https://127.0.0.1:{}/sync/blobs/audio", address.port());
        let audio = RenewableAudio {
            global: Arc::downgrade(&client.global_params.node),
            own: client_info.fingerprint.clone(),
            private_key: client.private_key.clone(),
            endpoint: PeerEndpoint {
                fingerprint: server_fingerprint.clone(),
                trusted_host: format!("127.0.0.1:{}", address.port()),
                host: "127.0.0.1".into(),
                port: address.port(),
            },
            url: url.clone(),
            access_url: url,
            size: 64,
            client: Arc::new(Mutex::new(AudioCredentials {
                client: http_client,
                generation: 0,
            })),
            cancel: CancellationToken::new(),
            failure_reason,
        };
        Ok(TlsAudioFixture {
            client,
            audio,
            faults: TlsAudioFaults {
                reject_next_authorize: faults.reject_next_authorize.clone(),
                corrupt_blob_ranges: faults.corrupt_blob_ranges.clone(),
            },
            server,
            _server_dir: server_dir,
        })
    }

    #[tokio::test]
    async fn concurrent_ranged_audio_reads_recover_from_one_authorization_rejection() -> Result<()>
    {
        use crate::server::sync_auth::TOKEN_TTL;
        use playback::player::AudioRequest;

        let fixture = tls_audio_fixture(None).await?;
        let audio = fixture.audio.clone();
        tokio::time::pause();
        tokio::time::advance(TOKEN_TTL + Duration::from_secs(1)).await;
        tokio::time::resume();
        fixture
            .faults
            .reject_next_authorize
            .store(true, Ordering::SeqCst);

        let (first, second) = tokio_time::timeout(TEST_DEADLINE, async {
            tokio::join!(
                audio.get(Some((0, Some(15)))),
                audio.get(Some((16, Some(31))))
            )
        })
        .await
        .context("ranged audio reads did not finish")?;
        let first = first?;
        let second = second?;
        assert_eq!(first.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(second.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(first.bytes().await?.as_ref(), &b"0123456789abcdef"[..]);
        assert_eq!(second.bytes().await?.as_ref(), &b"ghijklmnopqrstuv"[..]);
        assert!(
            !audio.cancel.is_cancelled(),
            "a transient rejection cancelled the authorized stream"
        );
        assert!(
            !fixture.faults.reject_next_authorize.load(Ordering::SeqCst),
            "the authorization rejection never reached a request"
        );

        fixture.server.abort();
        fixture.client.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn a_corrupt_ranged_audio_response_fails_typed_and_reaches_the_failure_slot() -> Result<()>
    {
        use playback::player::AudioRequest;

        // The production client of peer audio, validation included: a peer whose Content-Range
        // does not name the asked range must be rejected, and the typed cause must reach the
        // failure slot of the track, because the io errors of the transport cannot carry it.
        let failure_reason = std::sync::Arc::new(StdMutex::new(None));
        let fixture = tls_audio_fixture(Some(failure_reason.clone())).await?;
        fixture
            .faults
            .corrupt_blob_ranges
            .store(true, Ordering::SeqCst);

        let result = tokio_time::timeout(TEST_DEADLINE, fixture.audio.get(Some((1, Some(16)))))
            .await
            .context("the corrupt ranged read did not finish")?;
        let error = result.expect_err("a corrupt content range must not be served to playback");
        anyhow::ensure!(
            UnavailableReason::classify(&error) == UnavailableReason::SourceUnreachable,
            "corrupt range classified as {:?}, not unreachable: {error:#}",
            UnavailableReason::classify(&error)
        );
        anyhow::ensure!(
            *failure_reason.lock().unwrap() == Some(UnavailableReason::SourceUnreachable),
            "the failure slot did not carry the typed cause: {:?}",
            *failure_reason.lock().unwrap()
        );

        // The same peer answering correctly is accepted: the rejection is the corruption, not
        // the client.
        fixture
            .faults
            .corrupt_blob_ranges
            .store(false, Ordering::SeqCst);
        let response = tokio_time::timeout(TEST_DEADLINE, fixture.audio.get(Some((0, Some(15)))))
            .await
            .context("the correct ranged read did not finish")??;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.bytes().await?.as_ref(), &b"0123456789abcdef"[..]);

        fixture.server.abort();
        fixture.client.global_params.node.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn updated_track_never_selects_stale_local_audio() -> Result<()> {
        use database::actions::{
            file::get_media_files,
            metadata::{get_metadata_summary_by_files, scan_audio_library},
        };
        use playback::player::{PlaybackSource, PlayingItem};
        let root = tempfile::tempdir()?;
        let manager = fixture(root.path()).await?;
        let params = &manager.global_params;
        let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        fs::copy(
            fixture_dir.join("startup_0.ogg"),
            root.path().join("old.ogg"),
        )?;
        scan_audio_library(
            &params.test_library().fsio,
            &params.test_library().main_db,
            &params.node_id,
            root.path(),
            ScanOptions {
                cleanup: false,
                force: false,
                missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
                cancel_token: None,
            },
            |_| {},
            &params.test_library().library_sync.record_store(),
        )
        .await?;
        let store = LibrarySyncStore::open(
            (*params.test_library().main_db).clone(),
            params.node_id.parse()?,
            None,
        )
        .await?;
        store.import_existing(params.node_id.parse()?).await?;
        let file = get_media_files(&params.test_library().main_db, 0, 10)
            .await?
            .pop()
            .context("missing scanned audio")?;
        let track_id: Uuid = file.source_uuid.parse()?;
        let resolver = sync_worker::PairedAudioResolver(
            Arc::downgrade(&manager.global_params.node),
            manager
                .global_params
                .library_id
                .context("library")?
                .to_string(),
        );
        let queued_before_revision = PlaybackSource::Local(root.path().join("old.ogg"));
        assert!(
            !queued_before_revision
                .requires_stream_at_load(file.id, &resolver)
                .await?
        );
        let blob = hash_file(
            &params.test_library().fsio,
            &fixture_dir.join("startup_1.ogg"),
        )?;
        store
            .write_local(
                ObjectKey {
                    collection: Collection::Tracks,
                    id: track_id,
                },
                Some(Payload::Track(Track {
                    extension: "ogg".into(),
                    content_hash: Some(blob.digest.clone()),
                    content_size: Some(blob.size.try_into()?),
                    duration: "0".to_owned().try_into().unwrap(),
                    sample_rate: 0,
                    cover_art_id: None,
                    metadata: vec![MetadataTag {
                        id: Uuid::new_v4(),
                        position: 0,
                        key: "track_title".into(),
                        value: "new content".into(),
                    }],
                })),
            )
            .await?;
        // A queue entry created before the revision is rechecked when loaded.
        assert!(
            queued_before_revision
                .requires_stream_at_load(file.id, &resolver)
                .await?
        );
        let handles = vec![file.clone().into()];
        let queued = crate::utils::files_to_playback_request(
            &params.test_library().fsio,
            &params.test_library().main_db,
            &params.library_id.context("library")?.to_string(),
            &root.path(),
            &handles,
        )
        .await?;
        assert!(
            matches!(&queued[..], [(PlayingItem::LibraryTrack { track, .. }, PlaybackSource::LibraryStream)] if *track == file.source_uuid)
        );
        let summaries = get_metadata_summary_by_files(
            params.test_library().main_db.as_ref(),
            vec![file.clone()],
        )
        .await?;
        let parsed = crate::utils::parse_media_files(
            &params.test_library().fsio,
            &params.test_library().main_db,
            &params.node_id,
            &params.test_library().library_id.to_string(),
            summaries,
            Arc::new(params.test_library().media_root.clone()),
        )
        .await?;
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].path.is_empty());
        assert_eq!(
            catalog_track(&params.test_library().main_db, file.id).await?,
            Some((track_id, blob.clone(), "ogg".into()))
        );
        use playback::player::PlaybackSourceResolver;
        let queued_remote = PlaybackSource::LibraryStream;
        let updated_path = prepare_download(
            &params.test_library().main_db,
            &params.test_library().fsio,
            root.path(),
            track_id,
            &blob,
            "ogg",
        )
        .await?;
        fs::copy(fixture_dir.join("startup_1.ogg"), &updated_path)?;
        let scan_error = scan_audio_file(
            &params.test_library().fsio,
            &params.test_library().main_db,
            &params.node_id,
            root.path(),
            &updated_path,
            &store.record_store(),
        )
        .await
        .unwrap_err();
        assert!(
            scan_error
                .to_string()
                .contains("Audio file was not indexed")
        );
        attach_download(
            &params.test_library().main_db,
            &store.record_store(),
            &params.test_library().fsio,
            root.path(),
            track_id,
            &blob,
            &updated_path,
        )
        .await?;
        assert!(
            !queued_remote
                .requires_stream_at_load(file.id, &resolver)
                .await?
        );
        assert_eq!(
            resolver.local_path(file.id).await?,
            Some(
                params
                    .test_library()
                    .fsio
                    .canonicalize_path(&updated_path)?
            )
        );
        assert!(
            !queued_before_revision
                .requires_stream_at_load(file.id, &resolver)
                .await?
        );
        let current = get_media_files(&params.test_library().main_db, 0, 10)
            .await?
            .pop()
            .context("missing updated audio")?;
        assert_eq!(current.id, file.id);
        let file_uuid = Uuid::parse_str(&file.source_uuid)?;
        assert_eq!(resolver.resolve_id(file_uuid).await?, Some(current.id));
        params.main_token.cancel();
        Ok(())
    }

    #[tokio::test]
    async fn panel_token_is_accepted_until_it_is_tampered_with_or_expires() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let manager = fixture(dir.path()).await?;

        let token = manager.generate_jwt_token(None)?;
        assert!(manager.verify_jwt_token(&token));

        let (signed, signature) = token.rsplit_once('.').context("token signature")?;
        let (header, claims) = signed.split_once('.').context("token claims")?;
        let forged_claims = URL_SAFE_NO_PAD.encode(br#"{"exp":99999999999,"iat":0}"#);
        assert_ne!(forged_claims, claims);
        assert!(!manager.verify_jwt_token(&format!("{header}.{forged_claims}.{signature}")));

        // The second named by `exp` is already past the token's life.
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as usize;
        let expiring_now = encode(
            &Header::new(Algorithm::HS256),
            &JwtClaims {
                exp: now,
                iat: now - 60,
            },
            &EncodingKey::from_secret(&manager.jwt_secret),
        )?;
        assert!(
            !manager.verify_jwt_token(&expiring_now),
            "token was accepted when exp == now"
        );

        // A validity cannot be negative, so the expired token is signed here with the
        // manager's own secret: only its expiry can be what gets it rejected.
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as usize;
        let expired = encode(
            &Header::new(Algorithm::HS256),
            &JwtClaims {
                exp: now - 3600,
                iat: now - 7200,
            },
            &EncodingKey::from_secret(&manager.jwt_secret),
        )?;
        assert!(!manager.verify_jwt_token(&expired));

        manager.global_params.main_token.cancel();
        Ok(())
    }

    #[tokio::test]
    async fn panel_middleware_admits_only_requests_with_a_generated_token() -> Result<()> {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let dir = tempfile::tempdir()?;
        let manager = fixture(dir.path()).await?;
        let router = Router::new()
            .route("/panel/probe", get(|| async { "handler ran" }))
            .layer(middleware::from_fn(auth_middleware))
            .layer(Extension(manager.manager.clone()));
        let status_for = |authorization: Option<String>| {
            let router = router.clone();
            async move {
                let mut request = Request::builder().uri("/panel/probe");
                if let Some(value) = authorization {
                    request = request.header(header::AUTHORIZATION, value);
                }
                let response = router.oneshot(request.body(Body::empty())?).await?;
                anyhow::Ok(response.status())
            }
        };

        let token = manager.generate_jwt_token(None)?;
        assert_eq!(
            status_for(Some(format!("Bearer {token}"))).await?,
            StatusCode::OK
        );
        assert_eq!(status_for(None).await?, StatusCode::UNAUTHORIZED);
        assert_eq!(
            status_for(Some("Bearer not-a-token".to_owned())).await?,
            StatusCode::UNAUTHORIZED
        );

        // The panel's tokens are issued and checked by this one device, so there is no
        // clock difference to allow for: a token is refused from the second it expires.
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as usize;
        let just_expired = encode(
            &Header::new(Algorithm::HS256),
            &JwtClaims {
                exp: now - 10,
                iat: now - 20,
            },
            &EncodingKey::from_secret(&manager.jwt_secret),
        )?;
        assert_eq!(
            status_for(Some(format!("Bearer {just_expired}"))).await?,
            StatusCode::UNAUTHORIZED
        );

        manager.global_params.main_token.cancel();
        Ok(())
    }
}
