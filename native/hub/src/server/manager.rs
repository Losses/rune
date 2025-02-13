use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use anyhow::{bail, Context, Result};
use axum::{
    routing::{get, post},
    Router,
};
use axum_server::{tls_rustls::RustlsConfig, Handle};
use discovery::verifier::parse_certificate;
use log::{error, info};
use prost::Message;
use rand::distributions::{Alphanumeric, DistString};
use rustls::crypto::aws_lc_rs::default_provider;
use tokio::{sync::Mutex, task::JoinHandle};
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::PeerIpKeyExtractor, GovernorLayer,
};

use ::database::actions::cover_art::COVER_TEMP_DIR;
use ::discovery::{ssl::generate_self_signed_cert, DiscoveryParams};

use crate::{
    messages::*,
    server::{
        handlers::{
            device_info_handler::device_info_handler, file_handler::file_handler,
            ping_handler::ping_handler, register_handler::register_handler,
            websocket_handler::websocket_handler,
        },
        AppState, ServerState, WebSocketService,
    },
    utils::{GlobalParams, ParamsExtractor, RinfRustSignal},
    Signal,
};

#[derive(Debug)]
pub struct ServerManager {
    global_params: Arc<GlobalParams>,
    server_handle: Mutex<Option<JoinHandle<()>>>,
    addr: Mutex<Option<SocketAddr>>,
    is_running: std::sync::atomic::AtomicBool,
    shutdown_handle: Mutex<Option<Handle>>,
    certificate: String,
    private_key: String,
}

impl ServerManager {
    pub async fn new(global_params: Arc<GlobalParams>) -> Result<Self> {
        let config_path = Path::new(&*global_params.config_path);

        let certificate_id = get_or_generate_certificate_id(config_path).await?;

        let (_, certificate, private_key) =
            generate_or_load_certificates(config_path, &certificate_id)
                .await
                .context("Failed to initialize certificates")?;

        Ok(Self {
            global_params,
            server_handle: Mutex::new(None),
            addr: Mutex::new(None),
            is_running: AtomicBool::new(false),
            shutdown_handle: Mutex::new(None),
            certificate,
            private_key,
        })
    }

    pub async fn start(&self, addr: SocketAddr, discovery_params: DiscoveryParams) -> Result<()>
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
            discovery_device_info: discovery_params.device_info,
            permission_manager: self.global_params.permission_manager.clone(),
        });

        let governor_conf = GovernorConfigBuilder::default()
            .per_second(60)
            .burst_size(5)
            .key_extractor(PeerIpKeyExtractor)
            .finish()
            .unwrap();

        let register_route = Router::new()
            .route("/register", post(register_handler))
            .layer(GovernorLayer {
                config: governor_conf.into(),
            });

        let app = Router::new()
            .merge(register_route)
            .route("/ping", get(ping_handler))
            .route("/ws", get(websocket_handler))
            .route("/files/{*file_path}", get(file_handler))
            .route("/device-info", get(device_info_handler))
            .with_state(server_state);

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
            if let Err(e) = default_provider().install_default() {
                bail!(format!("{:#?}", e));
            };

            RustlsConfig::from_pem(
                self.certificate.as_bytes().to_vec(),
                self.private_key.as_bytes().to_vec(),
            )
            .await
            .context("Failed to create TLS configuration")?
        };

        let server_handle = tokio::spawn(async move {
            info!("Starting secure HTTPS/WSS server on {}", addr);
            let server = axum_server::bind_rustls(addr, tls_config)
                .handle(handle)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>());

            match server.await {
                Ok(_) => info!("Server stopped gracefully"),
                Err(e) => error!("Server error: {}", e),
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
}

pub async fn get_or_generate_certificate_id(config_path: &Path) -> Result<String> {
    info!("Generating certificate in: {:?}", config_path);

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

fn generate_random_alias() -> String {
    let mut rng = rand::thread_rng();
    let suffix: String = Alphanumeric.sample_string(&mut rng, 8);
    format!("R-{}", suffix)
}

pub async fn generate_or_load_certificates(
    config_path: &Path,
    certificate_id: &str,
) -> Result<(String, String, String)> {
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
