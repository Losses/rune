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

fn coalesce_worker_notify(
    notify: Arc<dyn Fn() + Send + Sync>,
    cancel: tokio_util::sync::CancellationToken,
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
                _ = tokio::time::sleep_until(deadline) => {},
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
pub struct JwtClaims {
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
    sync_cancel: Mutex<Option<tokio_util::sync::CancellationToken>>,
    sync_handle: Mutex<Option<JoinHandle<()>>>,
    stream_handles: Mutex<Vec<JoinHandle<()>>>,
    sync_auth: Mutex<Option<SyncAuth>>,
    watch_cancel: Mutex<Option<tokio_util::sync::CancellationToken>>,
    watch_handle: Mutex<Option<JoinHandle<()>>>,
    certificate: String,
    private_key: String,
    pub jwt_secret: Vec<u8>,
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
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>> {
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

        let node_id = uuid::Uuid::parse_str(&node.node_id)?;
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
            first_error = Some(anyhow::Error::from(error));
        }
        let streams = std::mem::take(&mut *self.stream_handles.lock().await);
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
            first_error = Some(anyhow::Error::from(error));
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
    ) -> Result<playback::player::RemoteAudioSource> {
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
                        http_request::sync::MetadataPeer::<RuneModel>::new(&base, client.clone());
                    use sync::SyncPeer;
                    anyhow::ensure!(
                        peer.identity().await?.namespace == library_id,
                        "peer library mismatch"
                    );
                    let url = format!("{base}/sync/blobs/{}", blob.digest);
                    let access_url = format!("{url}?track_id={track_id}");
                    if matches!(response.status(), reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::NOT_FOUND) {
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
                    let request = Arc::new(super::sync_worker::RenewableAudio {
                        global: Arc::downgrade(&node), own: own.clone(), private_key: self.private_key.clone(),
                        endpoint: super::sync_worker::PeerEndpoint { fingerprint: fingerprint.clone(), trusted_host: trusted_host.clone(), host: host.clone(), port },
                        url: url.clone(), access_url, size: blob.size,
                        client: Arc::new(Mutex::new(super::sync_worker::AudioCredentials { client, generation: 0 })), cancel: cancellation.clone(),
                    });
                    let failure_reason = Arc::new(StdMutex::new(None));
                    let watcher_failure_reason = failure_reason.clone();
                    let watch_cancel = cancellation.clone();
                    let watch_params = Arc::downgrade(&node);
                    let watch_fingerprint = fingerprint.clone();
                    let revocation_host = trusted_host.clone();
                    let watch_request = request.clone();
                    let watcher = tokio::spawn(async move {
                        let access_probe = async {
                            loop {
                                tokio::time::sleep(Duration::from_secs(1)).await;
                                if let Err(error) = watch_request.check_library_access().await {
                                    if watch_cancel.is_cancelled() {
                                        break;
                                    }
                                    if let Ok(mut reason) = watcher_failure_reason.lock() {
                                        *reason = Some(UnavailableReason::classify(&error));
                                    }
                                    watch_cancel.cancel();
                                    log::debug!("Audio library access check failed: {error:#}");
                                }
                            }
                        };
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
                    Ok::<_, anyhow::Error>(playback::player::RemoteAudioSource {
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

    pub fn verify_jwt_token(&self, token: &str) -> bool {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = true;
        validation.leeway = 0;

        decode::<JwtClaims>(
            token,
            &DecodingKey::from_secret(&self.jwt_secret),
            &validation,
        )
        .is_ok()
    }
}

pub async fn get_or_generate_alias(config_path: &Path) -> Result<String> {
    info!("Generating certificate in: {config_path:?}");

    let certificate_id_path = config_path.join("cid");
    if certificate_id_path.exists() {
        tokio::fs::read_to_string(&certificate_id_path)
            .await
            .map(|s| s.trim().to_string())
            .context("Failed to read cid file")
    } else {
        let certificate_id = generate_random_alias();
        tokio::fs::write(&certificate_id_path, &certificate_id)
            .await
            .context("Failed to save cid file")?;
        Ok(certificate_id)
    }
}

pub async fn update_alias(config_path: &Path, new_alias: &str) -> Result<()> {
    info!("Updating certificate alias in: {config_path:?} to: {new_alias}");

    let certificate_id_path = config_path.join("cid");

    tokio::fs::write(&certificate_id_path, new_alias)
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
        let cert = tokio::fs::read_to_string(&cert_path)
            .await
            .context("Failed to read certificate")?;
        let private_key = tokio::fs::read_to_string(&key_path)
            .await
            .context("Failed to read private key")?;

        let (_, fingerprint) = parse_certificate(&cert)?;
        Ok((fingerprint, cert, private_key))
    } else {
        let cert = generate_self_signed_cert(certificate_id, "Rune Player", "NET", 3650)?;

        tokio::fs::write(&cert_path, &cert.certificate)
            .await
            .context("Failed to save certificate")?;
        tokio::fs::write(&key_path, &cert.private_key)
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

    tokio::fs::write(&password_path, &hash)
        .await
        .context("Failed to write password hash")?;

    Ok(())
}

#[cfg(test)]
mod lifecycle_tests {
    use super::super::sync_worker;
    use super::super::sync_worker::{
        PeerEndpoint, PeerPass, TestPeerSession, authenticated_client, test_peer_pass,
    };
    use super::super::utils::device::load_device_info;
    use crate::Signal;
    use crate::macros::dispatch_remote;
    use crate::messages::*;
    use crate::request_error::classify;
    use crate::server::WebSocketService;
    use crate::utils::{ParamsExtractor, register_local_library_on_node};
    use migration::catalog_identity::{ParentKind, parent};
    use playback::decoder::RuneDecoder;
    use playback::stream_utils::{ManagedStream, create_stream_with_authorizer};
    use std::net::SocketAddr;
    use std::net::TcpListener;
    use std::path::Path;
    use tracing_subscriber::filter::LevelFilter;
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
            tokio::time::advance(Duration::from_millis(1)).await;
        }
        tokio::time::advance(Duration::from_millis(500)).await;
        tokio::task::yield_now().await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        notify();
        tokio::time::advance(Duration::from_millis(500)).await;
        tokio::task::yield_now().await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
        cancel.cancel();
    }

    use super::super::libraries::ListedLibrary;
    use super::*;
    use crate::utils::nid::get_or_create_node_id;
    use crate::utils::{
        Broadcaster, GlobalParams, RinfRustSignal, RunningMode, TaskTokens,
        sync_settings::SyncSettings,
    };
    use database::actions::downloads::{prepare_download, set_download_rule};
    use database::actions::file::{attach_download, catalog_track, hash_file, is_materialized};
    use database::actions::metadata::{
        MissingHostFilePolicy, ScanOptions, scan_audio_file, scan_audio_library,
    };
    use database::connection::StorageMode;
    use database::entities::{media_files, media_files::Column};
    use database::library_registry::{
        DbPlacement, LibraryKind, LibraryRecord, LibraryRecordId, LibraryRegistry, OwnedStorage,
        SubsonicOpenMode, create_replica, resolve_library,
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
    use std::{fs, ops::Deref, sync::OnceLock};
    use tokio::net::TcpStream;
    use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    use crate::backends::remote::encode_message;
    use crate::backends::remote::{connect_remote, decode_message};
    use crate::handlers::neighbors::select_pinned_pair_host;
    use crate::library_pool::{LibraryHandle, LibraryPool};
    use crate::messages::{
        DeviceConnectionLevel, DeviceSyncLevel, LibraryInitializeMode, OpenLibraryRequest,
        OpenLibraryResult, PlaybackStatus, RegisterLocalLibraryRequest,
        RegisterRemoteControlRequest, RequestErrorKind, RequestFailureCause, TrustListUpdated,
        TrustedServerCertificate,
    };
    use crate::node::{NodeSlot, ReconnectConfig};
    use crate::request_error::{ExpectedRequestError, request_failure_cause};
    use crate::server::api::{add_trusted_server, edit_trusted_hosts};
    use crate::server::sync::client::sync_library;
    use database::sync::model::RuneModel;
    use futures::{SinkExt, StreamExt};
    use reqwest::header::CONNECTION;
    use sync::{CheckpointKey, ReceiveStore, Scope, SyncModel, scope_id};
    use tokio::time::timeout;
    use tokio_tungstenite::tungstenite::Message;

    include!("access_policy_tests.rs");
    mod player_session_tests {
        use super::*;
        include!("player_session_tests.rs");
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
                fingerprint: fingerprint.to_owned(),
                alias: "Test remote".into(),
            },
            &node,
        )
        .await;
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
                call_id: 1,
                record_id: record_id.clone(),
            },
            node.clone(),
            Arc::new(Mutex::new(ScrobblingManager::new(
                10,
                Duration::from_secs(5),
            ))),
        )
        .await;
        match opened.result {
            OpenLibraryResult::Opened => node.current_ui().await,
            OpenLibraryResult::Failed(failure) => Err(anyhow::anyhow!(failure.detail)),
        }
    }

    async fn peer_step<T>(
        name: &'static str,
        future: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        peer_step_with_timeout(name, Duration::from_secs(15), future).await
    }

    /// A diagnostic watchdog: it names the step a hung test is stuck in. The limit is far
    /// above any normal step, even on a loaded host, and still well below the suite limit.
    async fn step<T>(name: &'static str, future: impl Future<Output = T>) -> T {
        step_with_timeout(name, Duration::from_secs(300), future).await
    }

    async fn step_with_timeout<T>(
        name: &'static str,
        limit: Duration,
        future: impl Future<Output = T>,
    ) -> T {
        eprintln!("test step started: {name}");
        match tokio::time::timeout(limit, future).await {
            Ok(result) => {
                eprintln!("test step finished: {name}");
                result
            }
            Err(_) => panic!("test step did not finish: {name}"),
        }
    }

    async fn peer_step_with_timeout<T>(
        name: &'static str,
        limit: Duration,
        future: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        tokio::time::timeout(limit, future)
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
        let mut builder = reqwest::Client::builder()
            .use_preconfigured_tls(tls)
            .no_proxy();
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
        fs::create_dir_all(dir.join(".rune"))?;
        let node = get_or_create_node_id(dir.to_str().context("config path")?).await?;
        let pool = LibraryPool::with_idle_timeout(dir, dir, &node.to_string(), idle_timeout)?;
        let _ = migration::initialize_node_id(node.to_string());
        LibraryRegistry::update(dir, |registry| {
            registry.libraries.push(LibraryRecord {
                record_id: LibraryRecordId(Uuid::new_v4()),
                alias: "Owned".into(),
                kind: LibraryKind::Owned {
                    library_id,
                    storage: OwnedStorage::Local {
                        path: dir.to_string_lossy().into_owned(),
                        placement: DbPlacement::Portable,
                    },
                    background_sync: true,
                },
            });
            Ok(())
        })
        .await?;
        let library = pool.acquire_ui(library_id).await?;
        SyncSettings {
            listen: false,
            ..SyncSettings::default()
        }
        .save(dir)?;
        let node_context = NodeContext::with_reconnect_config(
            dir.to_str().context("config path")?,
            dir.to_str().context("db path")?,
            broadcaster,
            false,
            reconnect_config,
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
                path: path.to_str().context("test library path")?.to_owned(),
                alias: "Test local".into(),
                mode: Some(LibraryInitializeMode::Portable),
            },
            &node,
        )
        .await;
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
        let node = NodeContext::new(config, config, Arc::new(Quiet), false).await?;
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
            task_tokens: Arc::new(Mutex::new(TaskTokens::default())),
            player: node_context.session()?.player.clone(),
            sfx_player: Arc::new(Mutex::new(SfxPlayer::new(Some((*token).clone())))),
            scrobbler: node_context.session()?.scrobbler.clone(),
            broadcaster: node_context.broadcaster.clone(),
            device_scanner: node_context.device_scanner.clone(),
            cert_validator: node_context.cert_validator.clone(),
            permission_manager: node_context.permission_manager.clone(),
            server_manager: OnceLock::new(),
            running_mode: RunningMode::Client,
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
        let payload = rinf::serialize(&GetMediaFilesCountRequest {})?;
        let (name, response) = service
            .handle_message(
                "GetMediaFilesCountRequest",
                payload,
                None,
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
        let close = CloseLibraryRequest {};
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
        let close = CloseLibraryRequest {};
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

    async fn info(manager: &Fixture) -> Result<discovery::utils::DeviceInfo> {
        load_device_info(Path::new(&*manager.global_params.config_path)).await
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
        let client = fixture(dir.path()).await?;
        let validator = client.global_params.cert_validator.read().await.clone();
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
        info: &discovery::utils::DeviceInfo,
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

        let capabilities: discovery::capabilities::Capabilities =
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
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
        server.global_params.node.shutdown().await?;
        peer.global_params.node.shutdown().await?;
        Ok(())
    }

    async fn close_ui(params: &GlobalParams) -> Result<()> {
        if params.library.is_some() {
            let request = CloseLibraryRequest {};
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
        std::fs::create_dir_all(&root)?;
        let registered = register_local_library_on_node(
            &RegisterLocalLibraryRequest {
                path: root.to_string_lossy().into_owned(),
                alias: "Removable".into(),
                mode: Some(LibraryInitializeMode::Portable),
            },
            &owner.global_params.node,
        )
        .await;
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
            record_id: record.record_id.0.to_string(),
        };
        let response = request
            .handle(owner.global_params.node.clone(), None, &request)
            .await?
            .context("remove response")?;
        anyhow::ensure!(response.outcome == RemoveLibraryRecordOutcome::Removed);
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
                path: root.to_string_lossy().into_owned(),
                alias: "Removable".into(),
                mode: Some(LibraryInitializeMode::Portable),
            },
            &owner.global_params.node,
        )
        .await;
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
        let payload = rinf::serialize(&GetMediaFilesCountRequest {})?;
        let (name, bytes) =
            socket_request(&mut socket, "GetMediaFilesCountRequest", &payload).await?;
        assert_eq!(name, "RemoteRequestFailed");
        let failure: RemoteRequestFailed = rinf::deserialize(&bytes)?;
        assert_eq!(failure.kind, RequestErrorKind::Unavailable);
        assert!(failure.detail.contains("no library is open on this device"));
        let node = a.global_params.node.clone();
        let reopened =
            fixture_from_library(node.clone(), node.library_pool.acquire_ui(id).await?).await?;
        let (name, bytes) =
            socket_request(&mut socket, "GetMediaFilesCountRequest", &payload).await?;
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
            config_path: first.path().to_string_lossy().into_owned(),
        };
        let node = slot
            .initialize(&request.config_path, &request.config_path, Arc::new(Quiet))
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
            let response = slot.handle(&request, Arc::new(Quiet)).await;
            assert!(response.success);
            assert!(response.error_kind.is_none());
            let same = slot
                .initialize(&request.config_path, &request.config_path, Arc::new(Quiet))
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
        let response = slot
            .handle(
                &InitializeNodeRequest {
                    config_path: second.path().to_string_lossy().into_owned(),
                },
                Arc::new(Quiet),
            )
            .await;
        assert!(!response.success);
        assert_eq!(response.error_kind, Some(RequestErrorKind::InvalidInput));
        assert!(
            response
                .error
                .context("path error")?
                .contains("configuration path cannot change")
        );
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
        Ok(())
    }

    async fn paired_full(a: &Fixture, b: &Fixture) -> Result<SocketAddr> {
        paired_full_with_protocol_version(a, b, discovery::protocol_version::PROTOCOL_VERSION).await
    }

    async fn paired_full_with_protocol_version(
        a: &Fixture,
        b: &Fixture,
        protocol_version: discovery::protocol_version::ProtocolVersion,
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
        a.get_address().await.context("missing listener address")
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

    async fn assert_websocket_request_is_ignored(name: &str, payload: Vec<u8>) -> Result<()> {
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
        socket
            .send(Message::Binary(
                encode_message(name, &payload, Some(ignored_id)).into(),
            ))
            .await?;
        let payload = rinf::serialize(&GetMediaFilesCountRequest {})?;
        socket
            .send(Message::Binary(
                encode_message("GetMediaFilesCountRequest", &payload, Some(following_id)).into(),
            ))
            .await?;
        timeout(Duration::from_secs(5), async {
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
        let payload = rinf::serialize(&FetchRemoteFileRequest { url: String::new() })?;
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

        timeout(Duration::from_secs(2), first_entered.notified())
            .await
            .context("first handler did not enter")?;
        assert!(
            timeout(Duration::from_millis(200), second_entered.notified())
                .await
                .is_err(),
            "second frame ran before the first handler finished"
        );

        release_first.notify_one();
        timeout(Duration::from_secs(2), second_entered.notified())
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
        for remove in [false, true] {
            let auth_token = if remove {
                None
            } else {
                let manager = a.global_params.node.manager()?;
                let auth = manager
                    .sync_auth
                    .lock()
                    .await
                    .clone()
                    .context("sync auth was not initialized")?;
                let token = auth.issue_token_for_test(&fingerprint).await;
                Some((auth, token))
            };
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
                PermissionManager::new(a.global_params.config_path.as_str())?
                    .change_user_status(&fingerprint, UserStatus::Blocked)
                    .await?;
            }
            let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
                .await?
                .context("socket closed without a revocation close frame")?
                .map_err(anyhow::Error::new)
                .context("reading the revocation close frame")?;
            assert!(
                matches!(frame, Message::Close(Some(frame)) if u16::from(frame.code) == 4403 && frame.reason == "revoked")
            );
            if let Some((auth, token)) = auth_token {
                tokio::time::timeout(Duration::from_secs(2), async {
                    loop {
                        if !auth.has_token_for_test(&token).await {
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .context("file-driven client block did not purge sync tokens")?;
            }
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if a.global_params
                        .node
                        .session()?
                        .attached_controllers()
                        .iter()
                        .all(|controller| controller.fingerprint != fingerprint)
                    {
                        return Ok::<_, anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await??;
            if !remove {
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
        }
        pair(&a, &b).await?;
        let mut socket = connect_remote(
            "127.0.0.1",
            addr.port(),
            config,
            validator.clone(),
            &b.global_params.node.sync_tokens,
        )
        .await?;
        let close_frame = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(5), socket.next())
                .await
                .context("server-stop close frame timed out")?
                .context("socket closed without a server-stop close frame")?
                .map_err(anyhow::Error::new)
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

    async fn socket_request(
        socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
        name: &str,
        payload: &[u8],
    ) -> Result<(String, Vec<u8>)> {
        use crate::backends::remote::{decode_message, encode_message};
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let id = Uuid::new_v4();
        socket
            .send(Message::Binary(
                encode_message(name, payload, Some(id)).into(),
            ))
            .await?;
        let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await?
            .context("socket closed")??;
        let (response_name, payload, response_id) =
            decode_message(&frame.into_data()).context("invalid frame")?;
        assert_eq!(response_id, id);
        Ok((response_name, payload))
    }

    async fn socket_request_for_library(
        socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
        name: &str,
        payload: &[u8],
        library_id: Uuid,
    ) -> Result<(String, Vec<u8>)> {
        use crate::backends::remote::{decode_message, encode_request};
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let id = Uuid::new_v4();
        socket
            .send(Message::Binary(
                encode_request(name, payload, id, library_id).into(),
            ))
            .await?;
        let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await?
            .context("socket closed")??;
        let (response_name, payload, response_id) =
            decode_message(&frame.into_data()).context("invalid frame")?;
        assert_eq!(response_id, id);
        Ok((response_name, payload))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn addressed_websocket_requests_select_and_authorize_the_named_library() -> Result<()> {
        use crate::Session;
        use crate::access_policy::{ConnectionLevel, DeviceGrants, LibraryGrants};
        use crate::backends::remote::connect_remote;
        use crate::messages::CreatePlaylistRequest;

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
            host: "127.0.0.2".into(),
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
        let count_request = rinf::serialize(&GetMediaFilesCountRequest {})?;

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

        // Legacy frames still use the currently open UI library until C6.
        let (name, payload) =
            socket_request(&mut socket, "GetMediaFilesCountRequest", &count_request).await?;
        assert_eq!(name, "GetMediaFilesCountResponse");
        let response: GetMediaFilesCountResponse = rinf::deserialize(&payload)?;
        assert_eq!(response.count, 2);

        // With no UI attached, the renderer still serves its explicitly addressed held library.
        renderer
            .global_params
            .node
            .close_ui(&renderer.global_params.main_token)
            .await;
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
        use crate::access_policy::DeviceGrants;
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
        let (name, _) = socket_request(
            &mut socket,
            "GetMediaFilesCountRequest",
            &rinf::serialize(&GetMediaFilesCountRequest {})?,
        )
        .await?;
        assert_eq!(name, "GetMediaFilesCountResponse");
        let (name, _) = socket_request(
            &mut socket,
            "PauseRequest",
            &rinf::serialize(&PauseRequest {})?,
        )
        .await?;
        assert_eq!(name, "");
        for (request, payload) in [
            (
                "CreatePlaylistRequest",
                rinf::serialize(&CreatePlaylistRequest {
                    name: "forbidden".into(),
                    group: "test".into(),
                })?,
            ),
            (
                "ScanAudioLibraryRequest",
                rinf::serialize(&ScanAudioLibraryRequest {
                    scan_id: "forbidden".into(),
                    force: false,
                })?,
            ),
        ] {
            let (name, payload) = socket_request(&mut socket, request, &payload).await?;
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
        let request = GetPlaylistByIdRequest { playlist_id: -1 };
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
        let request = GetMixByIdRequest { mix_id: -1 };
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
        for (name, payload) in [
            ("FetchParsedMediaFileRequest", vec![]),
            (
                "GetPlaylistByIdRequest",
                rinf::serialize(&GetPlaylistByIdRequest { playlist_id: -1 })?,
            ),
            (
                "GetMediaFilesCountRequest",
                rinf::serialize(&GetMediaFilesCountRequest {})?,
            ),
        ] {
            let (response_name, payload) = socket_request(&mut socket, name, &payload).await?;
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
        use crate::backends::remote::{connect_remote, encode_message};
        use crate::handlers::neighbors::authorized_get;
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
        async fn count(socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>) -> Result<()> {
            let payload = rinf::serialize(&GetMediaFilesCountRequest {})?;
            let (name, payload) =
                socket_request(socket, "GetMediaFilesCountRequest", &payload).await?;
            assert_eq!(name, "GetMediaFilesCountResponse");
            let response: GetMediaFilesCountResponse = rinf::deserialize(&payload)?;
            assert_eq!(response.count, 0);
            Ok(())
        }
        count(&mut socket).await?;
        let request = FetchRemoteFileRequest {
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
        assert!(
            tokio::time::timeout(Duration::from_millis(500), socket.next())
                .await
                .is_err()
        );
        count(&mut socket).await?;

        let name = format!("{}.txt", Uuid::new_v4());
        let bytes = b"remote file contents";
        std::fs::write(first.path().join(&name), bytes)?;
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
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.bytes().await?.as_ref(), bytes);
        for url in [
            format!("http://127.0.0.1:{}/files/library/{name}", addr.port()),
            format!("https://untrusted.invalid/files/library/{name}"),
        ] {
            let request = FetchRemoteFileRequest { url };
            let response = request
                .handle(request.extract_params(&b.global_params), None, &request)
                .await?
                .context("missing response")?;
            assert!(!response.success, "unexpected success for {}", request.url);
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
        let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
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
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        assert_eq!(
            response.json::<serde_json::Value>().await?,
            serde_json::json!({"error": "library_not_served"})
        );
        let unauthenticated = reqwest::Client::builder()
            .use_preconfigured_tls(
                Arc::new(b.global_params.cert_validator.read().await.clone()).into_client_config(),
            )
            .build()?;
        assert_eq!(
            unauthenticated.post(&url).send().await?.status(),
            reqwest::StatusCode::UNAUTHORIZED
        );
        let served = a.global_params.library_id.unwrap();
        let response = client
            .post(format!("{base}/libraries/{served}/sync/metadata"))
            .json(&operation)
            .send()
            .await?;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
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
            tokio::time::sleep(Duration::from_secs(1)).await;
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
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let host = fixture(first.path()).await?;
        let client = fixture(second.path()).await?;
        let address = paired_full(&host, &client).await?;
        host.stop_worker().await?;
        client.stop_worker().await?;
        let own = info(&client).await?.fingerprint;
        let peer = info(&host).await?.fingerprint;
        let auth = host
            .manager
            .sync_auth
            .lock()
            .await
            .clone()
            .context("host sync auth")?;

        for _ in 0..2 {
            authenticated_client(
                &client.global_params.node,
                &own,
                &client.private_key,
                &peer,
                "127.0.0.1",
                address.port(),
            )
            .await?;
        }
        assert_eq!(auth.challenge_count_for_test().await, 1);
        assert_eq!(auth.authorization_count_for_test().await, 1);

        let endpoint = PeerEndpoint {
            fingerprint: peer,
            trusted_host: format!("127.0.0.1:{}", address.port()),
            host: "127.0.0.1".into(),
            port: address.port(),
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
        assert_eq!(auth.challenge_count_for_test().await, 1);
        assert_eq!(auth.authorization_count_for_test().await, 1);

        anyhow::ensure!(
            auth.expire_tokens_for_fingerprint_for_test(&own).await,
            "host had no token to expire"
        );
        let (_, stale_client) = authenticated_client(
            &client.global_params.node,
            &own,
            &client.private_key,
            &endpoint.fingerprint,
            &endpoint.host,
            endpoint.port,
        )
        .await?;
        let url = format!("https://{address}/libraries");
        assert_eq!(
            stale_client.get(&url).send().await?.status(),
            reqwest::StatusCode::UNAUTHORIZED
        );
        sync_worker::invalidate_sync_token(
            &client.global_params.node,
            &endpoint.fingerprint,
            &endpoint.host,
            endpoint.port,
        );
        let (_, fresh_client) = authenticated_client(
            &client.global_params.node,
            &own,
            &client.private_key,
            &endpoint.fingerprint,
            &endpoint.host,
            endpoint.port,
        )
        .await?;
        assert!(fresh_client.get(&url).send().await?.status().is_success());
        assert_eq!(auth.challenge_count_for_test().await, 2);
        assert_eq!(auth.authorization_count_for_test().await, 2);
        host.global_params.node.shutdown().await?;
        client.global_params.node.shutdown().await?;
        Ok(())
    }

    async fn request_peer_libraries(
        manager: &Fixture,
        fingerprint: &str,
    ) -> Result<ListPeerLibrariesResponse> {
        // Forced passes and listings share the challenge endpoint's rate limit.
        tokio::time::sleep(Duration::from_secs(1)).await;
        let request = ListPeerLibrariesRequest {
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

    async fn request_add_library(
        manager: &Fixture,
        fingerprint: &str,
        library: &PeerLibrary,
    ) -> Result<String> {
        let request = AddRemoteLibraryRequest {
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
        anyhow::ensure!(response.error.is_none(), "add failed: {:?}", response.error);
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
                    kind: LibraryKind::RuneRemoteControl {
                        fingerprint: "remote".into(),
                    },
                },
                LibraryRecord {
                    record_id: subsonic_id,
                    alias: "Subsonic".into(),
                    kind: LibraryKind::Subsonic {
                        endpoint: "https://music.example".into(),
                        account: "alice".into(),
                        mode: SubsonicOpenMode::RemoteControl,
                    },
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
        std::fs::copy(
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
        let unknown = request_peer_libraries(&b, "unknown").await?;
        assert_eq!(
            unknown.error_kind,
            Some(LibraryOpenErrorKind::RemoteUntrusted)
        );
        let unauthed = reqwest::Client::builder()
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
            reqwest::StatusCode::UNAUTHORIZED
        );
        let listed = request_peer_libraries(&b, &fingerprint).await?;
        assert_eq!(listed.error_kind, None);
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
            tokio::time::sleep(Duration::from_secs(1)).await;
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
            .filter(Column::HlcUuid.eq(&original.hlc_uuid))
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
        assert_eq!(listed.error_kind, None);
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
        let saved = std::fs::read(&registry_path)?;
        std::fs::write(&registry_path, b"invalid registry")?;
        let malformed = request_peer_libraries(&replica, &fingerprint).await?;
        std::fs::write(&registry_path, saved)?;
        assert_eq!(
            malformed.error_kind,
            Some(LibraryOpenErrorKind::RemoteProtocol)
        );
        a.global_params
            .permission_manager
            .write()
            .await
            .change_user_status(&info(&b).await?.fingerprint, UserStatus::Pending)
            .await?;
        let denied = request_peer_libraries(&replica, &fingerprint).await?;
        assert_eq!(
            denied.error_kind,
            Some(LibraryOpenErrorKind::RemoteNotAuthorized)
        );
        a.stop().await?;
        let offline = request_peer_libraries(&replica, &fingerprint).await?;
        assert_eq!(
            offline.error_kind,
            Some(LibraryOpenErrorKind::RemoteUnreachable)
        );
        for manager in [&a, &b, &replica] {
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
        std::fs::copy(
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
            library_id: id.to_string(),
            enabled,
        };
        let response = request
            .handle(
                request.extract_params(&manager.global_params),
                None,
                &request,
            )
            .await?
            .context("option response")?;
        anyhow::ensure!(
            response.error.is_none(),
            "option failed: {:?}",
            response.error
        );
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
        tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                if library.library_sync.get(key).await?.is_some() {
                    return Ok::<_, anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .context("node worker did not converge without a local UI library")?
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
            state: "Playing".into(),
            progress_seconds: 12.0,
            progress_percentage: 0.5,
            artist: None,
            album: None,
            title: None,
            duration: 24.0,
            index: None,
            item: None,
            item_request: None,
            playback_mode: 0,
            ready: true,
            cover_art_path: None,
            lib_path: a.global_params.test_library().media_root.clone(),
        });
        let frame = peer_step_with_timeout(
            "controller playback status frame",
            Duration::from_secs(3),
            async { Ok(socket.next().await.context("controller socket closed")??) },
        )
        .await?;
        let (name, payload, _) = decode_message(&frame.into_data()).context("status frame")?;
        assert_eq!(name, "PlaybackStatus");
        let status: PlaybackStatus = rinf::deserialize(&payload)?;
        assert_eq!(status.progress_seconds, 12.0);
        let unexpected_frame =
            tokio::time::timeout(Duration::from_millis(200), socket.next()).await;
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
        tokio::time::timeout(Duration::from_secs(5), async {
            while connection.upgrade().is_some() {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .context("held library did not idle close")?;
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
        let received = tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                if let Some(row) = media_files::Entity::find()
                    .filter(Column::HlcUuid.eq(&original.hlc_uuid))
                    .one(&*replica.main_db)
                    .await?
                {
                    return Ok::<_, anyhow::Error>(row);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .context("unopened L2 did not synchronize")??;
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
        tokio::time::timeout(Duration::from_secs(40), async {
            loop {
                if is_materialized(
                    &*replica.main_db,
                    &replica.fsio,
                    Path::new(&replica.media_root),
                    received.id,
                )
                .await?
                {
                    return Ok::<_, anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .context("unopened L2 did not apply its download rule")??;

        // The restarted worker shares the challenge quota with this direct request.
        tokio::time::sleep(Duration::from_secs(1)).await;
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
                .query(&[("track_id", &original.hlc_uuid)])
                .send()
                .await?
                .status(),
            reqwest::StatusCode::OK
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
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(
            response.json::<serde_json::Value>().await?["Ok"]["namespace"],
            a_ui.to_string()
        );

        set_library_background(&a, library_id, false).await?;
        for response in [
            client.post(format!("{base}/sync/metadata")).send().await?,
            client.get(&blob_url).send().await?,
        ] {
            assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
            assert_eq!(
                response.json::<serde_json::Value>().await?["error"],
                "library_not_served"
            );
        }
        set_library_background(&a, library_id, true).await?;
        set_library_background(&b, library_id, false).await?;
        let connection = Arc::downgrade(&replica.main_db);
        drop(replica);
        tokio::time::sleep(Duration::from_secs(2)).await;
        // Leave the worker running across a complete normal retry interval.
        let later = scan_held_track(&a, library_id, "second.ogg").await?;
        tokio::time::sleep(Duration::from_secs(32)).await;
        let replica = b.global_params.library_pool.acquire(library_id).await?;
        assert!(
            media_files::Entity::find()
                .filter(Column::HlcUuid.eq(&later.hlc_uuid))
                .one(&*replica.main_db)
                .await?
                .is_none(),
            "disabled L2 received a later track"
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while connection.upgrade().is_some() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .context("disabled L2 kept its idle pool connection")?;
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
            Duration::from_secs(120),
            fixture_with_library(first.path(), library_id),
        )
        .await?;
        let b = peer_step_with_timeout(
            "create second fixture",
            Duration::from_secs(120),
            fixture_with_library(second.path(), library_id),
        )
        .await?;
        let a_info = peer_step("read first peer identity", info(&a)).await?;
        let b_info = peer_step("read second peer identity", info(&b)).await?;
        peer_step("pair library peers", pair(&a, &b)).await?;
        peer_step("pair library peers in reverse", pair(&b, &a)).await?;
        let a_reserve = TcpListener::bind("127.0.0.1:0")?;
        let b_reserve = TcpListener::bind("127.0.0.1:0")?;
        let a_addr = a_reserve.local_addr()?;
        let b_addr = SocketAddr::from(([127, 0, 0, 1], b_reserve.local_addr()?.port()));
        assert_ne!(a_addr.port(), b_addr.port());
        drop((a_reserve, b_reserve));
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg"),
            second.path().join("source.ogg"),
        )?;
        peer_step_with_timeout(
            "scan and publish source audio",
            Duration::from_secs(120),
            async {
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
                    id: source_track.hlc_uuid.parse()?,
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
            },
        )
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
                a_addr,
                DiscoveryParams {
                    device_info: a_info.clone(),
                },
            ),
        )
        .await?;
        // Expose the peer before its listener starts so the first metadata pass
        // deterministically encounters a refused connection.
        peer_step(
            "announce peer before second listener starts",
            announce_peer(&a, &b_info, b_addr),
        )
        .await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        peer_step(
            "start second sync listener",
            b.clone().start_sync(
                b_addr,
                DiscoveryParams {
                    device_info: b_info.clone(),
                },
            ),
        )
        .await?;
        peer_step(
            "announce first listener to second peer",
            announce_peer(&b, &a_info, a_addr),
        )
        .await?;
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let seen_a =
                    b.global_params
                        .device_scanner
                        .get_all_devices()
                        .iter()
                        .any(|device| {
                            device.fingerprint == a_info.fingerprint
                                && device.api_port == Some(a_addr.port())
                        });
                let seen_b =
                    a.global_params
                        .device_scanner
                        .get_all_devices()
                        .iter()
                        .any(|device| {
                            device.fingerprint == b_info.fingerprint
                                && device.api_port == Some(b_addr.port())
                        });
                if seen_a && seen_b {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .context("peers did not learn advertised ports")?;
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
        assert_eq!(unauthorized_metadata, reqwest::StatusCode::UNAUTHORIZED);
        let unauthorized_file = peer_step("unauthorized file request", async {
            Ok(pinned
                .get(format!("{base}/files/anything"))
                .send()
                .await?
                .status())
        })
        .await?;
        assert_eq!(unauthorized_file, reqwest::StatusCode::UNAUTHORIZED);
        let metadata_wait_started = Instant::now();
        let remote_id = tokio::time::timeout(Duration::from_secs(45), async {
            let receiver = a.global_params.test_library().library_sync.clone();
            loop {
                if receiver.get(&key).await.unwrap().is_some()
                    && b.global_params
                        .test_library()
                        .library_sync
                        .get(&reverse_key)
                        .await
                        .unwrap()
                        .is_some()
                    && let Some(track) = media_files::Entity::find()
                        .filter(Column::LocalAvailable.eq(false))
                        .one(&*a.global_params.test_library().main_db)
                        .await
                        .unwrap()
                {
                    break track.id;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .context("background sync did not transfer album and track metadata")?;
        anyhow::ensure!(
            metadata_wait_started.elapsed() < Duration::from_secs(20),
            "metadata transfer after a refused first pass took longer than 20 seconds"
        );
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
                .add_trusted_domains(["127.0.0.1", "localhost"], &b_info.fingerprint)
                .await?;
            Ok(())
        })
        .await?;
        // Let the background workers finish their shared per-IP auth burst.
        tokio::time::sleep(Duration::from_secs(1)).await;
        let audio_library = peer_step(
            "acquire audio library",
            a.global_params
                .library_pool
                .acquire(a.global_params.library_id.context("library")?),
        )
        .await?;
        let source = peer_step("resolve remote audio", async {
            Ok(loop {
                if let Ok(source) = a
                    .resolve_remote_audio(
                        &audio_library,
                        a.global_params.main_token.child_token(),
                        remote_id,
                    )
                    .await
                {
                    break source;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            })
        })
        .await
        .context("could not resolve remote audio")?;
        let expected = std::fs::read(second.path().join("source.ogg"))?;
        let source_path = source
            .url
            .strip_prefix(&format!("https://127.0.0.1:{}", b_addr.port()))
            .context("remote audio URL did not use the announced loopback address")?;
        let full_bytes = peer_step("full audio GET", async {
            let full = source.request.get(None).await?;
            anyhow::ensure!(
                full.status() == reqwest::StatusCode::OK,
                "full audio GET status"
            );
            Ok(full.bytes().await?)
        })
        .await?;
        assert_eq!(full_bytes.as_ref(), expected.as_slice());
        // Expire the server-side token, then exercise a real ranged GET with renewal.
        let expired_token = peer_step("expire audio token", async {
            Ok(b.sync_auth
                .lock()
                .await
                .as_ref()
                .unwrap()
                .expire_tokens_for_fingerprint_for_test(&a_info.fingerprint)
                .await)
        })
        .await?;
        anyhow::ensure!(expired_token, "audio renewal test found no token to expire");
        tokio::time::sleep(Duration::from_secs(1)).await;
        peer_step("inject transient renewal rejection", async {
            b.sync_auth
                .lock()
                .await
                .as_ref()
                .unwrap()
                .reject_next_authorization_for_test(&a_info.fingerprint)
                .await;
            Ok(())
        })
        .await?;
        // A's background sync may have fetched a fresh token while B's were
        // expired above; drop it so the stream itself has to renew.
        sync_worker::invalidate_sync_token(
            &a.global_params.node,
            &b_info.fingerprint,
            "127.0.0.2",
            b_addr.port(),
        );
        let renewed_ranges = tokio::join!(
            peer_step("first concurrent renewed ranged audio GET", async {
                let response = source.request.get(Some((0, Some(15)))).await?;
                anyhow::ensure!(
                    response.status() == reqwest::StatusCode::PARTIAL_CONTENT,
                    "ranged audio GET status"
                );
                Ok(response.bytes().await?)
            }),
            peer_step("second concurrent renewed ranged audio GET", async {
                let response = source.request.get(Some((16, Some(31)))).await?;
                anyhow::ensure!(
                    response.status() == reqwest::StatusCode::PARTIAL_CONTENT,
                    "ranged audio GET status"
                );
                Ok(response.bytes().await?)
            }),
        );
        let (first_range, second_range) = renewed_ranges;
        assert_eq!(first_range?.as_ref(), &expected[..16]);
        assert_eq!(second_range?.as_ref(), &expected[16..32]);
        anyhow::ensure!(
            !b.sync_auth
                .lock()
                .await
                .as_ref()
                .unwrap()
                .authorization_rejection_pending_for_test(&a_info.fingerprint)
                .await,
            "stream renewal did not consume its targeted transient rejection"
        );
        anyhow::ensure!(
            !source.cancellation.is_cancelled(),
            "concurrent token renewal revoked an authorized stream"
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
            tokio::task::spawn_blocking(move || -> Result<bool> {
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
        tokio::time::timeout(Duration::from_secs(10), stream_cancel.cancelled())
            .await
            .context("revocation did not cancel the remote stream")?;
        peer_step("stop paired workers", async {
            a.stop_worker().await?;
            b.stop_worker().await
        })
        .await?;
        tokio::time::sleep(Duration::from_secs(1)).await;
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
                        .header(reqwest::header::RANGE, "bytes=-12")
                },
            )
            .await?;
            anyhow::ensure!(
                suffix.status() == reqwest::StatusCode::PARTIAL_CONTENT,
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
                        .header(reqwest::header::RANGE, format!("bytes={}-", expected.len()))
                },
            )
            .await
        })
        .await?;
        assert_eq!(invalid.status(), reqwest::StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            invalid
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .unwrap(),
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
        tokio::time::sleep(Duration::from_secs(1)).await;
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
            ("/files/anything", reqwest::StatusCode::BAD_REQUEST),
            ("/ws", reqwest::StatusCode::NOT_FOUND),
            ("/panel/users", reqwest::StatusCode::NOT_FOUND),
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
        tokio::time::sleep(Duration::from_secs(1)).await;
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
            assert_eq!(
                anonymous_status,
                reqwest::StatusCode::UNAUTHORIZED,
                "{path}"
            );
            let authorized_status = peer_step("authorized full-listener request", async {
                Ok(request(&full_authorized).send().await?.status())
            })
            .await?;
            assert_ne!(
                authorized_status,
                reqwest::StatusCode::UNAUTHORIZED,
                "{path}"
            );
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
        assert_eq!(anonymous_ws_status, reqwest::StatusCode::UNAUTHORIZED);
        let full = peer_step("panel login route", async {
            Ok(pinned
                .post(format!("{base}/panel/auth/login"))
                .send()
                .await?
                .status())
        })
        .await?;
        assert_ne!(full, reqwest::StatusCode::NOT_FOUND);
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
        peer_step("wait for restarted listener readiness", async {
            loop {
                if let Ok(response) = restarted_client
                    .get(format!("{base}/ping"))
                    .header(CONNECTION, "close")
                    .send()
                    .await
                    && response.status().is_success()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Ok(())
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
        assert_eq!(restarted_status, reqwest::StatusCode::UNAUTHORIZED);
        b.global_params.node.token.cancel();
        let rebound = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(listener) = TcpListener::bind(b_addr) {
                    break listener;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .context("main cancellation did not release listener")?;
        drop(rebound);
        peer_step("stop cancelled second listener", b.stop()).await?;
        peer_step("stop restarted first listener", a.stop()).await?;
        a.global_params.node.token.cancel();
        let rebound = TcpListener::bind(a_addr)?;
        drop(rebound);
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
        std::fs::copy(
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
        let track_id: Uuid = file.hlc_uuid.parse()?;
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
            matches!(&queued[..], [(PlayingItem::InLibrary(item), PlaybackSource::LibraryStream)] if item.file_uuid.to_string() == file.hlc_uuid)
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
        std::fs::copy(fixture_dir.join("startup_1.ogg"), &updated_path)?;
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
        let file_uuid = Uuid::parse_str(&file.hlc_uuid)?;
        assert_eq!(resolver.resolve_id(file_uuid).await?, Some(current.id));
        params.main_token.cancel();
        Ok(())
    }
}
