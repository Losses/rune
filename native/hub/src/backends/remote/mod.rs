#[macro_use]
mod remote_request;

use crate::node::NodeContext;
use crate::request_error::classify;
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::Duration,
};

use anyhow::Result;
use futures::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use reqwest::redirect::Policy;
use rinf::{DartSignal, RustSignal};
use serde::Deserialize;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::{
    Connector, connect_async_tls_with_config,
    tungstenite::{client::IntoClientRequest, protocol::Message as TungsteniteMessage},
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_util::sync::CancellationToken;
use urlencoding::encode;
use uuid::Uuid;

use ::discovery::{
    client::{CertValidator, select_best_host},
    url::decode_rnsrv_url,
};
use ::fsio::FsIo;
use ::playback::{player::MockPlayer, sfx_player::SfxPlayer};
use ::scrobbling::manager::MockScrobblingManager;

use crate::{
    Signal,
    messages::*,
    server::{
        generate_or_load_certificates, get_or_generate_alias,
        sync_worker::{endpoint_host_port, obtain_token, tls_config, trusted_fingerprint},
    },
    utils::{GlobalParams, ParamsExtractor, RinfRustSignal, RunningMode, TaskTokens},
};

pub trait RinfDartSignal {
    fn name(&self) -> String;
    fn decode(x: &[u8]) -> Result<Self>
    where
        Self: Sized + for<'a> Deserialize<'a>,
    {
        rinf::deserialize(x).map_err(|e| anyhow::anyhow!("Deserialization failed: {e}"))
    }
}

for_all_requests0!(implement_rinf_dart_signal_trait);
for_all_responses0!(implement_rinf_rust_signal_trait);

pub fn encode_message(type_name: &str, payload: &[u8], uuid: Option<Uuid>) -> Vec<u8> {
    let type_len = type_name.len() as u8;
    let mut message_data = vec![type_len];
    message_data.extend_from_slice(type_name.as_bytes());
    message_data.extend_from_slice(payload);

    let request_id = match uuid {
        Some(x) => x,
        None => Uuid::new_v4(),
    };
    message_data.extend_from_slice(request_id.as_bytes());

    message_data
}

pub fn decode_message(payload: &[u8]) -> Option<(String, Vec<u8>, Uuid)> {
    if payload.len() < 17 {
        return None;
    }

    let type_len = payload[0] as usize;
    if payload.len() < 1 + type_len + 16 {
        return None;
    }

    let msg_type = String::from_utf8_lossy(&payload[1..1 + type_len]).to_string();
    let msg_payload = payload[1 + type_len..payload.len() - 16].to_vec();
    let request_id = Uuid::from_slice(&payload[payload.len() - 16..]).ok()?;
    Some((msg_type, msg_payload, request_id))
}

type MessageHandler = Box<dyn Fn(Vec<u8>) + Send + Sync>;
type HandlerMap = Arc<Mutex<HashMap<String, MessageHandler>>>;

#[derive(Debug, thiserror::Error)]
pub enum RemoteConnectError {
    #[error("remote endpoint has no trusted identity")]
    UntrustedEndpoint,
    #[error("remote device is not authorized")]
    NotAuthorized,
    #[error("remote endpoint is unreachable")]
    Unreachable,
    #[error("remote protocol error: {0}")]
    Protocol(String),
}

impl RemoteConnectError {
    pub fn kind(&self) -> LibraryOpenErrorKind {
        match self {
            Self::UntrustedEndpoint => LibraryOpenErrorKind::RemoteUntrusted,
            Self::NotAuthorized => LibraryOpenErrorKind::RemoteNotAuthorized,
            Self::Unreachable => LibraryOpenErrorKind::RemoteUnreachable,
            Self::Protocol(_) => LibraryOpenErrorKind::RemoteProtocol,
        }
    }
}

fn protocol(error: impl std::fmt::Display) -> RemoteConnectError {
    RemoteConnectError::Protocol(error.to_string())
}

pub(crate) fn token_error(error: anyhow::Error) -> RemoteConnectError {
    error!("Remote authentication failed: {error:#}");
    if let Some(error) = error.downcast_ref::<reqwest::Error>() {
        if matches!(
            error.status(),
            Some(reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN)
        ) {
            return RemoteConnectError::NotAuthorized;
        }
        if error.is_connect() || error.is_timeout() {
            return RemoteConnectError::Unreachable;
        }
    }
    protocol(error)
}

type RemoteStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub(crate) async fn connect_remote(
    host: &str,
    port: u16,
    config_path: &str,
    validator: CertValidator,
) -> std::result::Result<RemoteStream, RemoteConnectError> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let entries = validator.clone_trusted_entries().await;
        if !entries.iter().any(|(_, hosts)| {
            hosts.iter().any(|candidate| {
                endpoint_host_port(candidate).is_some_and(|(name, trusted_port)| {
                    name == host && trusted_port.is_none_or(|p| p == port)
                })
            })
        }) {
            return Err(RemoteConnectError::UntrustedEndpoint);
        }
        let server_fingerprint = trusted_fingerprint(&validator, host, port)
            .await
            .map_err(protocol)?;
        let alias = get_or_generate_alias(config_path.as_ref())
            .await
            .map_err(protocol)?;
        let (fingerprint, _, private_key) = generate_or_load_certificates(config_path, &alias)
            .await
            .map_err(protocol)?;
        let pinned_config = tls_config(validator, &server_fingerprint);
        let client = reqwest::Client::builder()
            .use_preconfigured_tls(pinned_config.clone())
            .redirect(Policy::none())
            .build()
            .map_err(protocol)?;
        let base = format!("https://{host}:{port}");
        let token = obtain_token(
            &client,
            &base,
            &fingerprint,
            &private_key,
            &server_fingerprint,
        )
        .await
        .map_err(token_error)?;
        let url = format!("wss://{host}:{port}/ws?host={}", encode(host));
        let mut request = url.into_client_request().map_err(protocol)?;
        request.headers_mut().insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {token}").parse().map_err(protocol)?,
        );
        let (stream, _) = connect_async_tls_with_config(
            request,
            None,
            false,
            Some(Connector::Rustls(Arc::new(pinned_config))),
        )
        .await
        .map_err(|error| {
            use tokio_tungstenite::tungstenite::Error;
            error!("WebSocket upgrade failed: {error}");
            match error {
                Error::Http(response) if matches!(response.status().as_u16(), 401 | 403) => {
                    RemoteConnectError::NotAuthorized
                }
                Error::Io(_) | Error::Tls(_) => RemoteConnectError::Unreachable,
                other => protocol(other),
            }
        })?;
        Ok(stream)
    })
    .await
    .map_err(|_| RemoteConnectError::Unreachable)?
}

fn remote_failure_detail(failure: &RemoteRequestFailed) -> String {
    format!(
        "Remote request {} failed: {}",
        failure.request, failure.detail
    )
}

fn classify_remote_loss(close_code: Option<u16>) -> RemoteLossKind {
    match close_code {
        Some(4403) => RemoteLossKind::Revoked,
        Some(1001) => RemoteLossKind::ServerStopped,
        Some(4409) => RemoteLossKind::Detached,
        _ => RemoteLossKind::NetworkLost,
    }
}

pub struct WebSocketDartBridge {
    handlers: HandlerMap,
}

impl Default for WebSocketDartBridge {
    fn default() -> Self {
        WebSocketDartBridge::new()
    }
}

impl WebSocketDartBridge {
    pub fn new() -> Self {
        WebSocketDartBridge {
            handlers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn register_handler<T>(&self, msg_type: &str)
    where
        T: RinfRustSignal + RustSignal + for<'a> Deserialize<'a> + 'static,
    {
        let handler_fn =
            Box::new(
                move |payload: Vec<u8>| match rinf::deserialize::<T>(payload.as_slice()) {
                    Ok(decoded) => {
                        decoded.send_signal_to_dart();
                    }
                    Err(e) => {
                        error!("Failed to decode message: {e}");
                        CrashResponse {
                            detail: format!("Failed to decode message: {e}"),
                        }
                        .send_signal_to_dart();
                    }
                },
            );

        self.handlers
            .lock()
            .await
            .insert(msg_type.to_string(), handler_fn);
    }

    pub async fn handle_message(&self, msg_type: &str, payload: Vec<u8>) {
        if let Some(handler) = self.handlers.lock().await.get(msg_type) {
            handler(payload);
        } else {
            error!("No handler registered for message type in the message bridge: {msg_type}");
        }
    }

    pub async fn run(&mut self, ws_stream: RemoteStream, node: Arc<NodeContext>) -> Result<()> {
        info!("WebSocket connection established");

        let (write, mut read) = ws_stream.split();
        let write = Arc::new(Mutex::new(write));

        let cancel_token: CancellationToken = node.token.child_token();

        let sfx_player = SfxPlayer::new(Some(cancel_token.clone()));
        let sfx_player: Arc<Mutex<SfxPlayer>> = Arc::new(Mutex::new(sfx_player));

        let cancel_token = Arc::new(cancel_token);

        let cancel_token_clone = Arc::clone(&cancel_token);
        let sfx_player_clone = Arc::clone(&sfx_player);
        let handle_event_sfx_play_request = || async move {
            let receiver = SfxPlayRequest::get_dart_signal_receiver();
            loop {
                tokio::select! {
                    _ = cancel_token_clone.cancelled() => {
                        break;
                    }
                    Some(dart_signal) = receiver.recv() => {
                        sfx_player_clone
                        .lock()
                        .await
                        .load(dart_signal.message.path.clone().into());
                    }
                }
            }
        };

        tokio::spawn(handle_event_sfx_play_request());

        for_all_non_local_requests3!(
            forward_event_to_remote,
            self_arc,
            cancel_token.clone(),
            write.clone()
        );

        let handlers = self.handlers.clone();
        let write_clone = Arc::clone(&write);
        let cancel_token_clone = Arc::clone(&cancel_token);
        let message_loop = || async move {
            loop {
                tokio::select! {
                    biased;
                    _ = cancel_token_clone.cancelled() => {
                        info!("Received cancel signal, closing connection");
                        let mut write = write_clone.lock().await;
                        if let Err(e) = write.close().await {
                            error!("Error closing websocket connection: {e}");
                        }
                        break;
                    }

                    message = read.next() => {
                        match message {
                            Some(Ok(msg)) => {
                                if let TungsteniteMessage::Close(frame) = &msg {
                                    let code = frame.as_ref().map(|frame| u16::from(frame.code));
                                    RemoteConnectionLost {
                                        kind: classify_remote_loss(code),
                                        detail: format!("WebSocket closed: {frame:?}"),
                                    }.send_signal_to_dart();
                                    break;
                                }
                                if let TungsteniteMessage::Binary(payload) = msg
                                    && let Some((msg_type, msg_payload, _request_id)) = decode_message(&payload) {
                                        debug!("Received message with type: {msg_type}");
                                        if let Some(handler) = handlers.lock().await.get(&msg_type) {
                                            handler(msg_payload);
                                        } else {
                                            error!("No handler registered for message type while receiving response: {msg_type}");
                                        }
                                    }
                            }
                            Some(Err(e)) => {
                                error!("Error receiving message: {e}");
                                RemoteConnectionLost {
                                    kind: classify_remote_loss(None),
                                    detail: e.to_string(),
                                }.send_signal_to_dart();
                                break;
                            }
                            None => {
                                RemoteConnectionLost {
                                    kind: classify_remote_loss(None),
                                    detail: "WebSocket ended without a close frame".to_string(),
                                }.send_signal_to_dart();
                                break;
                            },
                        }
                    }
                }
            }
            cancel_token_clone.cancel();
        };

        let global_params = GlobalParams {
            library_id: None,
            node: node.clone(),
            library_pool: node.library_pool.clone(),
            library: None,
            ui_library: Arc::new(Mutex::new(None)),
            sync_settings: node.sync_settings.clone(),
            config_path: node.config_path.clone(),
            node_id: node.node_id.clone(),
            main_token: Arc::clone(&cancel_token),
            task_tokens: Arc::new(Mutex::new(TaskTokens {
                scan_token: None,
                scan_id: None,
                analyze_token: None,
                deduplicate_token: None,
            })),
            player: Arc::new(Mutex::new(MockPlayer {})),
            sfx_player,
            scrobbler: Arc::new(Mutex::new(MockScrobblingManager::new())),
            broadcaster: node.broadcaster.clone(),
            device_scanner: node.device_scanner.clone(),
            cert_validator: node.cert_validator.clone(),
            permission_manager: node.permission_manager.clone(),
            server_manager: OnceLock::new(),
            running_mode: RunningMode::Server,
        };

        let global_params = Arc::new(global_params);
        global_params
            .server_manager
            .set(node.manager()?)
            .map_err(|_| anyhow::anyhow!("Library server already initialized"))?;
        node.attach_ui(&global_params).await;

        for_all_local_only_request_pairs2!(listen_local_gui_event, global_params, cancel_token);
        tokio::spawn(message_loop());

        Ok(())
    }
}

pub async fn server_player_loop(
    fsio: Arc<FsIo>,
    url: &str,
    node: Arc<NodeContext>,
    _alias: &str,
) -> Result<()> {
    info!("Media Library Received, initialize the server loop");

    let config_path = node.config_path.as_str();
    let ws_stream = tokio::time::timeout(Duration::from_secs(10), async {
        let cert_validator = Arc::new(node.cert_validator.read().await.clone());
        let hosts = decode_rnsrv_url(url).map_err(protocol)?;
        let entries = cert_validator.clone_trusted_entries().await;
        let hosts: Vec<_> = hosts
            .into_iter()
            .filter(|host| {
                entries.iter().any(|(_, candidates)| {
                    candidates.iter().any(|candidate| {
                        endpoint_host_port(candidate).is_some_and(|(name, port)| {
                            name == *host && port.is_none_or(|p| p == 7863)
                        })
                    })
                })
            })
            .collect();
        if hosts.is_empty() {
            return Err(RemoteConnectError::UntrustedEndpoint);
        }
        let client_config = cert_validator.clone().into_client_config();
        let host = select_best_host(hosts, Arc::new(client_config))
            .await
            .map_err(|error| {
                error!("Host selection failed: {error:#}");
                RemoteConnectError::Unreachable
            })?;
        connect_remote(&host, 7863, config_path, (*cert_validator).clone()).await
    })
    .await
    .map_err(|_| RemoteConnectError::Unreachable)??;

    let _ = fsio;
    {
        info!("Initializing bridge");
        let mut bridge = WebSocketDartBridge::new();

        info!("Registering remote handlers");
        bridge.handlers.lock().await.insert(
            "RemoteRequestFailed".to_owned(),
            Box::new(
                |payload| match rinf::deserialize::<RemoteRequestFailed>(&payload) {
                    Ok(failure) => {
                        let detail = remote_failure_detail(&failure);
                        let kind = failure.kind;
                        RequestFailedResponse {
                            request: failure.request,
                            kind,
                            detail: failure.detail,
                        }
                        .send_signal_to_dart();
                        if kind == RequestErrorKind::Internal {
                            error!("{detail}");
                            CrashResponse { detail }.send_signal_to_dart();
                        } else {
                            warn!("{detail}");
                        }
                    }
                    Err(e) => {
                        error!("Failed to decode message: {e}");
                        CrashResponse {
                            detail: format!("Failed to decode message: {e}"),
                        }
                        .send_signal_to_dart();
                    }
                },
            ),
        );
        for_all_responses!(register_remote_handlers, bridge);
        register_remote_handlers!(
            bridge,
            ScanAudioLibraryProgress,
            ScanAudioLibraryResponse,
            SetMediaLibraryPathResponse,
            AnalyzeAudioLibraryProgress,
            AnalyzeAudioLibraryResponse,
            PlaybackStatus,
            ScrobbleServiceStatusUpdated,
            CrashResponse,
            RealtimeFFT,
            PlaylistUpdate,
            LibraryContentChanged
        );

        bridge.run(ws_stream, node).await?;
    }

    Ok(())
}

#[cfg(test)]
mod loss_tests {
    use super::*;

    #[test]
    fn remote_failure_formats_crash_detail() {
        let failure = RemoteRequestFailed {
            kind: RequestErrorKind::Internal,
            request: "ExampleRequest".into(),
            detail: "outer: inner".into(),
        };
        assert_eq!(
            remote_failure_detail(&failure),
            "Remote request ExampleRequest failed: outer: inner"
        );
    }

    #[test]
    fn classifies_remote_losses() {
        assert_eq!(classify_remote_loss(Some(4403)), RemoteLossKind::Revoked);
        assert_eq!(classify_remote_loss(Some(4409)), RemoteLossKind::Detached);
        assert_eq!(
            classify_remote_loss(Some(1001)),
            RemoteLossKind::ServerStopped
        );
        assert_eq!(classify_remote_loss(None), RemoteLossKind::NetworkLost);
        assert_eq!(
            classify_remote_loss(Some(4000)),
            RemoteLossKind::NetworkLost
        );
    }
}
