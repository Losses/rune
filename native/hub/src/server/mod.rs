#[macro_use]
mod server_request;
pub mod api;
pub mod http;
mod manager;
pub mod utils;

pub use manager::generate_or_load_certificates;
pub use manager::get_or_generate_certificate_id;
pub use manager::ServerManager;

use std::{collections::HashMap, future::Future, path::PathBuf, pin::Pin, sync::Arc};

use discovery::{server::PermissionManager, utils::DeviceInfo};
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
    pub permission_manager: Arc<RwLock<PermissionManager>>,
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
