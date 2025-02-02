use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use axum::{routing::get, Router};
use axum_server::Handle;
use log::{error, info};
use prost::Message;
use tokio::{
    sync::{Mutex, RwLock},
    task::JoinHandle,
};

use database::actions::cover_art::COVER_TEMP_DIR;
use discovery::{
    http_api::create_discovery_router, permission::PermissionManager, DiscoveryParams,
};

use crate::{
    messages::*,
    server::{
        handlers::{file_handler::file_handler, websocket_handler::websocket_handler},
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
}

impl ServerManager {
    pub fn new(global_params: Arc<GlobalParams>) -> Self {
        Self {
            global_params,
            server_handle: Mutex::new(None),
            addr: Mutex::new(None),
            is_running: std::sync::atomic::AtomicBool::new(false),
            shutdown_handle: Mutex::new(None),
        }
    }

    pub async fn start<P: AsRef<Path>>(
        &self,
        addr: SocketAddr,
        discovery_params: DiscoveryParams,
        permission_path: P,
    ) -> Result<()>
    where
        Self: Send,
    {
        if self.is_running.load(std::sync::atomic::Ordering::SeqCst) {
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

        let permission_manager = Arc::new(PermissionManager::new(permission_path)?);

        let server_state = Arc::new(ServerState {
            app_state: app_state.clone(),
            websocket_service: websocket_service.clone(),
            discovery_device_info: discovery_params.device_info,
            discovery_active_sessions: Arc::new(RwLock::new(HashMap::new())),
            permission_manager,
        });

        let app = Router::new()
            .route("/ws", get(websocket_handler))
            .route("/files/{*file_path}", get(file_handler))
            .nest("/api/discovery", create_discovery_router())
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

        let server_handle = tokio::spawn(async move {
            info!("Starting combined HTTP/WebSocket server on {}", addr);
            let server = axum_server::bind(addr)
                .handle(handle)
                .serve(app.into_make_service());

            match server.await {
                Ok(_) => info!("Server stopped gracefully"),
                Err(e) => error!("Server error: {}", e),
            }
        });

        *self.server_handle.lock().await = Some(server_handle);
        *self.addr.lock().await = Some(addr);
        *self.shutdown_handle.lock().await = Some(shutdown_handle);
        self.is_running
            .store(true, std::sync::atomic::Ordering::SeqCst);

        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        if !self.is_running.load(std::sync::atomic::Ordering::SeqCst) {
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
        self.is_running
            .store(false, std::sync::atomic::Ordering::SeqCst);

        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.is_running.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub async fn get_address(&self) -> Option<SocketAddr> {
        *self.addr.lock().await
    }
}
