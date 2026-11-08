use crate::node::NodeContext;
use crate::request_error::ExpectedRequestError;
use std::{
    collections::HashMap,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use log::{error, warn};
use reqwest::redirect::Policy;
use rinf::DartSignal;
use serde::{Serialize, de::DeserializeOwned};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, oneshot};
use tokio_tungstenite::{
    Connector, connect_async_tls_with_config,
    tungstenite::{
        Error as TungsteniteError, client::IntoClientRequest,
        protocol::Message as TungsteniteMessage,
    },
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_util::sync::CancellationToken;
use urlencoding::encode;
use uuid::Uuid;

use crate::{
    messages::*,
    server::{
        api::fetch_device_info_at_port,
        generate_or_load_certificates, get_or_generate_alias,
        sync_worker::{
            SyncTokenStore, obtain_or_reuse_token, peer_base, tls_config, trusted_fingerprint,
        },
    },
    utils::{GlobalParams, RemoteEventScope, broadcastable::REMOTE_EVENT_DECLARATIONS},
};
#[cfg(test)]
use ::discovery::protocol_version::PROTOCOL_VERSION;
use ::discovery::protocol_version::{
    Compatibility, DEVICE_ALIAS_HEADER, PROTOCOL_VERSION_HEADER, ProtocolVersion, compatibility,
};
use ::discovery::{
    client::{CertValidator, select_best_host},
    endpoint::endpoint_host_port,
    url::decode_rnsrv_url,
};

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
    #[error("remote protocol version {peer:?} is incompatible with local version {local}")]
    PeerNeedsUpdate {
        local: ProtocolVersion,
        peer: Option<ProtocolVersion>,
        update_peer: bool,
        peer_alias: Option<String>,
    },
}

impl RemoteConnectError {
    pub fn kind(&self) -> LibraryOpenErrorKind {
        match self {
            Self::UntrustedEndpoint => LibraryOpenErrorKind::RemoteUntrusted,
            Self::NotAuthorized => LibraryOpenErrorKind::RemoteNotAuthorized,
            Self::Unreachable => LibraryOpenErrorKind::RemoteUnreachable,
            Self::Protocol(_) => LibraryOpenErrorKind::RemoteProtocol,
            Self::PeerNeedsUpdate { .. } => LibraryOpenErrorKind::RemoteProtocol,
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

pub(crate) type RemoteStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type PendingReplies = Arc<Mutex<HashMap<Uuid, oneshot::Sender<Result<(String, Vec<u8>)>>>>>;

/// A node-owned control connection to a remote renderer.
/// What a renderer link is for: controlling the renderer shows its session
/// in this device's UI; a one-off cast only sends requests and reads replies.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RendererLinkUse {
    Control,
    Cast,
}

pub struct RendererLink {
    id: Uuid,
    fingerprint: String,
    alias: String,
    peer_version: ProtocolVersion,
    write: Arc<Mutex<SplitSink<RemoteStream, TungsteniteMessage>>>,
    cancel: CancellationToken,
    locally_detached: Arc<AtomicBool>,
    pending_replies: PendingReplies,
    node: Weak<NodeContext>,
    reader: tokio::task::JoinHandle<()>,
}

impl RendererLink {
    pub(crate) async fn connect(
        node: Arc<NodeContext>,
        fingerprint: &str,
        link_use: RendererLinkUse,
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
        let stream = connect_remote_with_version(
            host,
            port,
            &node.config_path,
            validator.clone(),
            &node.sync_tokens,
            node.protocol_version(),
        )
        .await?;
        let device_info = fetch_device_info_at_port(
            host,
            port,
            Arc::new(Arc::new(validator.clone()).into_client_config()),
        )
        .await
        .map_err(|_| RemoteConnectError::Unreachable)?;
        let alias = device_info.alias;
        let Some(peer_version) = device_info.protocol_version else {
            return Err(RemoteConnectError::PeerNeedsUpdate {
                local: node.protocol_version(),
                peer: None,
                update_peer: true,
                peer_alias: Some(alias.clone()),
            });
        };
        let link = Self::new(
            stream,
            node.clone(),
            fingerprint.to_owned(),
            alias.clone(),
            peer_version,
            link_use,
        )
        .await;
        Ok((link, host.to_owned(), alias))
    }

    pub(crate) async fn new(
        stream: RemoteStream,
        node: Arc<NodeContext>,
        fingerprint: String,
        alias: String,
        peer_version: ProtocolVersion,
        link_use: RendererLinkUse,
    ) -> Self {
        let id = Uuid::new_v4();
        let cancel = node.token.child_token();
        let locally_detached = Arc::new(AtomicBool::new(false));
        let (write, mut read) = stream.split();
        let write = Arc::new(Mutex::new(write));
        let handlers = Arc::new(Mutex::new(HashMap::<String, MessageHandler>::new()));
        if link_use == RendererLinkUse::Control {
            register_session_handlers(&handlers, &node, id).await;
        }
        let reader_handlers = handlers.clone();
        let pending_replies: PendingReplies = Arc::new(Mutex::new(HashMap::new()));
        let reader_pending_replies = pending_replies.clone();
        let reader_cancel = cancel.clone();
        let reader_local_detach = locally_detached.clone();
        let weak_node = Arc::downgrade(&node);
        let reader = tokio::spawn(async move {
            let mut close_code = None;
            let mut detail = "Renderer connection ended".to_owned();
            loop {
                tokio::select! {
                    _ = reader_cancel.cancelled() => break,
                    message = read.next() => match message {
                        Some(Ok(TungsteniteMessage::Binary(payload))) => {
                            if let Some((name, body, request_id)) = decode_message(&payload) {
                                let pending = reader_pending_replies.lock().await.remove(&request_id);
                                if let Some(reply) = pending {
                                    let _ = reply.send(renderer_reply(name, body));
                                    continue;
                                }
                                if let Some(handler) = reader_handlers.lock().await.get(&name) {
                                    handler(body);
                                }
                            }
                        }
                        Some(Ok(TungsteniteMessage::Close(frame))) => {
                            close_code = frame.as_ref().map(|frame| u16::from(frame.code));
                            detail = format!("Renderer closed the connection: {frame:?}");
                            break;
                        }
                        None => { detail = "Renderer connection ended without a close frame".into(); break; }
                        Some(Err(error)) => { detail = format!("Renderer connection failed: {error}"); break; }
                        Some(Ok(_)) => {}
                    }
                }
            }
            reader_cancel.cancel();
            reader_pending_replies.lock().await.clear();
            if !reader_local_detach.load(Ordering::Acquire)
                && let Some(node) = weak_node.upgrade()
            {
                warn!("Renderer link closed (code {close_code:?}): {detail}");
                node.renderer_connection_lost(id, classify_remote_loss(close_code), detail)
                    .await;
            }
        });
        Self {
            id,
            fingerprint,
            alias,
            peer_version,
            write,
            cancel,
            locally_detached,
            pending_replies,
            node: Arc::downgrade(&node),
            reader,
        }
    }

    /// Send a catalogue-declared request and decode its declared response.
    pub async fn call<R>(&self, request: &R) -> Result<R::Response>
    where
        R: crate::TypedRequest + DartSignal + Serialize,
        R::Response: DeserializeOwned,
    {
        let payload = rinf::serialize(request).context("Could not encode renderer request")?;
        let (name, payload) = self.request(R::wire_name(), &payload).await?;
        if name != R::response_name() {
            return Err(ExpectedRequestError::UnexpectedResponse {
                expected: R::response_name().to_owned(),
                actual: name,
            }
            .into());
        }
        rinf::deserialize::<R::Response>(&payload).context("Could not decode renderer response")
    }

    /// Send a request to the renderer and wait for its matching response frame.
    async fn request(&self, name: &str, payload: &[u8]) -> Result<(String, Vec<u8>)> {
        let request_id = Uuid::new_v4();
        let (sender, receiver) = oneshot::channel();
        self.pending_replies.lock().await.insert(request_id, sender);
        // The reader clears pending replies when it ends; a link that already ended would
        // otherwise leave this request waiting for the timeout.
        if self.cancel.is_cancelled() {
            self.pending_replies.lock().await.remove(&request_id);
            return Err(ExpectedRequestError::Unavailable { what: "renderer" }.into());
        }
        let frame = encode_message(name, payload, Some(request_id));
        if let Err(error) = self
            .write
            .lock()
            .await
            .send(TungsteniteMessage::Binary(frame.into()))
            .await
        {
            self.pending_replies.lock().await.remove(&request_id);
            self.report_send_failure(format!("Could not send request to renderer: {error}"))
                .await;
            return Err(ExpectedRequestError::Unavailable { what: "renderer" }.into());
        }
        match tokio::time::timeout(Duration::from_secs(30), receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) | Err(_) => {
                self.pending_replies.lock().await.remove(&request_id);
                Err(ExpectedRequestError::Unavailable { what: "renderer" }.into())
            }
        }
    }

    async fn report_send_failure(&self, detail: String) {
        if let Some(node) = self.node.upgrade() {
            warn!("{detail}");
            node.renderer_link_lost(self.id).await;
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.cancel.is_cancelled()
    }

    pub(crate) fn id(&self) -> Uuid {
        self.id
    }

    pub(crate) fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub(crate) fn alias(&self) -> &str {
        &self.alias
    }

    pub(crate) fn peer_version(&self) -> ProtocolVersion {
        self.peer_version
    }

    pub(crate) fn close(&self) {
        self.locally_detached.store(true, Ordering::Release);
        self.cancel.cancel();
    }
}

/// Turn a reply frame the hub waited for into its typed outcome.
fn renderer_reply(name: String, body: Vec<u8>) -> Result<(String, Vec<u8>)> {
    if name != "RemoteRequestFailed" {
        return Ok((name, body));
    }
    let failure = rinf::deserialize::<RemoteRequestFailed>(&body)
        .map_err(|error| anyhow::anyhow!("Could not decode renderer failure: {error}"))?;
    Err(ExpectedRequestError::Remote {
        kind: failure.kind,
        detail: failure.detail,
        cause: failure.cause,
    }
    .into())
}

impl Drop for RendererLink {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.reader.abort();
    }
}

/// Only the active control link may publish events. Library events belong to @RR;
/// Play on continues to show this device's library.
async fn register_session_handlers(handlers: &HandlerMap, node: &Arc<NodeContext>, id: Uuid) {
    for declaration in REMOTE_EVENT_DECLARATIONS {
        let weak_node = Arc::downgrade(node);
        let scope = declaration.scope;
        let decode = declaration.decode;
        handlers.lock().await.insert(
            declaration.name.into(),
            Box::new(move |payload| {
                let Some(node) = weak_node.upgrade() else {
                    return;
                };
                if !node
                    .renderer_link()
                    .ok()
                    .flatten()
                    .is_some_and(|link| link.id() == id)
                    || (scope == RemoteEventScope::Library
                        && !node.remote_ui_library().unwrap_or(false))
                {
                    return;
                }
                match decode(&payload) {
                    Ok(message) => node.gui_broadcaster.broadcast(message.as_ref()),
                    Err(error) => error!("Failed to decode renderer event: {error}"),
                }
            }),
        );
    }
}

#[cfg(test)]
pub(crate) async fn connect_remote(
    host: &str,
    port: u16,
    config_path: &str,
    validator: CertValidator,
    sync_tokens: &SyncTokenStore,
) -> std::result::Result<RemoteStream, RemoteConnectError> {
    connect_remote_with_version(
        host,
        port,
        config_path,
        validator,
        sync_tokens,
        PROTOCOL_VERSION,
    )
    .await
}

async fn connect_remote_with_version(
    host: &str,
    port: u16,
    config_path: &str,
    validator: CertValidator,
    sync_tokens: &SyncTokenStore,
    local_version: ProtocolVersion,
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
        let base = peer_base(host, port);
        let token = obtain_or_reuse_token(
            sync_tokens,
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
        let mut request = url.as_str().into_client_request().map_err(protocol)?;
        request.headers_mut().insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {token}").parse().map_err(protocol)?,
        );
        request.headers_mut().insert(
            PROTOCOL_VERSION_HEADER,
            local_version.to_string().parse().map_err(protocol)?,
        );
        let result = connect_async_tls_with_config(
            request,
            None,
            false,
            Some(Connector::Rustls(Arc::new(pinned_config.clone()))),
        )
        .await;
        let (stream, response) = match result {
            Ok(connection) => connection,
            Err(TungsteniteError::Http(response))
                if response.status() == reqwest::StatusCode::UNAUTHORIZED =>
            {
                sync_tokens.invalidate(&server_fingerprint, &base);
                let fresh_token = obtain_or_reuse_token(
                    sync_tokens,
                    &client,
                    &base,
                    &fingerprint,
                    &private_key,
                    &server_fingerprint,
                )
                .await
                .map_err(token_error)?;
                let mut retry = url.as_str().into_client_request().map_err(protocol)?;
                retry.headers_mut().insert(
                    reqwest::header::AUTHORIZATION,
                    format!("Bearer {fresh_token}").parse().map_err(protocol)?,
                );
                retry.headers_mut().insert(
                    PROTOCOL_VERSION_HEADER,
                    local_version.to_string().parse().map_err(protocol)?,
                );
                connect_async_tls_with_config(
                    retry,
                    None,
                    false,
                    Some(Connector::Rustls(Arc::new(pinned_config))),
                )
                .await
                .map_err(|error| map_websocket_connect_error(error, local_version))?
            }
            Err(error) => return Err(map_websocket_connect_error(error, local_version)),
        };
        let peer_version = response
            .headers()
            .get(PROTOCOL_VERSION_HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| ProtocolVersion::parse(value).ok());
        if !peer_version.is_some_and(|peer| {
            matches!(
                compatibility(local_version, peer),
                Compatibility::Compatible { .. }
            )
        }) {
            return Err(incompatible_peer(local_version, peer_version, None));
        }
        Ok(stream)
    })
    .await
    .map_err(|_| RemoteConnectError::Unreachable)?
}

fn map_websocket_connect_error(
    error: TungsteniteError,
    local_version: ProtocolVersion,
) -> RemoteConnectError {
    error!("WebSocket upgrade failed: {error}");
    match error {
        TungsteniteError::Http(response)
            if response.status() == reqwest::StatusCode::UPGRADE_REQUIRED =>
        {
            let peer = response
                .headers()
                .get(PROTOCOL_VERSION_HEADER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| ProtocolVersion::parse(value).ok());
            let alias = response
                .headers()
                .get(DEVICE_ALIAS_HEADER)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            incompatible_peer(local_version, peer, alias)
        }
        TungsteniteError::Http(response) if matches!(response.status().as_u16(), 401 | 403) => {
            RemoteConnectError::NotAuthorized
        }
        TungsteniteError::Io(_) | TungsteniteError::Tls(_) => RemoteConnectError::Unreachable,
        other => protocol(other),
    }
}

fn incompatible_peer(
    local: ProtocolVersion,
    peer: Option<ProtocolVersion>,
    peer_alias: Option<String>,
) -> RemoteConnectError {
    let update_peer = peer.is_none_or(|peer| match compatibility(local, peer) {
        Compatibility::PeerOlder => true,
        Compatibility::PeerNewer => false,
        Compatibility::PreReleaseMismatch | Compatibility::Compatible { .. } => true,
    });
    RemoteConnectError::PeerNeedsUpdate {
        local,
        peer,
        update_peer,
        peer_alias,
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

/// Open the peer's current UI library using the same target as its session.
pub(crate) async fn open_remote_library(
    node: Arc<NodeContext>,
    path: &str,
) -> Result<Arc<GlobalParams>> {
    let url = path.strip_prefix("@RR|").unwrap_or(path);
    let hosts = if url.starts_with("rnsrv://") {
        decode_rnsrv_url(url)
            .map_err(protocol)?
            .into_iter()
            .map(|host| (host, 7863))
            .collect::<Vec<_>>()
    } else {
        let parsed = url::Url::parse(url).map_err(protocol)?;
        if !matches!(parsed.scheme(), "ws" | "wss") {
            return Err(protocol("invalid remote library scheme").into());
        }
        vec![(
            parsed
                .host_str()
                .ok_or_else(|| protocol("invalid remote host"))?
                .to_owned(),
            parsed.port().unwrap_or(7863),
        )]
    };
    let entries = node
        .cert_validator
        .read()
        .await
        .clone_trusted_entries()
        .await;
    let fingerprint = entries
        .iter()
        .find(|(_, candidates)| {
            candidates.iter().any(|candidate| {
                endpoint_host_port(candidate).is_some_and(|(name, port)| {
                    hosts.iter().any(|(host, wanted_port)| {
                        name == *host && port.is_none_or(|port| port == *wanted_port)
                    })
                })
            })
        })
        .map(|(fingerprint, _)| fingerprint.clone())
        .ok_or(RemoteConnectError::UntrustedEndpoint)?;
    let (link, _, alias) =
        RendererLink::connect(node.clone(), &fingerprint, RendererLinkUse::Control).await?;
    let params = Arc::new(node.params_for_ui()?);
    node.attach_remote_ui(&params, fingerprint).await?;
    node.install_renderer_link(link, alias)?;
    node.remote_ui_renderer()?
        .call(&RefreshSessionRequest {})
        .await?;
    node.session()?.player.lock().await.pause();
    Ok(params)
}

pub(crate) async fn select_renderer_host(
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
    fn remote_failure_keeps_typed_detail() {
        let failure = RemoteRequestFailed {
            kind: RequestErrorKind::Internal,
            request: "ExampleRequest".into(),
            call_id: None,
            detail: "outer: inner".into(),
            cause: None,
        };
        let error = renderer_reply(
            "RemoteRequestFailed".into(),
            rinf::serialize(&failure).unwrap(),
        )
        .unwrap_err();
        assert!(matches!(error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Remote { kind: RequestErrorKind::Internal, detail, .. }) if detail == "outer: inner"));
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
