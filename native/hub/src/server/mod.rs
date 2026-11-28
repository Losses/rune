mod access;
pub mod api;
pub mod http;
pub(crate) mod libraries;
mod manager;
pub(crate) mod server_request;
pub mod sync;
mod sync_auth;
pub(crate) mod sync_worker;
pub use sync_worker::PairedAudioResolver;
use tokio_util::sync as tokio_util_sync;
pub mod utils;

pub use manager::{
    ServerManager, generate_or_load_certificates, get_or_generate_alias, update_root_password,
};

use std::{
    path::PathBuf,
    sync::{Arc, Weak},
};

use anyhow::Result;
use log::error;
use tokio::sync::{RwLock, broadcast};
use uuid::Uuid;

use ::discovery::{protocol::DiscoveryService, server::PermissionManager, utils::DeviceInfo};

use crate::{
    Session,
    backends::remote::encode_message,
    macros::dispatch_remote,
    messages::LibraryTaskStateChanged,
    node::NodeContext,
    utils::{Broadcaster, RinfRustSignal},
};

pub type BroadcastTx = broadcast::Sender<Vec<u8>>;

pub struct AppState {
    pub cover_temp_dir: PathBuf,
}

#[derive(Clone)]
pub struct ServerState {
    pub node: Weak<NodeContext>,
    pub app_state: Arc<AppState>,
    pub websocket_service: Arc<WebSocketService>,
    pub session_cancel: tokio_util_sync::CancellationToken,
    pub discovery_device_info: Arc<RwLock<DeviceInfo>>,
    pub permission_manager: Arc<RwLock<PermissionManager>>,
    pub device_scanner: Arc<DiscoveryService>,
}

pub struct WebSocketService {
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
        WebSocketService { broadcast_tx }
    }

    pub async fn handle_addressed_message(
        &self,
        msg_type: &str,
        payload: Vec<u8>,
        session: Option<Session>,
        library_id: Option<Uuid>,
        node: &Arc<NodeContext>,
    ) -> Option<(String, Result<Vec<u8>>)> {
        dispatch_remote(msg_type, &payload, session, library_id, node).await
    }

    #[cfg(test)]
    async fn handle_message(
        &self,
        msg_type: &str,
        payload: Vec<u8>,
        session: Option<Session>,
        node: &Arc<NodeContext>,
    ) -> Option<(String, Result<Vec<u8>>)> {
        self.handle_addressed_message(msg_type, payload, session, None, node)
            .await
    }
}

impl Broadcaster for WebSocketService {
    fn broadcast(&self, message: &dyn RinfRustSignal) {
        let type_name = message.name();
        let remote_scan = message
            .as_any()
            .downcast_ref::<LibraryTaskStateChanged>()
            .and_then(|event| event.state.remote_scan_failure());
        let payload = match remote_scan.map_or_else(
            || message.encode_to_vec(),
            |state| LibraryTaskStateChanged { state }.encode_to_vec(),
        ) {
            Ok(payload) => payload,
            Err(e) => {
                error!("Failed to encode message: {e}");
                return;
            }
        };

        let message_data = encode_message(&type_name, &payload, None);

        if let Err(e) = self.broadcast_tx.send(message_data) {
            error!("Failed to broadcast message: {e}");
        }
    }
}

impl Clone for WebSocketService {
    fn clone(&self) -> Self {
        WebSocketService {
            broadcast_tx: self.broadcast_tx.clone(),
        }
    }
}
