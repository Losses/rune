use super::sync::http as super_sync_http;
use super::sync::http::BlobContext;
use super::sync::http::SharedLibrary;
use super::sync::http::blob_router;
use super::sync_auth::SyncAuth;
use crate::backends::remote::RemoteConnectError;
use database::actions::file::catalog_track;
use database::sync::model::RuneModel;
use std::net::TcpListener;
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
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
use ::fsio::FsIo;

use crate::{
    Signal,
    messages::*,
    server::{
        AppState, ServerState, WebSocketService,
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
    utils::{GlobalParams, ParamsExtractor, RinfRustSignal},
};

async fn register_request_handlers(
    websocket_service: Arc<WebSocketService>,
    global_params: Arc<GlobalParams>,
) {
    for_all_remote_request_pairs2!(listen_server_event, websocket_service, global_params);
}

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
    pub global_params: Arc<GlobalParams>,
    server_handle: Mutex<Option<JoinHandle<()>>>,
    start_lock: Mutex<()>,
    addr: Mutex<Option<SocketAddr>>,
    is_running: AtomicBool,
    sync_only: AtomicBool,
    shutdown_handle: Mutex<Option<Handle>>,
    sync_cancel: Mutex<Option<tokio_util::sync::CancellationToken>>,
    sync_handle: Mutex<Option<JoinHandle<()>>>,
    stream_handles: Mutex<Vec<JoinHandle<()>>>,
    #[cfg(test)]
    sync_auth_for_test: Mutex<Option<super::sync_auth::SyncAuth>>,
    watch_cancel: Mutex<Option<tokio_util::sync::CancellationToken>>,
    watch_handle: Mutex<Option<JoinHandle<()>>>,
    certificate: String,
    private_key: String,
    pub jwt_secret: Vec<u8>,
    pub fsio: Arc<FsIo>,
}

impl ServerManager {
    /// Fetch the owned libraries advertised by a trusted paired device.
    pub async fn list_peer_libraries(
        &self,
        fingerprint: &str,
    ) -> Result<Vec<PeerLibrary>, RemoteConnectError> {
        super::libraries::list_peer_libraries(&self.global_params, &self.private_key, fingerprint)
            .await
    }

    pub async fn new(global_params: Arc<GlobalParams>) -> Result<Self> {
        let config_path = Path::new(&*global_params.config_path);

        let certificate_id = get_or_generate_alias(config_path).await?;

        let (_, certificate, private_key) =
            generate_or_load_certificates(config_path, &certificate_id)
                .await
                .context("Failed to initialize certificates")?;

        let jwt_secret = get_or_generate_jwt_secret(config_path)
            .await
            .context("Failed to initialize JWT secret")?;

        #[cfg(not(target_os = "android"))]
        let fsio = Arc::new(FsIo::new());
        #[cfg(target_os = "android")]
        let fsio = Arc::new(FsIo::new(
            Path::new(".rune/.android-fs.db"),
            &global_params.lib_path,
        )?);

        Ok(Self {
            global_params,
            server_handle: Mutex::new(None),
            start_lock: Mutex::new(()),
            addr: Mutex::new(None),
            is_running: AtomicBool::new(false),
            sync_only: AtomicBool::new(false),
            shutdown_handle: Mutex::new(None),
            sync_cancel: Mutex::new(None),
            sync_handle: Mutex::new(None),
            stream_handles: Mutex::new(Vec::new()),
            #[cfg(test)]
            sync_auth_for_test: Mutex::new(None),
            watch_cancel: Mutex::new(None),
            watch_handle: Mutex::new(None),
            certificate,
            private_key,
            jwt_secret,
            fsio,
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
        let _start_guard = self.start_lock.lock().await;
        anyhow::ensure!(
            !self.global_params.main_token.is_cancelled(),
            "library is shutting down"
        );
        self.start_worker().await?;
        if self.is_running.load(Ordering::SeqCst) {
            return Err(anyhow::anyhow!("Server already running"));
        }
        self.global_params
            .player
            .lock()
            .await
            .set_source_resolver(Arc::new(super::sync_worker::PairedAudioResolver(
                Arc::downgrade(&self),
            )));
        let mut device_info = discovery_params.device_info.clone();
        let websocket_service = Arc::new(WebSocketService::new());

        register_request_handlers(websocket_service.clone(), self.global_params.clone()).await;

        let app_state = Arc::new(AppState {
            lib_path: PathBuf::from(&*self.global_params.lib_path),
            cover_temp_dir: COVER_TEMP_DIR.clone(),
        });

        let watch_cancel = tokio_util_sync::CancellationToken::new();
        let server_state = Arc::new(ServerState {
            session_cancel: watch_cancel.clone(),
            app_state: app_state.clone(),
            websocket_service: websocket_service.clone(),
            discovery_device_info: Arc::new(RwLock::new(discovery_params.device_info)),
            permission_manager: self.global_params.permission_manager.clone(),
            device_scanner: self.global_params.device_scanner.clone(),
            fsio: Arc::clone(&self.fsio),
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

        let node_id = uuid::Uuid::parse_str(&self.global_params.node_id)?;
        let library = Arc::new(RwLock::new(
            self.global_params
                .library_sync
                .clone()
                .ok_or_else(|| anyhow::anyhow!("local metadata replica is unavailable"))?,
        ));
        let (_, fingerprint) = parse_certificate(&self.certificate)?;
        let auth = SyncAuth::new(
            self.global_params.permission_manager.clone(),
            fingerprint.clone(),
        );
        #[cfg(test)]
        {
            *self.sync_auth_for_test.lock().await = Some(auth.clone());
        }
        let signal_path = self.global_params.lib_path.to_string();
        let broadcaster = self.global_params.broadcaster.clone();
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            broadcaster.broadcast(&crate::messages::LibraryContentChanged {
                path: signal_path.clone(),
            });
        });
        let sync_routes =
            super::sync::library_router(SharedLibrary(library.clone()), notify.clone())
                .merge(blob_router(BlobContext {
                    store: library.clone(),
                    db: (*self.global_params.main_db).clone(),
                    fsio: self.global_params.fsio.clone(),
                    root: PathBuf::from(&*self.global_params.lib_path),
                    node_id,
                    upload_lock: Arc::new(Mutex::new(())),
                    notify: notify.clone(),
                }))
                .layer(middleware::from_fn_with_state(
                    library.read().await.identity().await?.namespace,
                    super::sync::require_library,
                ));
        let sync_routes = Router::new()
            .nest("/libraries/{library_id}", sync_routes)
            .route("/libraries", get(super::libraries::list_libraries))
            .layer(middleware::from_fn_with_state(
                auth.clone(),
                super::sync_auth::require_auth,
            ));
        let app = Router::new()
            .merge(register_route)
            .route("/ping", get(ping_handler))
            .route("/check-fingerprint", get(check_fingerprint_handler))
            .route("/device-info", get(device_info_handler));
        let app = if sync_only {
            app
        } else {
            let device_routes = Router::new()
                .route("/ws", get(websocket_handler))
                .route("/files/{*file_path}", get(file_handler))
                .route("/media/metadata/{id}", get(get_media_metadata_handler))
                .route("/media/cover/{id}", get(get_cover_art_handler))
                .layer(middleware::from_fn_with_state(
                    auth.clone(),
                    super::sync_auth::require_auth,
                ));
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
            "Library files path: {}",
            app_state.lib_path.to_string_lossy()
        );
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
        let lifetime_token = self.global_params.main_token.clone();
        let lifetime_scanner = self.global_params.device_scanner.clone();
        *self.watch_handle.lock().await = Some(tokio::spawn(async move {
            tokio::select! {
                _ = watch_stop.cancelled() => {},
                _ = lifetime_token.cancelled() => {
                    session_stop.cancel();
                    lifetime_handle.shutdown();
                    lifetime_scanner.stop_announcements().await;
                    lifetime_scanner.stop_listening().await;
                }
            }
        }));
        *self.watch_cancel.lock().await = Some(watch_cancel);
        if !self.global_params.device_scanner.is_listening().await
            && let Err(error) = self
                .global_params
                .device_scanner
                .start_listening(Some(fingerprint.clone()))
                .await
        {
            log::warn!("Could not start paired-device discovery: {error}");
        }
        if !self.global_params.device_scanner.is_announcing().await
            && let Err(error) = self
                .global_params
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
        if self.global_params.main_token.is_cancelled() {
            self.stop().await?;
            anyhow::bail!("library is shutting down");
        }
        Ok(())
    }

    pub async fn start_worker(self: &Arc<Self>) -> Result<()> {
        let mut handle = self.sync_handle.lock().await;
        if handle.is_some() {
            return Ok(());
        }
        let library = self
            .global_params
            .library_sync
            .clone()
            .ok_or_else(|| anyhow::anyhow!("local metadata replica is unavailable"))?;
        let (_, fingerprint) = parse_certificate(&self.certificate)?;
        self.global_params
            .player
            .lock()
            .await
            .set_source_resolver(Arc::new(super::sync_worker::PairedAudioResolver(
                Arc::downgrade(self),
            )));
        let path = self.global_params.lib_path.to_string();
        let broadcaster = self.global_params.broadcaster.clone();
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            broadcaster.broadcast(&crate::messages::LibraryContentChanged { path: path.clone() })
        });
        let cancel = self.global_params.main_token.child_token();
        let worker = super::sync_worker::run(
            self.global_params.clone(),
            super_sync_http::SharedLibrary(Arc::new(RwLock::new(library))),
            coalesce_worker_notify(notify, cancel.clone()),
            fingerprint,
            self.private_key.clone(),
            cancel.clone(),
        );
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
            if let Err(error) = handle.await {
                first_error.get_or_insert_with(|| error.into());
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub async fn stop(&self) -> Result<()> {
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
            handle.shutdown();
        }

        if let Some(handle) = self.server_handle.lock().await.take()
            && let Err(error) = handle.await
        {
            log::error!("Server task failed during shutdown: {error}");
            first_error.get_or_insert_with(|| error.into());
        }

        self.global_params.device_scanner.stop_announcements().await;
        self.global_params.device_scanner.stop_listening().await;
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
        id: i32,
    ) -> Result<playback::player::RemoteAudioSource> {
        let params = &self.global_params;
        let library = params
            .library_sync
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("local metadata replica is unavailable"))?;
        let (track_id, blob, extension) = catalog_track(&params.main_db, id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("track is not available for remote streaming"))?;
        let library_id = library.identity().await?.namespace;
        let (_, own) = parse_certificate(&self.certificate)?;
        let entries = super::sync_worker::peer_endpoints(params, &own).await;
        for hosts in entries {
            for endpoint in hosts {
                let fingerprint = endpoint.fingerprint;
                let host = endpoint.host;
                let trusted_host = endpoint.trusted_host;
                let port = endpoint.port;
                let result = async {
                    let (base, mut client) = super::sync_worker::authenticated_client(
                        params,
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
                    let mut response = client.head(&url).query(&[("track_id", track_id.to_string())]).send().await?;
                    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                        let (_, renewed) = super::sync_worker::authenticated_client(params, &own, &self.private_key, &fingerprint, &host, port).await?;
                        client = renewed;
                        response = client.head(&url).query(&[("track_id", track_id.to_string())]).send().await?;
                    }
                    response.error_for_status()?;
                    let mut stream_handles = self.stream_handles.lock().await;
                    let cancellation = self.sync_cancel.lock().await.as_ref()
                        .ok_or_else(|| anyhow::anyhow!("library synchronization is closing"))?.child_token();
                    stream_handles.retain(|handle| !handle.is_finished());
                    let request = Arc::new(super::sync_worker::RenewableAudio {
                        global: params.clone(), own: own.clone(), private_key: self.private_key.clone(),
                        endpoint: super::sync_worker::PeerEndpoint { fingerprint: fingerprint.clone(), trusted_host: trusted_host.clone(), host: host.clone(), port },
                        url: url.clone(), size: blob.size,
                        client: Arc::new(Mutex::new(super::sync_worker::AudioCredentials { client, generation: 0 })), cancel: cancellation.clone(),
                    });
                    let watch_cancel = cancellation.clone();
                    let watch_params = params.clone();
                    let watch_fingerprint = fingerprint.clone();
                    let revocation_host = trusted_host.clone();
                    let watch_request = request.clone();
                    let probe_url = format!("{base}/sync/head");
                    let watcher = tokio::spawn(async move {
                        let remote_probe = async {
                            loop {
                                tokio::time::sleep(Duration::from_secs(5)).await;
                                let client = watch_request.client.lock().await.client.clone();
                                match client.get(&probe_url).send().await {
                                    Ok(response) if response.status() == reqwest::StatusCode::UNAUTHORIZED => {
                                        if let Err(error) = watch_request.refresh().await {
                                            log::debug!("Audio authorization refresh failed: {error:#}");
                                            if watch_cancel.is_cancelled() { break; }
                                        }
                                    },
                                    _ => {},
                                }
                            }
                        };
                        tokio::select! {
                            _ = watch_cancel.cancelled() => {},
                            _ = super::sync_worker::wait_for_revocation(&watch_params, &watch_fingerprint, &revocation_host) => watch_cancel.cancel(),
                            _ = remote_probe => {},
                        }
                    });
                    stream_handles.push(watcher);
                    Ok::<_, anyhow::Error>(playback::player::RemoteAudioSource {
                        request, url, extension: extension.clone(), cancellation,
                    })
                }
                .await;
                if let Ok(source) = result {
                    return Ok(source);
                }
            }
        }
        anyhow::bail!("no paired device currently has this audio")
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
    use super::super::sync_worker::{PeerEndpoint, PeerPass, authenticated_client, test_peer_pass};
    use super::super::utils::device::load_device_info;
    use database::connection::RecommendationDbConnection;
    use migration::catalog_identity::ParentKind;
    use playback::decoder::RuneDecoder;
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

    use super::*;
    use crate::utils::{
        Broadcaster, GlobalParams, RinfRustSignal, RunningMode, TaskTokens,
        sync_settings::SyncSettings,
    };
    use crate::utils::{initialize_databases, nid::get_or_create_node_id};
    use database::actions::downloads::prepare_download;
    use database::actions::file::{attach_download, catalog_track, hash_file, is_materialized};
    use database::actions::metadata::{
        MissingHostFilePolicy, ScanOptions, scan_audio_file, scan_audio_library,
    };
    use database::connection::{MainDbConnection, StorageMode};
    use database::entities::{media_files, media_files::Column};
    use database::library_registry::{
        DbPlacement, LibraryAuthority, LibraryDescriptor, LibraryRegistry, LibraryStorage,
        create_replica, resolve_library,
    };
    use database::sync::{
        LibrarySyncStore,
        model::{Album, Collection, MetadataTag, ObjectKey, Payload, Track},
    };
    use discovery::{
        client::CertValidator,
        protocol::DiscoveryService,
        server::{PermissionManager, UserStatus},
        utils::DeviceType,
    };
    use migration::{Migrator, MigratorTrait};
    use playback::{player::MockPlayer, sfx_player::SfxPlayer};
    use scrobbling::manager::MockScrobblingManager;
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
    use std::sync::OnceLock;
    use tokio::net::TcpStream;
    use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    use crate::library_pool::{LibraryPool, LibraryResources};

    struct Quiet;
    impl Broadcaster for Quiet {
        fn broadcast(&self, _: &dyn RinfRustSignal) {}
    }

    async fn fixture(dir: &Path) -> Result<Arc<ServerManager>> {
        fixture_with_library(dir, Uuid::new_v4()).await
    }

    async fn fixture_with_library(dir: &Path, library_id: Uuid) -> Result<Arc<ServerManager>> {
        let mut options = sea_orm::ConnectOptions::new(format!(
            "sqlite://{}?mode=rwc",
            dir.join("catalog.sqlite").display(),
        ));
        options.max_connections(1).min_connections(1);
        let db = sea_orm::Database::connect(options).await?;
        let node = get_or_create_node_id(dir.to_str().context("config path")?).await?;
        let _ = migration::initialize_node_id(node.to_string());
        Migrator::up(&db, None).await?;
        LibraryRegistry::update(dir, |registry| {
            registry.libraries.push(LibraryDescriptor {
                library_id,
                alias: "Owned".into(),
                authority: LibraryAuthority::Owned,
                storage: LibraryStorage::Local {
                    path: dir.to_string_lossy().into_owned(),
                },
                db_placement: DbPlacement::Portable,
                background_sync: true,
            });
            Ok(())
        })
        .await?;
        fixture_with_db(dir, dir, library_id, node, db).await
    }

    async fn fixture_open_replica(config: &Path, root: &Path) -> Result<Arc<ServerManager>> {
        let fsio = FsIo::new();
        let mut registry = LibraryRegistry::load(config)?;
        let descriptor = resolve_library(
            &fsio,
            &mut registry,
            root.to_str().context("replica path")?,
            "",
            StorageMode::Portable,
        )
        .await?;
        let node = get_or_create_node_id(config.to_str().context("config path")?).await?;
        let connections = initialize_databases(
            &fsio,
            root.to_str().context("replica path")?,
            None,
            &node.to_string(),
        )
        .await?;
        fixture_with_db(
            root,
            config,
            descriptor.library_id,
            node,
            (*connections.main_db).clone(),
        )
        .await
    }

    async fn fixture_with_db(
        dir: &Path,
        config: &Path,
        library_id: Uuid,
        node: Uuid,
        db: MainDbConnection,
    ) -> Result<Arc<ServerManager>> {
        let fsio = Arc::new(FsIo::new());
        let token = Arc::new(CancellationToken::new());
        let library_sync =
            crate::utils::initialize_library_replica(&db, &node.to_string(), library_id).await?;
        let pool = LibraryPool::new(config, config, &node.to_string())?;
        let main_db = Arc::new(db);
        let recommend_db = Arc::new(RecommendationDbConnection::new());
        let descriptor = LibraryRegistry::load(config)?
            .libraries
            .into_iter()
            .find(|library| library.library_id == library_id)
            .context("fixture descriptor")?;
        let library = pool
            .register_ui(LibraryResources {
                main_db: main_db.clone(),
                library_sync: library_sync.clone(),
                recommend_db: recommend_db.clone(),
                fsio: fsio.clone(),
                media_root: dir.to_string_lossy().into_owned(),
                descriptor,
            })
            .await?;
        let params = Arc::new(GlobalParams {
            library_id: Some(library_id),
            library_pool: pool,
            ui_library: Arc::new(Mutex::new(Some(library))),
            sync_settings: Arc::new(RwLock::new(SyncSettings::default())),
            fsio,
            lib_path: Arc::new(dir.to_string_lossy().into_owned()),
            config_path: Arc::new(config.to_string_lossy().into_owned()),
            node_id: Arc::new(node.to_string()),
            main_db,
            library_sync: Some(library_sync),
            recommend_db,
            main_token: token.clone(),
            task_tokens: Arc::new(Mutex::new(TaskTokens::default())),
            player: Arc::new(Mutex::new(MockPlayer)),
            sfx_player: Arc::new(Mutex::new(SfxPlayer::new(Some((*token).clone())))),
            scrobbler: Arc::new(Mutex::new(MockScrobblingManager::new())),
            broadcaster: Arc::new(Quiet),
            device_scanner: Arc::new(DiscoveryService::without_store()),
            cert_validator: Arc::new(RwLock::new(CertValidator::new(config).await?)),
            permission_manager: Arc::new(RwLock::new(PermissionManager::new(config)?)),
            server_manager: OnceLock::new(),
            running_mode: RunningMode::Client,
        });
        let manager = Arc::new(ServerManager::new(params.clone()).await?);
        params
            .server_manager
            .set(manager.clone())
            .expect("manager set once");
        Ok(manager)
    }
    #[tokio::test]
    async fn request_gate_server_dispatch_rejects_local_requests() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let manager = fixture(dir.path()).await?;
        let service = Arc::new(WebSocketService::new());
        register_request_handlers(service.clone(), manager.global_params.clone()).await;
        assert!(
            service
                .handle_message("FetchRemoteFileRequest", Vec::new(), None)
                .await
                .is_none()
        );
        let payload = rinf::serialize(&GetMediaFilesCountRequest {})?;
        let (name, response) = service
            .handle_message("GetMediaFilesCountRequest", payload, None)
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
        assert!(Arc::ptr_eq(&library.main_db, &params.main_db));
        assert!(params.ui_library.lock().await.is_some());
        let close = CloseLibraryRequest {
            path: (*params.lib_path).clone(),
        };
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

    #[test]
    fn request_gate_owner_requests_are_not_forwardable() {
        macro_rules! collect_names {
            ($names:expr, $unused:expr $(, ($request:ident, $response:ident))* $(, $request_only:ident)*) => {{
                $( $names.push(stringify!($request)); )*
                $( $names.push(stringify!($request_only)); )*
            }};
        }
        let mut names = Vec::new();
        for_all_remote_request_pairs2!(collect_names, names, ());
        for name in [
            "ListLocalLibrariesRequest",
            "SetLibraryBackgroundSyncRequest",
            "ListPeerLibrariesRequest",
            "AddRemoteLibraryRequest",
            "UpdateClientStatusRequest",
            "AuthenticateSingleServiceRequest",
            "AuthenticateMultipleServiceRequest",
            "LogoutSingleServiceRequest",
        ] {
            assert!(!names.contains(&name), "{name} must remain local");
        }
    }

    async fn info(manager: &ServerManager) -> Result<discovery::utils::DeviceInfo> {
        load_device_info(Path::new(&*manager.global_params.config_path)).await
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
        manager.global_params.main_token.cancel();
        Ok(())
    }
    async fn pair(a: &ServerManager, b: &ServerManager) -> Result<()> {
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

    async fn paired_full(a: &Arc<ServerManager>, b: &Arc<ServerManager>) -> Result<SocketAddr> {
        pair(a, b).await?;
        a.clone()
            .start(
                ([127, 0, 0, 1], 0).into(),
                DiscoveryParams {
                    device_info: info(a).await?,
                },
            )
            .await?;
        a.get_address().await.context("missing listener address")
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
            connect_remote("127.0.0.1", 1, config, validator.clone()).await,
            Err(RemoteConnectError::UntrustedEndpoint)
        ));
        let addr = paired_full(&a, &b).await?;
        let validator = b.global_params.cert_validator.read().await.clone();
        let fingerprint = info(&b).await?.fingerprint;
        for remove in [false, true] {
            let mut socket =
                connect_remote("127.0.0.1", addr.port(), config, validator.clone()).await?;
            let manager = a.global_params.permission_manager.read().await;
            if remove {
                manager.remove_user(&fingerprint).await?;
            } else {
                manager
                    .change_user_status(&fingerprint, UserStatus::Blocked)
                    .await?;
            }
            drop(manager);
            let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
                .await?
                .context("socket closed without frame")??;
            assert!(
                matches!(frame, Message::Close(Some(frame)) if u16::from(frame.code) == 4403 && frame.reason == "revoked")
            );
            if !remove {
                assert!(matches!(
                    connect_remote("127.0.0.1", addr.port(), config, validator.clone()).await,
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
        let mut socket =
            connect_remote("127.0.0.1", addr.port(), config, validator.clone()).await?;
        a.stop().await?;
        let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await?
            .context("socket closed without frame")??;
        assert!(
            matches!(frame, Message::Close(Some(frame)) if u16::from(frame.code) == 1001 && frame.reason == "server stopping")
        );
        assert!(matches!(
            connect_remote("127.0.0.1", addr.port(), config, validator).await,
            Err(RemoteConnectError::Unreachable)
        ));
        a.global_params.main_token.cancel();
        b.global_params.main_token.cancel();
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
                    assert!(failure.detail.contains("Playlist not found with id: -1"));
                } else {
                    assert!(failure.detail.contains("Failed to deserialize request"));
                }
            }
        }
        socket.close(None).await?;
        a.stop().await?;
        a.global_params.main_token.cancel();
        b.global_params.main_token.cancel();
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
        let mut socket =
            connect_remote("127.0.0.1", addr.port(), config, validator.clone()).await?;
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
        let response =
            authorized_get(config, validator.clone(), "127.0.0.1", addr.port(), url).await?;
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
            connect_remote("127.0.0.1", addr.port(), config, validator)
                .await
                .is_err()
        );
        socket.close(None).await?;
        a.stop().await?;
        a.global_params.main_token.cancel();
        b.global_params.main_token.cancel();
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
            &b.global_params,
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
        a.global_params.main_token.cancel();
        b.global_params.main_token.cancel();
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
                .library_sync
                .as_ref()
                .unwrap()
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
                    .count(&*manager.global_params.main_db)
                    .await?,
                1
            );
            assert!(
                manager
                    .global_params
                    .library_sync
                    .as_ref()
                    .unwrap()
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
        a.global_params.main_token.cancel();
        b.global_params.main_token.cancel();
        Ok(())
    }

    async fn request_peer_libraries(
        manager: &ServerManager,
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
        manager: &ServerManager,
        fingerprint: &str,
        library: &PeerLibrary,
    ) -> Result<String> {
        let request = AddRemoteLibraryRequest {
            fingerprint: fingerprint.to_owned(),
            library_id: library.library_id.clone(),
            alias: library.alias.clone(),
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
        Ok(response.path)
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
        let source = first.path().join("source.ogg");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg"),
            &source,
        )?;
        scan_audio_file(
            &a.global_params.fsio,
            &a.global_params.main_db,
            &a.global_params.node_id,
            first.path(),
            &source,
            &a.global_params
                .library_sync
                .as_ref()
                .unwrap()
                .record_store(),
        )
        .await?;
        let original = media_files::Entity::find()
            .one(&*a.global_params.main_db)
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
        let path = request_add_library(&b, &fingerprint, &libraries[0]).await?;
        assert_eq!(
            path,
            request_add_library(&b, &fingerprint, &libraries[0]).await?
        );
        let replica = fixture_open_replica(second.path(), Path::new(&path)).await?;
        assert_eq!(replica.global_params.library_id, Some(library_id));
        let endpoint = PeerEndpoint {
            fingerprint: fingerprint.clone(),
            trusted_host: format!("127.0.0.1:{}", addr.port()),
            host: "127.0.0.1".into(),
            port: addr.port(),
        };
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if test_peer_pass(
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
            .one(&*replica.global_params.main_db)
            .await?
            .context("replicated track")?;
        assert!(!remote.local_available);
        scan_audio_library(
            &replica.global_params.fsio,
            &replica.global_params.main_db,
            &replica.global_params.node_id,
            Path::new(&path),
            ScanOptions {
                cleanup: true,
                force: false,
                missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
                cancel_token: None,
            },
            |_| {},
            &replica
                .global_params
                .library_sync
                .as_ref()
                .unwrap()
                .record_store(),
        )
        .await?;
        assert_eq!(
            media_files::Entity::find_by_id(remote.id)
                .one(&*replica.global_params.main_db)
                .await?,
            Some(remote)
        );
        let listed = request_peer_libraries(&replica, &fingerprint).await?;
        assert_eq!(listed.error_kind, None);
        let libraries = listed.libraries;
        assert_eq!(libraries.len(), 1);
        assert!(libraries[0].added);
        assert_eq!(
            path,
            request_add_library(&replica, &fingerprint, &libraries[0]).await?
        );
        let registry = LibraryRegistry::load(second.path())?;
        assert_eq!(registry.libraries.len(), 2);
        assert_eq!(
            registry
                .libraries
                .iter()
                .find(|entry| entry.library_id == library_id)
                .unwrap()
                .authority,
            LibraryAuthority::Hosted {
                fingerprint: fingerprint.clone()
            }
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
            manager.global_params.main_token.cancel();
        }
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
        let a = fixture_with_library(first.path(), library_id).await?;
        let b = fixture_with_library(second.path(), library_id).await?;
        let a_info = info(&a).await?;
        let b_info = info(&b).await?;
        pair(&a, &b).await?;
        let a_reserve = TcpListener::bind("127.0.0.1:0")?;
        let b_reserve = TcpListener::bind("127.0.0.1:0")?;
        let a_addr = a_reserve.local_addr()?;
        let b_addr = SocketAddr::from(([127, 0, 0, 2], b_reserve.local_addr()?.port()));
        assert_ne!(a_addr.port(), b_addr.port());
        drop((a_reserve, b_reserve));
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg"),
            second.path().join("source.ogg"),
        )?;
        scan_audio_library(
            &b.global_params.fsio,
            &b.global_params.main_db,
            &b.global_params.node_id,
            second.path(),
            ScanOptions {
                cleanup: false,
                force: false,
                missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
                cancel_token: None,
            },
            |_| {},
            &b.global_params
                .library_sync
                .as_ref()
                .unwrap()
                .record_store(),
        )
        .await?;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        let source_track = media_files::Entity::find()
            .filter(Column::LocalAvailable.eq(true))
            .one(&*b.global_params.main_db)
            .await?
            .context("fixture scanner did not create the source audio row")?;
        let track_key = ObjectKey {
            collection: Collection::Tracks,
            id: source_track.hlc_uuid.parse()?,
        };
        anyhow::ensure!(
            b.global_params
                .library_sync
                .as_ref()
                .unwrap()
                .get(&track_key)
                .await?
                .is_some(),
            "fixture scanner did not publish source track metadata"
        );
        let key = ObjectKey {
            collection: Collection::Albums,
            id: migration::catalog_identity::parent(ParentKind::Album, "background album"),
        };
        let reverse_key = ObjectKey {
            collection: Collection::Albums,
            id: migration::catalog_identity::parent(ParentKind::Album, "first node album"),
        };
        a.global_params
            .library_sync
            .as_ref()
            .unwrap()
            .write_local(reverse_key.clone(), Some(album_payload("first node album")))
            .await?;
        let store = LibrarySyncStore::open(
            (*b.global_params.main_db).clone(),
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
        a.clone()
            .start_sync(
                a_addr,
                DiscoveryParams {
                    device_info: a_info.clone(),
                },
            )
            .await?;
        b.clone()
            .start_sync(
                b_addr,
                DiscoveryParams {
                    device_info: b_info.clone(),
                },
            )
            .await?;
        // Deliver actual discovery datagrams deterministically when multicast is unavailable.
        for (receiver, info, addr) in [(&a, &b_info, b_addr), (&b, &a_info, a_addr)] {
            let announcement = serde_json::json!({
                "fingerprint": info.fingerprint, "alias": info.alias,
                "deviceModel": info.device_model, "deviceType": info.device_type,
                "api_port": addr.port(), "announce": true,
            });
            receiver
                .global_params
                .device_scanner
                .observe_announcement(&serde_json::to_vec(&announcement)?, addr, None)
                .await?;
        }
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
        let pinned = reqwest::Client::builder()
            .use_preconfigured_tls(
                Arc::new(b.global_params.cert_validator.read().await.clone()).into_client_config(),
            )
            .no_proxy()
            .build()?;
        let base = format!("https://{a_addr}");
        assert_eq!(
            pinned
                .post(format!("{base}/libraries/{library_id}/sync/metadata"))
                .send()
                .await?
                .status(),
            reqwest::StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            pinned
                .get(format!("{base}/files/anything"))
                .send()
                .await?
                .status(),
            reqwest::StatusCode::UNAUTHORIZED
        );
        let remote_id = tokio::time::timeout(Duration::from_secs(45), async {
            let receiver = a.global_params.library_sync.as_ref().unwrap();
            loop {
                if receiver.get(&key).await.unwrap().is_some()
                    && b.global_params
                        .library_sync
                        .as_ref()
                        .unwrap()
                        .get(&reverse_key)
                        .await
                        .unwrap()
                        .is_some()
                    && let Some(track) = media_files::Entity::find()
                        .filter(Column::LocalAvailable.eq(false))
                        .one(&*a.global_params.main_db)
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
            .add_trusted_domains(["127.0.0.2"], &b_info.fingerprint)
            .await?;
        let source = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(source) = a.resolve_remote_audio(remote_id).await {
                    break source;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .context("could not resolve remote audio")?;
        let expected = std::fs::read(second.path().join("source.ogg"))?;
        let full = source.request.get(None).await?;
        assert_eq!(full.status(), reqwest::StatusCode::OK);
        assert_eq!(full.bytes().await?.as_ref(), expected.as_slice());
        // Expire the server-side token, then exercise a real ranged GET with renewal.
        b.sync_auth_for_test
            .lock()
            .await
            .as_ref()
            .unwrap()
            .expire_tokens_for_test()
            .await;
        let response = source.request.get(Some((0, Some(15)))).await?;
        assert_eq!(response.status(), reqwest::StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.bytes().await?.as_ref(), &expected[..16]);
        let (_, raw_client) = authenticated_client(
            &a.global_params,
            &a_info.fingerprint,
            &a.private_key,
            &b_info.fingerprint,
            "127.0.0.2",
            b_addr.port(),
        )
        .await?;
        let suffix = raw_client
            .get(&source.url)
            .header(reqwest::header::RANGE, "bytes=-12")
            .send()
            .await?;
        assert_eq!(suffix.status(), reqwest::StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            suffix.bytes().await?.as_ref(),
            &expected[expected.len() - 12..]
        );
        let invalid = raw_client
            .get(&source.url)
            .header(reqwest::header::RANGE, format!("bytes={}-", expected.len()))
            .send()
            .await?;
        assert_eq!(invalid.status(), reqwest::StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            invalid
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .unwrap(),
            format!("bytes */{}", expected.len()).as_str()
        );
        let stream_cancel = source.cancellation.clone();
        let stream = playback::stream_utils::create_stream_with_authorizer(
            source.request.clone(),
            &source.url,
        )
        .await?;
        let extension = source.extension;
        let decoded = tokio::task::spawn_blocking(move || -> Result<bool> {
            use std::io::{Read, Seek, SeekFrom};
            let mut stream = stream;
            let mut tail = [0u8; 12];
            stream.seek(SeekFrom::End(-12))?;
            stream.read_exact(&mut tail)?;
            anyhow::ensure!(
                tail.as_slice() == &expected[expected.len() - 12..],
                "stream seek returned wrong bytes"
            );
            stream.seek(SeekFrom::Start(0))?;
            let mut decoder = RuneDecoder::new(stream, Some(&extension))?;
            Ok(decoder.next().is_some())
        })
        .await??;
        assert!(decoded, "remote audio must decode without materializing it");
        b.global_params
            .permission_manager
            .write()
            .await
            .change_user_status(&a_info.fingerprint, UserStatus::Blocked)
            .await?;
        assert!(source.request.get(Some((16, Some(31)))).await.is_err());
        tokio::time::timeout(Duration::from_secs(10), stream_cancel.cancelled())
            .await
            .context("revocation did not cancel the remote stream")?;
        b.global_params
            .permission_manager
            .write()
            .await
            .change_user_status(&a_info.fingerprint, UserStatus::Approved)
            .await?;
        assert!(
            !is_materialized(
                a.global_params.main_db.as_ref(),
                &a.global_params.fsio,
                first.path(),
                remote_id
            )
            .await?
        );
        let (_, authorized) = sync_worker::authenticated_client(
            &b.global_params,
            &b_info.fingerprint,
            &b.private_key,
            &a_info.fingerprint,
            "127.0.0.1",
            a_addr.port(),
        )
        .await?;
        for path in ["/files/anything", "/ws", "/panel/users"] {
            assert_eq!(
                authorized
                    .get(format!("{base}{path}"))
                    .send()
                    .await?
                    .status(),
                reqwest::StatusCode::NOT_FOUND
            );
        }
        a.clone()
            .start(
                a_addr,
                DiscoveryParams {
                    device_info: a_info.clone(),
                },
            )
            .await?;
        let (_, full_authorized) = authenticated_client(
            &b.global_params,
            &b_info.fingerprint,
            &b.private_key,
            &a_info.fingerprint,
            "127.0.0.1",
            a_addr.port(),
        )
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
            assert_eq!(
                request(&pinned).send().await?.status(),
                reqwest::StatusCode::UNAUTHORIZED,
                "{path}"
            );
            assert_ne!(
                request(&full_authorized).send().await?.status(),
                reqwest::StatusCode::UNAUTHORIZED,
                "{path}"
            );
        }
        let query = urlencoding::encode(&b_info.fingerprint);
        assert_eq!(
            pinned
                .get(format!("{base}/ws?fingerprint={query}"))
                .header("Connection", "upgrade")
                .header("Upgrade", "websocket")
                .header("Sec-WebSocket-Version", "13")
                .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
                .send()
                .await?
                .status(),
            reqwest::StatusCode::UNAUTHORIZED
        );
        let full = pinned
            .post(format!("{base}/panel/auth/login"))
            .send()
            .await?
            .status();
        assert_ne!(full, reqwest::StatusCode::NOT_FOUND);
        a.stop().await?;
        a.clone()
            .start_sync(
                a_addr,
                DiscoveryParams {
                    device_info: a_info,
                },
            )
            .await?;
        assert_eq!(
            pinned
                .get(format!("{base}/files/anything"))
                .send()
                .await?
                .status(),
            reqwest::StatusCode::UNAUTHORIZED
        );
        b.global_params.main_token.cancel();
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
        b.stop().await?;
        a.stop().await?;
        a.global_params.main_token.cancel();
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
            &params.fsio,
            &params.main_db,
            &params.node_id,
            root.path(),
            ScanOptions {
                cleanup: false,
                force: false,
                missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
                cancel_token: None,
            },
            |_| {},
            &params.library_sync.as_ref().unwrap().record_store(),
        )
        .await?;
        let store =
            LibrarySyncStore::open((*params.main_db).clone(), params.node_id.parse()?, None)
                .await?;
        store.import_existing(params.node_id.parse()?).await?;
        let file = get_media_files(&params.main_db, 0, 10)
            .await?
            .pop()
            .context("missing scanned audio")?;
        let track_id: Uuid = file.hlc_uuid.parse()?;
        let resolver = sync_worker::PairedAudioResolver(Arc::downgrade(&manager));
        let queued_before_revision = PlaybackSource::Local(root.path().join("old.ogg"));
        assert!(
            !queued_before_revision
                .requires_stream_at_load(file.id, &resolver)
                .await?
        );
        let blob = hash_file(&params.fsio, &fixture_dir.join("startup_1.ogg"))?;
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
            &params.fsio,
            &params.main_db,
            &params.node_id,
            &root.path(),
            &handles,
        )
        .await?;
        assert!(
            matches!(&queued[..], [(PlayingItem::InLibrary(id), PlaybackSource::LibraryStream)] if *id == file.id)
        );
        let summaries =
            get_metadata_summary_by_files(params.main_db.as_ref(), vec![file.clone()]).await?;
        let parsed = crate::utils::parse_media_files(
            &params.fsio,
            &params.main_db,
            &params.node_id,
            summaries,
            params.lib_path.clone(),
        )
        .await?;
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].path.is_empty());
        assert_eq!(
            catalog_track(&params.main_db, file.id).await?,
            Some((track_id, blob.clone(), "ogg".into()))
        );
        use playback::player::PlaybackSourceResolver;
        let queued_remote = PlaybackSource::LibraryStream;
        let updated_path = prepare_download(
            &params.main_db,
            &params.fsio,
            root.path(),
            track_id,
            &blob,
            "ogg",
        )
        .await?;
        std::fs::copy(fixture_dir.join("startup_1.ogg"), &updated_path)?;
        let scan_error = scan_audio_file(
            &params.fsio,
            &params.main_db,
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
            &params.main_db,
            &store.record_store(),
            &params.fsio,
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
            Some(params.fsio.canonicalize_path(&updated_path)?)
        );
        assert!(
            !queued_before_revision
                .requires_stream_at_load(file.id, &resolver)
                .await?
        );
        let current = get_media_files(&params.main_db, 0, 10)
            .await?
            .pop()
            .context("missing updated audio")?;
        assert_eq!(current.id, file.id);
        assert_eq!(resolver.resolve_id(file.id).await?, Some(current.id));
        params.main_token.cancel();
        Ok(())
    }
}
