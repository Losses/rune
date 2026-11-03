use super::sync::http as super_sync_http;
use super::sync::http::BlobContext;
use super::sync::http::SharedLibrary;
use super::sync::http::blob_router;
use super::sync_auth::SyncAuth;
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

        for_all_request_pairs2!(
            listen_server_event,
            websocket_service.clone(),
            self.global_params.clone()
        );

        let app_state = Arc::new(AppState {
            lib_path: PathBuf::from(&*self.global_params.lib_path),
            cover_temp_dir: COVER_TEMP_DIR.clone(),
        });

        let server_state = Arc::new(ServerState {
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
            app.merge(auth_routes)
                .merge(protected_routes)
                .route("/ws", get(websocket_handler))
                .route("/files/{*file_path}", get(file_handler))
                .route("/media/metadata/{id}", get(get_media_metadata_handler))
                .route("/media/cover/{id}", get(get_cover_art_handler))
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

        let watch_cancel = tokio_util_sync::CancellationToken::new();
        let watch_stop = watch_cancel.clone();
        let lifetime_handle = shutdown_handle.clone();
        let lifetime_token = self.global_params.main_token.clone();
        let lifetime_scanner = self.global_params.device_scanner.clone();
        *self.watch_handle.lock().await = Some(tokio::spawn(async move {
            tokio::select! {
                _ = watch_stop.cancelled() => {},
                _ = lifetime_token.cancelled() => {
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
    use super::super::sync_worker::authenticated_client;
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
    use database::actions::downloads::prepare_download;
    use database::actions::file::{attach_download, catalog_track, hash_file, is_materialized};
    use database::actions::metadata::{
        MissingHostFilePolicy, ScanOptions, scan_audio_file, scan_audio_library,
    };
    use database::entities::{media_files, media_files::Column};
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
    use std::sync::OnceLock;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    struct Quiet;
    impl Broadcaster for Quiet {
        fn broadcast(&self, _: &dyn RinfRustSignal) {}
    }

    async fn fixture(dir: &std::path::Path) -> Result<Arc<ServerManager>> {
        let fsio = Arc::new(FsIo::new());
        let db = sea_orm::Database::connect("sqlite::memory:").await?;
        let node = Uuid::new_v4();
        let _ = migration::initialize_node_id(node.to_string());
        Migrator::up(&db, None).await?;
        let token = Arc::new(CancellationToken::new());
        let library_sync = crate::utils::initialize_library_replica(&db, &node.to_string()).await?;
        let params = Arc::new(GlobalParams {
            sync_settings: Arc::new(RwLock::new(SyncSettings::default())),
            fsio,
            lib_path: Arc::new(dir.to_string_lossy().into_owned()),
            config_path: Arc::new(dir.to_string_lossy().into_owned()),
            node_id: Arc::new(node.to_string()),
            main_db: Arc::new(db.clone()),
            library_sync: Some(library_sync),
            recommend_db: Arc::new(RecommendationDbConnection::new()),
            main_token: token.clone(),
            task_tokens: Arc::new(Mutex::new(TaskTokens::default())),
            player: Arc::new(Mutex::new(MockPlayer)),
            sfx_player: Arc::new(Mutex::new(SfxPlayer::new(Some((*token).clone())))),
            scrobbler: Arc::new(Mutex::new(MockScrobblingManager::new())),
            broadcaster: Arc::new(Quiet),
            device_scanner: Arc::new(DiscoveryService::without_store()),
            cert_validator: Arc::new(RwLock::new(CertValidator::new(dir).await?)),
            permission_manager: Arc::new(RwLock::new(PermissionManager::new(dir)?)),
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
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn paired_background_sync_and_listener_lifecycle() -> Result<()> {
        let _ = tracing_subscriber::fmt()
            .with_max_level(LevelFilter::WARN)
            .with_test_writer()
            .try_init();
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        let a = fixture(first.path()).await?;
        let b = fixture(second.path()).await?;
        let a_info = info(&a).await?;
        let b_info = info(&b).await?;
        let certificate = b.certificate.clone();
        a.global_params
            .permission_manager
            .write()
            .await
            .add_user(
                certificate,
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
        let a_reserve = TcpListener::bind("127.0.0.1:0")?;
        let b_reserve = TcpListener::bind("127.0.0.1:0")?;
        let a_addr = a_reserve.local_addr()?;
        let b_addr = SocketAddr::from(([127, 0, 0, 2], b_reserve.local_addr()?.port()));
        assert_ne!(a_addr.port(), b_addr.port());
        drop((a_reserve, b_reserve));
        b.global_params
            .cert_validator
            .read()
            .await
            .add_trusted_domains(["127.0.0.1"], &a_info.fingerprint)
            .await?;
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
            .write_local(
                key.clone(),
                Some(Payload::Album(Album {
                    name: "background album".into(),
                    group: "test".into(),
                })),
            )
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
                .post(format!("{base}/sync/metadata"))
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
