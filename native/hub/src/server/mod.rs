#[macro_use]
mod server_request;
mod access;
pub mod api;
pub mod http;
pub(crate) mod libraries;
mod manager;
pub mod sync;
mod sync_auth;
pub(crate) mod sync_worker;
pub use sync_worker::PairedAudioResolver;
pub mod utils;

pub use manager::{
    ServerManager, generate_or_load_certificates, get_or_generate_alias, update_root_password,
};

use std::{
    collections::HashMap,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Weak},
};

use anyhow::Result;
use log::error;
use tokio::sync::{Mutex, RwLock, broadcast};

use ::discovery::{protocol::DiscoveryService, server::PermissionManager, utils::DeviceInfo};

use crate::{
    Session,
    backends::remote::encode_message,
    node::NodeContext,
    utils::{Broadcaster, RinfRustSignal},
};

pub type HandlerFn = Box<
    dyn Fn(
            Vec<u8>,
            Option<Session>,
            Option<uuid::Uuid>,
        ) -> BoxFuture<'static, (String, Result<Vec<u8>>)>
        + Send
        + Sync,
>;
pub type HandlerMap = Arc<Mutex<HashMap<String, HandlerFn>>>;
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

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
        F: Fn(Vec<u8>, Option<Session>, Option<uuid::Uuid>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = (String, Result<Vec<u8>>)> + Send + 'static,
    {
        self.handlers.lock().await.insert(
            msg_type.to_string(),
            Box::new(move |payload, session, library_id| {
                Box::pin(handler(payload, session, library_id))
            }),
        );
    }

    pub async fn handle_addressed_message(
        &self,
        msg_type: &str,
        payload: Vec<u8>,
        session: Option<Session>,
        library_id: Option<uuid::Uuid>,
    ) -> Option<(String, Result<Vec<u8>>)> {
        self.dispatch_message(msg_type, payload, session, library_id)
            .await
    }

    pub async fn handle_message(
        &self,
        msg_type: &str,
        payload: Vec<u8>,
        session: Option<Session>,
    ) -> Option<(String, Result<Vec<u8>>)> {
        self.dispatch_message(msg_type, payload, session, None)
            .await
    }

    async fn dispatch_message(
        &self,
        msg_type: &str,
        payload: Vec<u8>,
        session: Option<Session>,
        library_id: Option<uuid::Uuid>,
    ) -> Option<(String, Result<Vec<u8>>)> {
        let future = {
            let handlers = self.handlers.lock().await;
            let handler = handlers.get(msg_type)?;
            handler(payload, session, library_id)
        };
        Some(future.await)
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
            handlers: self.handlers.clone(),
            broadcast_tx: self.broadcast_tx.clone(),
        }
    }
}

/// Register a test handler that signals when it starts and waits for a release.
#[cfg(test)]
pub(crate) async fn register_gated_handler(
    service: &WebSocketService,
    name: &str,
) -> (Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>) {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let entered_handler = entered.clone();
    let release_handler = release.clone();
    let response = format!("{name}-response");
    service
        .register_handler(name, move |_, _, _| {
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
    use std::sync::Arc;
    use tokio::time::{Duration, timeout};

    #[tokio::test]
    async fn blocked_request_does_not_block_another_request() {
        let service = Arc::new(WebSocketService::new());
        let (entered, release) = register_gated_handler(&service, "slow").await;
        service
            .register_handler("fast", |_, _, _| async {
                ("fast-response".into(), Ok(Vec::new()))
            })
            .await;

        let slow_service = service.clone();
        let slow =
            tokio::spawn(
                async move { slow_service.handle_message("slow", Vec::new(), None).await },
            );
        timeout(Duration::from_secs(2), entered.notified())
            .await
            .expect("slow handler did not enter");

        let fast_result = timeout(
            Duration::from_secs(2),
            service.handle_message("fast", Vec::new(), None),
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
    }

    #[tokio::test]
    async fn registration_is_not_blocked_by_a_running_handler() {
        let service = Arc::new(WebSocketService::new());
        let (entered, release) = register_gated_handler(&service, "slow").await;

        let slow_service = service.clone();
        let slow =
            tokio::spawn(
                async move { slow_service.handle_message("slow", Vec::new(), None).await },
            );
        timeout(Duration::from_secs(2), entered.notified())
            .await
            .expect("slow handler did not enter");

        timeout(
            Duration::from_secs(2),
            service.register_handler("new", |_, _, _| async {
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
    }
}
