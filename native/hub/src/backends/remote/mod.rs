#[macro_use]
mod remote_request;

use crate::node::NodeContext;
use crate::request_error::classify;
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::Duration,
};

use anyhow::{Context, Result};
use futures::stream::SplitSink;
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
    endpoint::endpoint_host_port,
    url::decode_rnsrv_url,
};
use ::fsio::FsIo;
use ::playback::{player::MockPlayer, sfx_player::SfxPlayer};
use ::scrobbling::manager::MockScrobblingManager;

use crate::macros::{RequestClass, request_class};
use crate::register_response_handlers;
use crate::{
    Signal,
    messages::*,
    server::{
        api::fetch_device_info_at_port,
        generate_or_load_certificates, get_or_generate_alias,
        sync_worker::{obtain_token, tls_config, trusted_fingerprint},
    },
    utils::{Broadcaster, GlobalParams, ParamsExtractor, RinfRustSignal, RunningMode, TaskTokens},
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

/// Encode a request addressed to a particular library.
pub fn encode_request(
    type_name: &str,
    payload: &[u8],
    request_id: Uuid,
    library_id: Uuid,
) -> Vec<u8> {
    let type_len = type_name.len() as u8;
    let mut message_data = vec![0, type_len];
    message_data.extend_from_slice(type_name.as_bytes());
    message_data.extend_from_slice(library_id.as_bytes());
    message_data.extend_from_slice(payload);
    message_data.extend_from_slice(request_id.as_bytes());
    message_data
}

/// Decode either request form, returning its optional library address.
pub fn decode_message_with_library_id(
    payload: &[u8],
) -> Option<(String, Vec<u8>, Uuid, Option<Uuid>)> {
    if payload.first() == Some(&0) {
        if payload.len() < 1 + 1 + 1 + 16 + 16 {
            return None;
        }
        let type_len = payload[1] as usize;
        if type_len == 0 || payload.len() < 2 + type_len + 16 + 16 {
            return None;
        }
        let type_start = 2;
        let library_start = type_start + type_len;
        let request_start = payload.len() - 16;
        if library_start + 16 > request_start {
            return None;
        }
        let library_id = Uuid::from_slice(&payload[library_start..library_start + 16]).ok()?;
        let request_id = Uuid::from_slice(&payload[request_start..]).ok()?;
        let msg_type = String::from_utf8_lossy(&payload[type_start..library_start]).to_string();
        let msg_payload = payload[library_start + 16..request_start].to_vec();
        Some((msg_type, msg_payload, request_id, Some(library_id)))
    } else {
        let (name, body, request_id) = decode_message(payload)?;
        Some((name, body, request_id, None))
    }
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

/// A node-owned control connection to a remote renderer.
pub struct RendererLink {
    write: Arc<Mutex<SplitSink<RemoteStream, TungsteniteMessage>>>,
    cancel: CancellationToken,
    reader: tokio::task::JoinHandle<()>,
}

impl RendererLink {
    pub(crate) async fn connect(
        node: Arc<NodeContext>,
        fingerprint: &str,
    ) -> std::result::Result<(Self, String, String), RemoteConnectError> {
        let validator = node.cert_validator.read().await.clone();
        let hosts = validator.get_hosts_for_fingerprint(fingerprint).await;
        if hosts.is_empty() {
            return Err(RemoteConnectError::UntrustedEndpoint);
        }
        let hosts = hosts
            .into_iter()
            .filter_map(|value| endpoint_host_port(&value))
            .map(|(host, port)| format!("https://{host}:{}", port.unwrap_or(7863)))
            .collect::<Vec<_>>();
        if hosts.is_empty() {
            return Err(RemoteConnectError::UntrustedEndpoint);
        }
        let endpoint = select_renderer_host(&node, hosts).await?;
        let parsed = url::Url::parse(&endpoint).map_err(protocol)?;
        let host = parsed
            .host_str()
            .ok_or_else(|| protocol("invalid renderer host"))?;
        let port = parsed.port().unwrap_or(7863);
        let stream = connect_remote(host, port, &node.config_path, validator.clone()).await?;
        let alias = fetch_device_info_at_port(
            host,
            port,
            Arc::new(Arc::new(validator.clone()).into_client_config()),
        )
        .await
        .map(|info| info.alias)
        .map_err(|_| RemoteConnectError::Unreachable)?;
        let link = Self::new(
            stream,
            node.gui_broadcaster.clone(),
            node.token.child_token(),
        )
        .await;
        Ok((link, host.to_owned(), alias))
    }

    async fn new(
        stream: RemoteStream,
        broadcaster: Arc<dyn Broadcaster>,
        cancel: CancellationToken,
    ) -> Self {
        let (write, mut read) = stream.split();
        let write = Arc::new(Mutex::new(write));
        let handlers = Arc::new(Mutex::new(HashMap::<String, MessageHandler>::new()));
        register_renderer_handlers(&handlers, broadcaster).await;
        let reader_handlers = handlers.clone();
        let reader_cancel = cancel.clone();
        let reader = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = reader_cancel.cancelled() => break,
                    message = read.next() => match message {
                        Some(Ok(TungsteniteMessage::Binary(payload))) => {
                            if let Some((name, body, _)) = decode_message(&payload)
                                && let Some(handler) = reader_handlers.lock().await.get(&name)
                            {
                                handler(body);
                            }
                        }
                        Some(Ok(TungsteniteMessage::Close(_))) | None | Some(Err(_)) => break,
                        Some(Ok(_)) => {}
                    }
                }
            }
            reader_cancel.cancel();
        });
        Self {
            write,
            cancel,
            reader,
        }
    }

    /// Forward a serialized control request to the renderer.
    pub async fn forward(&self, name: &str, payload: &[u8]) -> Result<()> {
        let frame = encode_message(name, payload, Some(Uuid::new_v4()));
        self.write
            .lock()
            .await
            .send(TungsteniteMessage::Binary(frame.into()))
            .await
            .context("Could not send control request to renderer")
    }

    pub(crate) fn close(&self) {
        self.cancel.cancel();
    }
}

/// Route a classified control request through the node's attached renderer.
/// Returns `false` when this request remains local.
pub async fn forward_control_request(
    node: &NodeContext,
    name: &str,
    payload: &[u8],
) -> Result<bool> {
    if !is_control_request(name) {
        return Ok(false);
    }
    let Some(link) = node.renderer_link()? else {
        return Ok(false);
    };
    link.forward(name, payload).await?;
    Ok(true)
}

pub fn is_control_request(name: &str) -> bool {
    matches!(request_class(name), Some(RequestClass::Control))
}

impl Drop for RendererLink {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.reader.abort();
    }
}

async fn register_renderer_handlers(handlers: &HandlerMap, broadcaster: Arc<dyn Broadcaster>) {
    let status_broadcaster = broadcaster.clone();
    handlers.lock().await.insert(
        "PlaybackStatus".into(),
        Box::new(
            move |payload| match rinf::deserialize::<PlaybackStatus>(&payload) {
                Ok(message) => status_broadcaster.broadcast(&message),
                Err(error) => error!("Failed to decode renderer status: {error}"),
            },
        ),
    );
    let playlist_broadcaster = broadcaster.clone();
    handlers.lock().await.insert(
        "PlaylistUpdate".into(),
        Box::new(
            move |payload| match rinf::deserialize::<PlaylistUpdate>(&payload) {
                Ok(message) => playlist_broadcaster.broadcast(&message),
                Err(error) => error!("Failed to decode renderer queue: {error}"),
            },
        ),
    );
    handlers.lock().await.insert(
        "RealtimeFFT".into(),
        Box::new(
            move |payload| match rinf::deserialize::<RealtimeFFT>(&payload) {
                Ok(message) => broadcaster.broadcast(&message),
                Err(error) => error!("Failed to decode renderer spectrum: {error}"),
            },
        ),
    );
    for_all_responses!(register_response_handlers, handlers);
    handlers.lock().await.insert(
        "RemoteRequestFailed".into(),
        Box::new(|payload| dispatch_remote_failure(&payload)),
    );
}

#[macro_export]
macro_rules! register_response_handlers {
    ($handlers:expr, $($response:ty),*) => {
        $(
            let handler: MessageHandler = Box::new(|payload| {
                match rinf::deserialize::<$response>(&payload) {
                    Ok(response) => response.send_signal_to_dart(),
                    Err(error) => error!("Failed to decode renderer response: {error}"),
                }
            });
            $handlers.lock().await.insert(stringify!($response).into(), handler);
        )*
    };
}

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
        let server_fingerprint =
            trusted_fingerprint(&validator, host, port)
                .await
                .map_err(|error| {
                    error!("Could not identify trusted renderer at {host}:{port}: {error:#}");
                    protocol(error)
                })?;
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
        .map_err(|error| {
            error!("Could not obtain renderer token at {host}:{port}: {error:#}");
            token_error(error)
        })?;
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

fn dispatch_remote_failure(payload: &[u8]) {
    match rinf::deserialize::<RemoteRequestFailed>(payload) {
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
        Err(error) => {
            error!("Failed to decode message: {error}");
            CrashResponse {
                detail: format!("Failed to decode message: {error}"),
            }
            .send_signal_to_dart();
        }
    }
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
        let endpoint = select_renderer_host(&node, hosts).await?;
        let parsed = url::Url::parse(&endpoint).map_err(protocol)?;
        let host = parsed
            .host_str()
            .ok_or_else(|| protocol("invalid renderer host"))?;
        connect_remote(
            host,
            parsed.port().unwrap_or(7863),
            config_path,
            (*cert_validator).clone(),
        )
        .await
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

async fn select_renderer_host(
    node: &NodeContext,
    hosts: Vec<String>,
) -> std::result::Result<String, RemoteConnectError> {
    let validator = node.cert_validator.read().await.clone();
    let client_config = Arc::new(validator).into_client_config();
    let hosts = hosts
        .into_iter()
        .map(|host| {
            if host.contains("://") {
                host
            } else {
                format!("https://{host}:7863")
            }
        })
        .collect();
    select_best_host(hosts, Arc::new(client_config))
        .await
        .map_err(|error| {
            error!("Host selection failed: {error:#}");
            RemoteConnectError::Unreachable
        })
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

    #[test]
    fn request_frames_round_trip_with_and_without_library_address() {
        let id = Uuid::new_v4();
        let library_id = Uuid::new_v4();
        let legacy = encode_message("FetchMediaFilesRequest", b"legacy", Some(id));
        assert_eq!(
            decode_message_with_library_id(&legacy),
            Some((
                "FetchMediaFilesRequest".into(),
                b"legacy".to_vec(),
                id,
                None,
            ))
        );
        let addressed = encode_request("FetchMediaFilesRequest", b"addressed", id, library_id);
        assert_eq!(
            decode_message_with_library_id(&addressed),
            Some((
                "FetchMediaFilesRequest".into(),
                b"addressed".to_vec(),
                id,
                Some(library_id),
            ))
        );
    }

    #[test]
    fn truncated_addressed_request_frame_is_rejected() {
        let id = Uuid::new_v4();
        let mut frame = encode_request("FetchMediaFilesRequest", b"", id, Uuid::new_v4());
        frame.truncate(2 + "FetchMediaFilesRequest".len() + 8);
        assert_eq!(decode_message_with_library_id(&frame), None);
    }
}
