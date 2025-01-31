#[macro_use]
mod server_request;
mod manager;
pub mod handlers;
pub use manager::ServerManager;

use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};

use anyhow::Result;
use async_trait::async_trait;
use discovery::{
    http_api::{DiscoveryState, FileProvider, SessionInfo},
    pin::{PinConfig, PinValidationState},
    utils::{DeviceInfo, FileMetadata},
};
use log::error;
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::{
    remote::encode_message,
    utils::{Broadcaster, RinfRustSignal},
};

pub type HandlerFn = Box<dyn Fn(Vec<u8>) -> BoxFuture<'static, (String, Vec<u8>)> + Send + Sync>;
pub type HandlerMap = Arc<Mutex<HashMap<String, HandlerFn>>>;
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub type BroadcastTx = broadcast::Sender<Vec<u8>>;

pub struct AppState {
    pub lib_path: PathBuf,
    pub cover_temp_dir: PathBuf,
}

#[derive(Clone)]
pub struct ServerState {
    pub app_state: Arc<AppState>,
    pub websocket_service: Arc<WebSocketService>,
    pub discovery_device_info: DeviceInfo,
    pub discovery_active_sessions: Arc<RwLock<HashMap<String, SessionInfo>>>,
    pub discovery_pin_config: Arc<RwLock<PinConfig>>,
    pub discovery_file_provider: Arc<dyn FileProvider>,
}

impl PinValidationState for ServerState {
    fn pin_config(&self) -> &Arc<RwLock<PinConfig>> {
        &self.discovery_pin_config
    }
}

impl DiscoveryState for ServerState {
    type FileProvider = dyn FileProvider;

    fn device_info(&self) -> &DeviceInfo {
        &self.discovery_device_info
    }

    fn active_sessions(&self) -> Arc<RwLock<HashMap<String, SessionInfo>>> {
        self.discovery_active_sessions.clone()
    }

    fn pin_config(&self) -> Arc<RwLock<PinConfig>> {
        self.discovery_pin_config.clone()
    }

    fn file_provider(&self) -> Arc<Self::FileProvider> {
        self.discovery_file_provider.clone()
    }
}

pub struct LocalFileProvider {
    pub root_dir: PathBuf,
}

impl LocalFileProvider {
    pub fn new<P: AsRef<Path>>(root_dir: P) -> Self {
        Self {
            root_dir: root_dir.as_ref().to_path_buf(),
        }
    }
}

#[async_trait]
impl FileProvider for LocalFileProvider {
    async fn get_files(&self) -> Result<HashMap<String, FileMetadata>> {
        Ok(HashMap::new())
    }
}

pub struct WebSocketService {
    pub handlers: HandlerMap,
    pub broadcast_tx: BroadcastTx,
}

impl Default for WebSocketService {
    fn default() -> Self {
        WebSocketService::new()
    }
}

impl WebSocketService {
    pub fn new() -> Self {
        let (broadcast_tx, _) = broadcast::channel(100);
        WebSocketService {
            handlers: Arc::new(Mutex::new(HashMap::new())),
            broadcast_tx,
        }
    }

    pub async fn register_handler<F, Fut>(&self, msg_type: &str, handler: F)
    where
        F: Fn(Vec<u8>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = (String, Vec<u8>)> + Send + 'static,
    {
        self.handlers.lock().await.insert(
            msg_type.to_string(),
            Box::new(move |payload| Box::pin(handler(payload))),
        );
    }

    pub async fn handle_message(
        &self,
        msg_type: &str,
        payload: Vec<u8>,
    ) -> Option<(String, Vec<u8>)> {
        let handlers = self.handlers.lock().await;
        let handler = handlers.get(msg_type)?;
        Some(handler(payload).await)
    }
}

impl Broadcaster for WebSocketService {
    fn broadcast(&self, message: &dyn RinfRustSignal) {
        let type_name = message.name();
        let payload = message.encode_message();
        let message_data = encode_message(&type_name, &payload, None);

        if let Err(e) = self.broadcast_tx.send(message_data) {
            error!("Failed to broadcast message: {}", e);
        }
    }
}

impl Clone for WebSocketService {
    fn clone(&self) -> Self {
        WebSocketService {
            handlers: self.handlers.clone(),
            broadcast_tx: self.broadcast_tx.clone(),
        }
    }
}
