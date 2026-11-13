use crate::node::{HandoffPlaybackEvent, NodeContext};
use crate::request_error::ExpectedRequestError;
use crate::request_owner::OpenLibrary;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::{
    collections::HashMap,
    sync::{
        Arc, RwLock as StdRwLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use database::library_registry::LibraryRecordId;
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
    macros::{RequestScope, listen_local_gui},
    messages::*,
    rpc::TargetRequest,
    server::{
        api::fetch_device_info_at_port,
        generate_or_load_certificates, get_or_generate_alias,
        sync_worker::{
            SyncTokenStore, obtain_or_reuse_token, peer_base, tls_config, trusted_fingerprint,
        },
    },
    utils::{
        Broadcaster, GlobalParams, RemoteEventScope, RinfRustSignal,
        broadcastable::REMOTE_EVENT_DECLARATIONS,
    },
    visualizer::{ChartItem, VISUALIZER},
};
#[cfg(test)]
use ::discovery::protocol_version::PROTOCOL_VERSION;
use ::discovery::protocol_version::{
    Compatibility, DEVICE_ALIAS_HEADER, PROTOCOL_VERSION_HEADER, ProtocolVersion, compatibility,
};
use ::discovery::{
    client::{CertValidator, select_best_host},
    endpoint::{DEFAULT_DEVICE_PORT, DeviceAddress, device_url},
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
    active_library_id: Arc<StdRwLock<Option<Uuid>>>,
    fingerprint: String,
    alias: String,
    peer_version: ProtocolVersion,
    write: Arc<Mutex<SplitSink<RemoteStream, TungsteniteMessage>>>,
    cancel: CancellationToken,
    locally_detached: Arc<AtomicBool>,
    pending_replies: PendingReplies,
    #[cfg(test)]
    sent_requests: AtomicUsize,
    node: Weak<NodeContext>,
    reader: tokio::task::JoinHandle<()>,
}

impl RendererLink {
    pub(crate) async fn connect(
        node: Arc<NodeContext>,
        fingerprint: &str,
        link_use: RendererLinkUse,
    ) -> std::result::Result<(Self, String, String), RemoteConnectError> {
        Self::connect_with_addresses(node, fingerprint, None, link_use).await
    }

    pub(crate) async fn connect_with_addresses(
        node: Arc<NodeContext>,
        fingerprint: &str,
        addresses: Option<Vec<String>>,
        link_use: RendererLinkUse,
    ) -> std::result::Result<(Self, String, String), RemoteConnectError> {
        let validator = node.cert_validator.read().await.clone();
        let trusted_hosts = validator.get_hosts_for_fingerprint(fingerprint).await;
        if trusted_hosts.is_empty() {
            return Err(RemoteConnectError::UntrustedEndpoint);
        }
        let hosts = addresses.unwrap_or(trusted_hosts.clone());
        if hosts.is_empty() || hosts.iter().any(|host| !trusted_hosts.contains(host)) {
            return Err(RemoteConnectError::UntrustedEndpoint);
        }
        let hosts = hosts
            .into_iter()
            .map(|value| {
                value
                    .parse::<DeviceAddress>()
                    .map_err(protocol)?
                    .url("https", "/")
                    .map(|url| url.to_string())
                    .map_err(protocol)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if hosts.is_empty() {
            return Err(RemoteConnectError::UntrustedEndpoint);
        }
        let endpoint = select_renderer_host(&node, hosts).await?;
        let parsed = url::Url::parse(&endpoint).map_err(protocol)?;
        let host = parsed
            .host_str()
            .ok_or_else(|| protocol("invalid renderer host"))?;
        let address = DeviceAddress::new(host, parsed.port().unwrap_or(DEFAULT_DEVICE_PORT))
            .map_err(protocol)?;
        let host = address.host().to_owned();
        let port = address.port();
        let stream = connect_remote_with_version(
            &host,
            port,
            &node.config_path,
            validator.clone(),
            &node.sync_tokens,
            node.protocol_version(),
        )
        .await?;
        let device_info = fetch_device_info_at_port(
            &host,
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
        let current_library = link
            .call(&GetOpenLibraryRequest { call_id: None })
            .await
            .map_err(protocol)?;
        let library_id = current_library
            .library_id
            .map(|library_id| Uuid::parse_str(&library_id).map_err(protocol))
            .transpose()?;
        link.set_active_library_id(library_id);
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
        let active_library_id = Arc::new(StdRwLock::new(None));
        let (write, mut read) = stream.split();
        let write = Arc::new(Mutex::new(write));
        let handlers = Arc::new(Mutex::new(HashMap::<String, MessageHandler>::new()));
        if link_use == RendererLinkUse::Control {
            register_session_handlers(&handlers, &node, id, active_library_id.clone()).await;
        }
        let reader_handlers = handlers.clone();
        let pending_replies: PendingReplies = Arc::new(Mutex::new(HashMap::new()));
        let reader_pending_replies = pending_replies.clone();
        let reader_cancel = cancel.clone();
        let reader_local_detach = locally_detached.clone();
        let reader_write = write.clone();
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
                            let reply = async {
                                reader_write
                                    .lock()
                                    .await
                                    .send(TungsteniteMessage::Close(frame.clone()))
                                    .await
                            };
                            match tokio::time::timeout(Duration::from_secs(1), reply).await {
                                Ok(Ok(())) => {}
                                Ok(Err(error)) => {
                                    detail = format!(
                                        "Could not reply to renderer close frame: {error}"
                                    );
                                }
                                Err(_) => {
                                    detail = "Timed out replying to renderer close frame".into();
                                }
                            }
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
            active_library_id,
            fingerprint,
            alias,
            peer_version,
            write,
            cancel,
            locally_detached,
            pending_replies,
            #[cfg(test)]
            sent_requests: AtomicUsize::new(0),
            node: Arc::downgrade(&node),
            reader,
        }
    }

    /// Send a catalogue-declared request and decode its declared response.
    pub async fn call<R>(&self, request: &R) -> Result<R::Response>
    where
        R: TargetRequest + DartSignal + Serialize,
        R::Response: DeserializeOwned,
    {
        self.call_addressed(request, None).await
    }

    /// Send a catalogue-declared library request to the named library.
    pub async fn call_for_library<R>(&self, library_id: Uuid, request: &R) -> Result<R::Response>
    where
        R: TargetRequest + DartSignal + Serialize,
        R::Response: DeserializeOwned,
    {
        self.call_addressed(request, Some(library_id)).await
    }

    /// Send a library request to the renderer's currently open library.
    pub async fn call_for_current_library<R>(&self, request: &R) -> Result<R::Response>
    where
        R: TargetRequest + DartSignal + Serialize,
        R::Response: DeserializeOwned,
    {
        let library_id = self
            .active_library_id()
            .ok_or(ExpectedRequestError::Unavailable { what: "library id" })?;
        self.call_addressed(request, Some(library_id)).await
    }

    async fn call_addressed<R>(&self, request: &R, library_id: Option<Uuid>) -> Result<R::Response>
    where
        R: TargetRequest + DartSignal + Serialize,
        R::Response: DeserializeOwned,
    {
        if (R::SCOPE == RequestScope::Library) != library_id.is_some() {
            return Err(ExpectedRequestError::Unavailable {
                what: if R::SCOPE == RequestScope::Library {
                    "library id"
                } else {
                    "library address for non-library request"
                },
            }
            .into());
        }
        let payload = rinf::serialize(request).context("Could not encode renderer request")?;
        let (name, payload) = self.request(R::wire_name(), &payload, library_id).await?;
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
    async fn request(
        &self,
        name: &str,
        payload: &[u8],
        library_id: Option<Uuid>,
    ) -> Result<(String, Vec<u8>)> {
        let request_id = Uuid::new_v4();
        let (sender, receiver) = oneshot::channel();
        self.pending_replies.lock().await.insert(request_id, sender);
        // The reader clears pending replies when it ends; a link that already ended would
        // otherwise leave this request waiting for the timeout.
        if self.cancel.is_cancelled() {
            self.pending_replies.lock().await.remove(&request_id);
            return Err(ExpectedRequestError::Unavailable { what: "renderer" }.into());
        }
        let frame = match library_id {
            Some(library_id) => encode_request(name, payload, request_id, library_id),
            None => encode_message(name, payload, Some(request_id)),
        };
        let send = async {
            self.write
                .lock()
                .await
                .send(TungsteniteMessage::Binary(frame.into()))
                .await
        };
        let send_result = tokio::time::timeout(Duration::from_secs(10), send).await;
        let send_error = match send_result {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(format!("Could not send request to renderer: {error}")),
            Err(_) => Some("Timed out sending request to renderer".to_owned()),
        };
        if let Some(detail) = send_error {
            self.pending_replies.lock().await.remove(&request_id);
            self.report_send_failure(detail).await;
            return Err(ExpectedRequestError::Unavailable { what: "renderer" }.into());
        }
        #[cfg(test)]
        self.sent_requests.fetch_add(1, Ordering::Relaxed);
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

    pub(crate) fn active_library_id(&self) -> Option<Uuid> {
        self.active_library_id.read().ok().and_then(|value| *value)
    }

    #[cfg(test)]
    pub(crate) fn sent_request_count(&self) -> usize {
        self.sent_requests.load(Ordering::Relaxed)
    }

    fn set_active_library_id(&self, library_id: Option<Uuid>) {
        if let Ok(mut active_library_id) = self.active_library_id.write() {
            *active_library_id = library_id;
        }
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
async fn register_session_handlers(
    handlers: &HandlerMap,
    node: &Arc<NodeContext>,
    id: Uuid,
    active_library_id: Arc<StdRwLock<Option<Uuid>>>,
) {
    for declaration in REMOTE_EVENT_DECLARATIONS {
        let weak_node = Arc::downgrade(node);
        let scope = declaration.scope;
        let decode = declaration.decode;
        let event_name = declaration.name;
        let event_library_id = active_library_id.clone();
        handlers.lock().await.insert(
            declaration.name.into(),
            Box::new(move |payload| {
                let Some(node) = weak_node.upgrade() else {
                    return;
                };
                let current_link = node.renderer_link().ok().flatten();
                let active = current_link.as_ref().is_some_and(|link| link.id() == id);
                if event_name == "RequestCapabilities" {
                    match rinf::deserialize::<RequestCapabilities>(&payload) {
                        Ok(capabilities) => {
                            let library_id = capabilities
                                .library_id
                                .as_deref()
                                .map(Uuid::parse_str)
                                .transpose();
                            match library_id {
                                Ok(library_id) => {
                                    if let Ok(mut active) = event_library_id.write() {
                                        *active = library_id;
                                    }
                                    if active {
                                        node.gui_broadcaster.broadcast(&capabilities);
                                    }
                                }
                                Err(error) => {
                                    error!("Renderer published an invalid library id: {error}");
                                }
                            }
                        }
                        Err(error) => error!("Failed to decode renderer capabilities: {error}"),
                    }
                    return;
                }
                if !active
                    || (scope == RemoteEventScope::Library
                        && !matches!(node.current_library(), Ok(OpenLibrary::RunePeer { .. })))
                {
                    return;
                }
                match decode(&payload) {
                    Ok(message) => {
                        if let Some(status) = message.as_any().downcast_ref::<PlaybackStatus>() {
                            node.publish_handoff_playback_event(HandoffPlaybackEvent::Status(
                                Box::new(status.clone()),
                            ));
                        } else if let Some(playlist) =
                            message.as_any().downcast_ref::<PlaylistUpdate>()
                        {
                            node.publish_handoff_playback_event(HandoffPlaybackEvent::Playlist(
                                playlist.items.clone(),
                            ));
                        }
                        update_visualizer_feed(message.as_ref());
                        node.gui_broadcaster.broadcast(message.as_ref());
                    }
                    Err(error) => error!("Failed to decode renderer event: {error}"),
                }
            }),
        );
    }
}

pub(crate) fn update_visualizer_feed(message: &dyn RinfRustSignal) {
    if let Some(spectrum) = message.as_any().downcast_ref::<RealtimeFFT>() {
        VISUALIZER.set_feed_spectrum(&spectrum.value);
        return;
    }
    if let Some(status) = message.as_any().downcast_ref::<PlaybackStatus>() {
        let (item, position, playing) = feed_playback_args(status);
        VISUALIZER.set_feed_playback(item, position, playing);
        return;
    }
    if let Some(chart) = message.as_any().downcast_ref::<VisualizerFeedChart>() {
        if let Err(error) = VISUALIZER.set_feed_chart(chart_item(&chart.item), &chart.chart) {
            log::warn!("Could not use the renderer's visualizer chart: {error:#}");
        }
        return;
    }
    if let Some(covers) = message.as_any().downcast_ref::<VisualizerFeedCovers>()
        && let Err(error) = VISUALIZER.set_feed_covers(
            &chart_item(&covers.item),
            covers.playing.clone(),
            covers.random.clone(),
        )
    {
        log::warn!("Could not use the renderer's visualizer covers: {error:#}");
    }
}

fn feed_playback_args(status: &PlaybackStatus) -> (Option<ChartItem>, Duration, bool) {
    (
        status.item_request.as_ref().map(chart_item),
        Duration::from_secs_f64(f64::from(status.progress_seconds.max(0.0))),
        status.state == PlaybackState::Playing,
    )
}

/// Deliver one Rune wire frame from either a Rune renderer or a declared Subsonic feed.
pub(crate) fn deliver_visualizer_feed_frame(
    payload: &[u8],
    broadcaster: &dyn Broadcaster,
    mut on_playback_status: impl FnMut(&PlaybackStatus),
) -> Result<()> {
    let (name, body, _) = decode_message(payload)
        .ok_or_else(|| anyhow::anyhow!("Malformed visualizer feed frame"))?;
    if !matches!(
        name.as_str(),
        "PlaybackStatus" | "VisualizerFeedChart" | "VisualizerFeedCovers" | "RealtimeFFT"
    ) {
        return Ok(());
    }
    let declaration = REMOTE_EVENT_DECLARATIONS
        .iter()
        .find(|declaration| declaration.name == name)
        .ok_or_else(|| anyhow::anyhow!("Undeclared visualizer feed message"))?;
    let message = (declaration.decode)(&body)?;
    update_visualizer_feed(message.as_ref());
    if let Some(status) = message.as_any().downcast_ref::<PlaybackStatus>() {
        on_playback_status(status);
    } else if message.as_any().is::<RealtimeFFT>() {
        broadcaster.broadcast(message.as_ref());
    }
    Ok(())
}

impl From<PlayingItemRef> for ChartItem {
    fn from(item: PlayingItemRef) -> Self {
        match item {
            PlayingItemRef::LibraryTrack { library_id, track } => {
                ChartItem::LibraryTrack { library_id, track }
            }
            PlayingItemRef::IndependentFile { path } => ChartItem::IndependentFile { path },
        }
    }
}

fn chart_item(item: &PlayingItemRef) -> ChartItem {
    item.clone().into()
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
                candidate
                    .parse::<DeviceAddress>()
                    .is_ok_and(|address| address.host() == host && address.port() == port)
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
        let base = peer_base(host, port).map_err(protocol)?;
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
        let url = device_url(host, port, "wss", &format!("/ws?host={}", encode(host)))
            .map_err(protocol)?
            .to_string();
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

/// Open a trusted peer's current UI library by stable identity and its trusted addresses.
pub(crate) async fn open_remote_library_for_fingerprint(
    node: Arc<NodeContext>,
    fingerprint: &str,
    addresses: Vec<String>,
    record_id: Option<LibraryRecordId>,
) -> Result<(Arc<GlobalParams>, u64)> {
    if addresses.is_empty() {
        return Err(RemoteConnectError::UntrustedEndpoint.into());
    }
    let (link, _, alias) = RendererLink::connect_with_addresses(
        node.clone(),
        fingerprint,
        Some(addresses),
        RendererLinkUse::Control,
    )
    .await?;
    link.call(&RefreshSessionRequest { call_id: None }).await?;
    let mut params = node.params_for_ui()?;
    params.library_id = link.active_library_id();
    let params = Arc::new(params);
    let generation = node
        .attach_remote_ui(&params, fingerprint.to_owned(), record_id)
        .await?;
    let previous_player = node.current_player_snapshot()?;
    node.install_renderer_link(link, alias)?;
    node.transition_current_player(previous_player, node.current_player()?)
        .await?;
    listen_local_gui(&params, &params.main_token);
    Ok((params, generation))
}

pub(crate) async fn select_renderer_host(
    node: &NodeContext,
    hosts: Vec<String>,
) -> std::result::Result<String, RemoteConnectError> {
    let validator = node.cert_validator.read().await.clone();
    let client_config = Arc::new(validator).into_client_config();
    let hosts = hosts
        .into_iter()
        .map(|host| -> std::result::Result<String, RemoteConnectError> {
            if host.contains("://") {
                Ok(host)
            } else {
                let address = host.parse::<DeviceAddress>().map_err(protocol)?;
                address
                    .url("https", "/")
                    .map(|url| url.to_string())
                    .map_err(protocol)
            }
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
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
    fn rune_playing_status_keeps_the_visualizer_feed_playing() {
        let status = PlaybackStatus {
            session_id: "renderer".into(),
            state: PlaybackState::Playing,
            progress_seconds: 12.0,
            progress_percentage: 0.2,
            artist: None,
            album: None,
            title: None,
            duration: 60.0,
            index: Some(0),
            item: None,
            item_request: None,
            playback_mode: 0,
            ready: true,
            cover_art_path: None,
            lib_path: String::new(),
        };

        let (_, position, playing) = feed_playback_args(&status);
        assert_eq!(position, Duration::from_secs(12));
        assert!(playing);
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
