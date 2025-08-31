use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

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
use tokio::{
    fs::{read_to_string, write},
    sync::{Mutex, RwLock},
    task::JoinHandle,
};
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
            check_fingerprint::check_fingerprint_handler, device_info::device_info_handler,
            file::file_handler, list::list_users_handler, media::{get_cover_art_handler, get_media_metadata_handler}, panel_alias::update_alias_handler,
            panel_auth_middleware::auth_middleware, panel_broadcast::toggle_broadcast_handler,
            panel_delete_user::delete_user_handler, panel_login::login_handler,
            panel_refresh::refresh_handler, panel_self::self_handler,
            panel_status::update_user_status_handler, ping::ping_handler,
            register::register_handler, websocket::websocket_handler,
        },
    },
    utils::{GlobalParams, ParamsExtractor, RinfRustSignal},
};

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
    addr: Mutex<Option<SocketAddr>>,
    is_running: std::sync::atomic::AtomicBool,
    shutdown_handle: Mutex<Option<Handle>>,
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
            addr: Mutex::new(None),
            is_running: AtomicBool::new(false),
            shutdown_handle: Mutex::new(None),
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
    ) -> Result<()>
    where
        Self: Send,
    {
        if self.is_running.load(Ordering::SeqCst) {
            return Err(anyhow::anyhow!("Server already running"));
        }

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

        let app = Router::new()
            .merge(register_route)
            .merge(auth_routes)
            .merge(protected_routes)
            .route("/ping", get(ping_handler))
            .route("/ws", get(websocket_handler))
            .route("/check-fingerprint", get(check_fingerprint_handler))
            .route("/files/{*file_path}", get(file_handler))
            .route("/device-info", get(device_info_handler))
            .route("/media/metadata/:id", get(get_media_metadata_handler))
            .route("/media/cover/:id", get(get_cover_art_handler))
            .with_state(server_state)
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

        let server_handle = tokio::spawn(async move {
            info!("Starting secure HTTPS/WSS server on {addr}");
            let server = axum_server::bind_rustls(addr, tls_config)
                .handle(handle)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>());

            match server.await {
                Ok(_) => info!("Server stopped gracefully"),
                Err(e) => error!("Server error: {e}"),
            }
        });

        *self.server_handle.lock().await = Some(server_handle);
        *self.addr.lock().await = Some(addr);
        *self.shutdown_handle.lock().await = Some(shutdown_handle);
        self.is_running.store(true, Ordering::SeqCst);

        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        if !self.is_running.load(Ordering::SeqCst) {
            return Err(anyhow::anyhow!("Server not running"));
        }

        if let Some(handle) = self.shutdown_handle.lock().await.as_ref() {
            handle.shutdown();
        }

        if let Some(handle) = self.server_handle.lock().await.take() {
            handle.await?;
        }

        *self.addr.lock().await = None;
        *self.shutdown_handle.lock().await = None;
        self.is_running.store(false, Ordering::SeqCst);

        Ok(())
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
