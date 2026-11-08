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

#[cfg(test)]
use futures::future::BoxFuture;
#[cfg(test)]
use std::future::Future;
#[cfg(test)]
use tokio::sync::{Mutex, Notify};

use ::discovery::{protocol::DiscoveryService, server::PermissionManager, utils::DeviceInfo};

use crate::{
    Session,
    backends::remote::encode_message,
    macros::dispatch_remote,
    node::NodeContext,
    utils::{Broadcaster, RinfRustSignal},
};

#[cfg(test)]
type TestOverride = Box<
    dyn Fn(Vec<u8>, Option<Session>, Option<Uuid>) -> BoxFuture<'static, (String, Result<Vec<u8>>)>
        + Send
        + Sync,
>;

pub type BroadcastTx = broadcast::Sender<Vec<u8>>;

pub struct AppState {
    pub cover_temp_dir: PathBuf,
}

#[derive(Clone)]
pub struct ServerState {
    pub node: Weak<NodeContext>,
    pub app_state: Arc<AppState>,
    pub websocket_service: Arc<WebSocketService>,
    pub session_cancel: tokio_util::sync::CancellationToken,
    pub discovery_device_info: Arc<RwLock<DeviceInfo>>,
    pub permission_manager: Arc<RwLock<PermissionManager>>,
    pub device_scanner: Arc<DiscoveryService>,
}

pub struct WebSocketService {
    #[cfg(test)]
    overrides: Arc<Mutex<Vec<(String, TestOverride)>>>,
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
            #[cfg(test)]
            overrides: Arc::new(Mutex::new(Vec::new())),
            broadcast_tx,
        }
    }

    #[cfg(test)]
    pub(crate) async fn override_handler<F, Fut>(&self, msg_type: &str, handler: F)
    where
        F: Fn(Vec<u8>, Option<Session>, Option<Uuid>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = (String, Result<Vec<u8>>)> + Send + 'static,
    {
        self.overrides.lock().await.push((
            msg_type.to_string(),
            Box::new(move |payload, session, library_id| {
                Box::pin(handler(payload, session, library_id))
            }),
        ));
    }

    pub async fn handle_addressed_message(
        &self,
        msg_type: &str,
        payload: Vec<u8>,
        session: Option<Session>,
        library_id: Option<Uuid>,
        node: &Arc<NodeContext>,
    ) -> Option<(String, Result<Vec<u8>>)> {
        #[cfg(test)]
        {
            let overrides = self.overrides.lock().await;
            if let Some((_, handler)) = overrides.iter().rev().find(|(name, _)| name == msg_type) {
                let future = handler(payload, session, library_id);
                drop(overrides);
                return Some(future.await);
            }
        }
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
        let payload = match message.encode_to_vec() {
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
            #[cfg(test)]
            overrides: self.overrides.clone(),
            broadcast_tx: self.broadcast_tx.clone(),
        }
    }
}

/// Register a test handler that signals when it starts and waits for a release.
#[cfg(test)]
pub(crate) async fn register_gated_handler(
    service: &WebSocketService,
    name: &str,
) -> (Arc<Notify>, Arc<Notify>) {
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let entered_handler = entered.clone();
    let release_handler = release.clone();
    let response = format!("{name}-response");
    service
        .override_handler(name, move |_, _, _| {
            let entered = entered_handler.clone();
            let release = release_handler.clone();
            let response = response.clone();
            async move {
                entered.notify_one();
                release.notified().await;
                (response, Ok(Vec::new()))
            }
        })
        .await;
    (entered, release)
}

#[cfg(test)]
mod tests {
    use super::{WebSocketService, register_gated_handler};
    use crate::node::NodeContext;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::time::{Duration, timeout};

    async fn test_node() -> (TempDir, Arc<NodeContext>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        let node = NodeContext::new(path, path, Arc::new(WebSocketService::new()), false)
            .await
            .unwrap();
        (dir, node)
    }

    #[tokio::test]
    async fn blocked_request_does_not_block_another_request() {
        let (_dir, node) = test_node().await;
        let service = node.websocket_service.clone();
        let (entered, release) = register_gated_handler(&service, "slow").await;
        service
            .override_handler("fast", |_, _, _| async {
                ("fast-response".into(), Ok(Vec::new()))
            })
            .await;

        let slow_service = service.clone();
        let slow_node = node.clone();
        let slow = tokio::spawn(async move {
            slow_service
                .handle_message("slow", Vec::new(), None, &slow_node)
                .await
        });
        timeout(Duration::from_secs(2), entered.notified())
            .await
            .expect("slow handler did not enter");

        let fast_result = timeout(
            Duration::from_secs(2),
            service.handle_message("fast", Vec::new(), None, &node),
        )
        .await
        .expect("fast request blocked behind slow request");
        assert_eq!(
            fast_result.expect("fast handler missing").0,
            "fast-response"
        );
        assert!(
            !slow.is_finished(),
            "slow request finished before its gate opened"
        );

        release.notify_one();
        let slow_result = timeout(Duration::from_secs(2), slow)
            .await
            .expect("slow request did not finish after release")
            .expect("slow task panicked");
        assert_eq!(
            slow_result.expect("slow handler missing").0,
            "slow-response"
        );
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn registration_is_not_blocked_by_a_running_handler() {
        let (_dir, node) = test_node().await;
        let service = node.websocket_service.clone();
        let (entered, release) = register_gated_handler(&service, "slow").await;

        let slow_service = service.clone();
        let slow_node = node.clone();
        let slow = tokio::spawn(async move {
            slow_service
                .handle_message("slow", Vec::new(), None, &slow_node)
                .await
        });
        timeout(Duration::from_secs(2), entered.notified())
            .await
            .expect("slow handler did not enter");

        timeout(
            Duration::from_secs(2),
            service.override_handler("new", |_, _, _| async {
                ("new-response".into(), Ok(Vec::new()))
            }),
        )
        .await
        .expect("registration blocked behind slow request");

        release.notify_one();
        timeout(Duration::from_secs(2), slow)
            .await
            .expect("slow request did not finish after release")
            .expect("slow task panicked");
        node.shutdown().await.unwrap();
    }
}
