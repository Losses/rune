use crate::access_policy::Action;
use discovery::access_policy::{AccessPolicy, PeerIdentity};
use std::time::Duration;
use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    Extension,
    extract::{
        ConnectInfo, Query, State,
        ws::{CloseFrame, Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    http::{HeaderMap, HeaderName, HeaderValue},
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use tokio::{
    sync::{Mutex, broadcast::Receiver, broadcast::error::RecvError, mpsc},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::{
    Session,
    backends::remote::{decode_message, decode_message_with_library_id, encode_message},
    macros::capability_snapshot,
    messages::{RemoteRequestFailed, RequestCapabilities, RequestCapability},
    node::NodeContext,
    request_error::{FailedCall, classify, request_failure_cause},
    server::{ServerState, sync_auth::AuthenticatedFingerprint},
};
use discovery::protocol_version::{
    Compatibility, DEVICE_ALIAS_HEADER, PROTOCOL_VERSION, PROTOCOL_VERSION_HEADER, ProtocolVersion,
    compatibility,
};
use discovery::server::{User, UserStatus};

async fn capabilities_for_peer(
    node: &NodeContext,
    peer: &PeerIdentity,
) -> Option<RequestCapabilities> {
    let capabilities = capability_snapshot(node).ok()?;
    let library_id = node
        .current_ui()
        .await
        .ok()
        .and_then(|params| params.library_id);
    Some(node.capabilities_for_peer(capabilities, peer, library_id))
}

fn tailor_capability_message(
    message: Vec<u8>,
    capabilities: Option<&RequestCapabilities>,
) -> Vec<u8> {
    let Some(capabilities) = capabilities else {
        return message;
    };
    let Some((name, _, request_id)) = decode_message(&message) else {
        return message;
    };
    if name != "RequestCapabilities" {
        return message;
    }
    match rinf::serialize(capabilities) {
        Ok(payload) => encode_message(&name, &payload, Some(request_id)),
        Err(error) => {
            error!("Could not encode controller capabilities: {error}");
            message
        }
    }
}

fn is_capability_message(message: &[u8]) -> bool {
    decode_message(message).is_some_and(|(name, _, _)| name == "RequestCapabilities")
}

pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    Extension(AuthenticatedFingerprint(fingerprint)): Extension<AuthenticatedFingerprint>,
    Query(params): Query<HashMap<String, String>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
) -> Response {
    let (local_version, local_alias) = {
        let info = state.discovery_device_info.read().await;
        (
            info.protocol_version.unwrap_or(PROTOCOL_VERSION),
            info.alias.clone(),
        )
    };
    let version_header = HeaderName::from_static(PROTOCOL_VERSION_HEADER);
    let mut response_headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(&local_version.to_string()) {
        response_headers.insert(version_header.clone(), value);
    }
    // The client names this device in its "needs update" message.
    if let Ok(alias) = HeaderValue::from_str(&local_alias) {
        response_headers.insert(HeaderName::from_static(DEVICE_ALIAS_HEADER), alias);
    }
    let peer_version = headers
        .get(&version_header)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| ProtocolVersion::parse(value).ok());
    let host = params
        .get("host")
        .cloned()
        .unwrap_or("127.0.0.1".to_owned());
    let user = state
        .permission_manager
        .read()
        .await
        .verify_by_fingerprint(&fingerprint)
        .await
        .filter(|user| user.status == UserStatus::Approved);
    match user {
        Some(user) => {
            let compatible = peer_version.is_some_and(|version| {
                matches!(
                    compatibility(local_version, version),
                    Compatibility::Compatible { .. }
                )
            });
            if !compatible {
                return (StatusCode::UPGRADE_REQUIRED, response_headers).into_response();
            }
            info!("Connection authorized for {} @ {}", user.alias, addr);
            let host = format!("https://{host}:7863");
            let broadcast_rx = state.websocket_service.broadcast_tx.subscribe();
            (
                response_headers,
                ws.on_upgrade(move |socket| handle_socket(socket, state, user, host, broadcast_rx)),
            )
                .into_response()
        }
        None => {
            warn!("Unauthorized connection attempt from {}", addr);
            StatusCode::UNAUTHORIZED.into_response()
        }
    }
}

pub async fn handle_socket(
    socket: WebSocket,
    state: Arc<ServerState>,
    user: User,
    host: String,
    mut broadcast_rx: Receiver<Vec<u8>>,
) {
    let mut changes = state.permission_manager.read().await.changes();
    let Some(node) = state.node.upgrade() else {
        return;
    };
    let Ok(session) = node.session() else {
        return;
    };
    let policy = node.access_policy.clone();
    let controller_guard = policy
        .check(
            &PeerIdentity {
                fingerprint: user.fingerprint.clone(),
                public_key: user.public_key.clone(),
            },
            &Action::Control,
        )
        .is_ok()
        .then(|| session.attach_controller(user.fingerprint.clone(), user.alias.clone()));
    let controller_cancel = controller_guard
        .as_ref()
        .map(|guard| guard.cancellation_token());
    let is_controller = controller_cancel.is_some();
    drop(node);
    let peer = PeerIdentity {
        fingerprint: user.fingerprint.clone(),
        public_key: user.public_key.clone(),
    };
    let permission_manager = state.permission_manager.clone();
    let session_cancel = state.session_cancel.clone();
    let session_fingerprint = user.fingerprint.clone();
    let controller_watch_cancel = CancellationToken::new();
    let controller_watch = {
        let policy = policy.clone();
        let peer = peer.clone();
        let cancel = controller_watch_cancel.clone();
        let mut changes = policy.changes();
        tokio::spawn(async move {
            let Some(guard) = controller_guard else {
                return;
            };
            loop {
                if policy.check(&peer, &Action::Control).is_err() {
                    break;
                }
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    result = changes.changed() => if result.is_err() { break; },
                }
            }
            drop(guard);
        })
    };
    let (mut sender, receiver) = socket.split();
    let receiver = Arc::new(Mutex::new(receiver));
    let (tx, mut rx) = mpsc::channel(32);

    let alias = user.alias.clone();
    let fingerprint = user.fingerprint.clone();

    info!("[{alias}] WebSocket connection established");

    // Clone alias for send_task
    let send_task_alias = alias.clone();
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Err(e) = sender.send(msg).await {
                error!("[{send_task_alias}] Failed to send message: {e}");
                break;
            }
        }
    });

    // Clone alias for incoming task
    let incoming_tx = tx.clone();
    let incoming_alias = alias.clone();
    let incoming_public_key = peer.public_key.clone();
    let incoming_receiver = receiver.clone();
    let outgoing_node = state.node.clone();
    enum IncomingEnd {
        PeerClose(Option<CloseFrame>),
        Ended,
    }
    let incoming = async move {
        loop {
            let message = incoming_receiver.lock().await.next().await;
            let msg = match message {
                Some(Ok(msg)) => msg,
                Some(Err(error)) => {
                    warn!("[{incoming_alias}] WebSocket receive failed: {error}");
                    break;
                }
                None => break,
            };
            match msg {
                WsMessage::Close(frame) => {
                    return IncomingEnd::PeerClose(frame);
                }
                WsMessage::Binary(payload)
                    if let Some((msg_type, msg_payload, uuid, library_id)) =
                        decode_message_with_library_id(&payload) =>
                {
                    debug!("[{incoming_alias}] Received: {msg_type}");

                    let Some(node) = state.node.upgrade() else {
                        break;
                    };
                    let result = state
                        .websocket_service
                        .handle_addressed_message(
                            &msg_type,
                            msg_payload,
                            Some(Session {
                                fingerprint: fingerprint.to_owned(),
                                public_key: incoming_public_key.clone(),
                                host: host.to_owned(),
                            }),
                            library_id,
                            &node,
                        )
                        .await;
                    if let Some((resp_type, response)) = result {
                        let (response_type, response) = match response {
                            Ok(response) => (resp_type, response),
                            Err(e) => {
                                error!("[{incoming_alias}] Failed to handle {msg_type}: {e:#}");
                                let failure = remote_failure(msg_type, &e);
                                match rinf::serialize(&failure) {
                                    Ok(payload) => ("RemoteRequestFailed".to_owned(), payload),
                                    Err(error) => {
                                        error!(
                                            "[{incoming_alias}] Failed to encode request failure: {error}"
                                        );
                                        continue;
                                    }
                                }
                            }
                        };

                        let response_payload =
                            encode_message(&response_type, &response, Some(uuid));
                        if let Err(e) = incoming_tx
                            .send(WsMessage::Binary(response_payload.into()))
                            .await
                        {
                            error!("[{incoming_alias}] Failed to queue response: {e}");
                            continue;
                        }
                    }
                }
                _ => {}
            }
        }

        drop(incoming_tx);
        IncomingEnd::Ended
    };

    // Clone alias for outgoing task
    let broadcast_tx = tx.clone();
    let outgoing_alias = alias.clone();
    let outgoing_policy = policy.clone();
    let outgoing_peer = peer.clone();
    let outgoing = async move {
        let mut grant_changes = outgoing_policy.changes();
        if is_controller
            && let Some(node) = outgoing_node.upgrade()
            && let Some(capabilities) = capabilities_for_peer(&node, &outgoing_peer).await
            && capabilities.set_liked_request != RequestCapability::Available
            && let Ok(payload) = rinf::serialize(&capabilities)
        {
            let message = encode_message("RequestCapabilities", &payload, None);
            if broadcast_tx
                .send(WsMessage::Binary(message.into()))
                .await
                .is_err()
            {
                return;
            }
        }
        loop {
            let message = tokio::select! {
                message = broadcast_rx.recv() => match message {
                    Ok(message) => Some(message),
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                },
                changed = grant_changes.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let Some(node) = outgoing_node.upgrade() else {
                        break;
                    };
                    if let Some(capabilities) = capabilities_for_peer(&node, &outgoing_peer).await
                        && let Ok(payload) = rinf::serialize(&capabilities)
                    {
                        Some(encode_message("RequestCapabilities", &payload, None))
                    } else {
                        None
                    }
                }
            };
            let Some(message) = message else {
                continue;
            };
            if outgoing_policy
                .check(&outgoing_peer, &Action::Control)
                .is_err()
            {
                continue;
            }
            let is_capability = is_capability_message(&message);
            let Some(node) = outgoing_node.upgrade() else {
                break;
            };
            let message = if is_capability {
                let Some(capabilities) = capabilities_for_peer(&node, &outgoing_peer).await else {
                    continue;
                };
                tailor_capability_message(message, Some(&capabilities))
            } else {
                message
            };
            if let Err(error) = broadcast_tx.send(WsMessage::Binary(message.into())).await {
                error!("[{outgoing_alias}] Failed to queue broadcast: {error}");
                break;
            }
        }

        drop(broadcast_tx);
    };

    let revoked = async {
        loop {
            let approved = permission_manager
                .read()
                .await
                .verify_by_fingerprint(&session_fingerprint)
                .await
                .is_some_and(|user| user.status == UserStatus::Approved);
            if !approved {
                break;
            }
            tokio::select! {
                result = changes.changed() => if result.is_err() { break; },
            }
        }
    };
    let controller_cancelled = async {
        match &controller_cancel {
            Some(cancel) => cancel.cancelled().await,
            None => std::future::pending().await,
        }
    };
    let (close, drain_peer) = tokio::select! {
        biased;
        _ = session_cancel.cancelled() => (
            Some(CloseFrame { code: 1001, reason: "server stopping".into() }),
            true,
        ),
        end = incoming => match end {
            IncomingEnd::PeerClose(frame) => (
                Some(frame.unwrap_or(CloseFrame {
                    code: 1000,
                    reason: "".into(),
                })),
                false,
            ),
            IncomingEnd::Ended => (None, false),
        },
        _ = outgoing => (None, false),
        _ = revoked => (
            Some(CloseFrame { code: 4403, reason: "revoked".into() }),
            true,
        ),
        _ = controller_cancelled => (
            Some(CloseFrame { code: 4409, reason: "detached".into() }),
            true,
        ),
    };
    if let Some(frame) = close {
        let _ = tx.send(WsMessage::Close(Some(frame))).await;
        if drain_peer {
            let drain = async {
                let mut receiver = receiver.lock().await;
                while let Some(message) = receiver.next().await {
                    if matches!(message, Ok(WsMessage::Close(_)) | Err(_)) {
                        break;
                    }
                }
            };
            let _ = tokio::time::timeout(Duration::from_secs(1), drain).await;
        }
    }
    drop(tx);
    controller_watch_cancel.cancel();
    let _ = controller_watch.await;

    wait_for_send_task(send_task).await;
    info!("[{alias}] WebSocket connection closed");
}

async fn wait_for_send_task(mut send_task: JoinHandle<()>) {
    match tokio::time::timeout(Duration::from_secs(1), &mut send_task).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) if !error.is_cancelled() => {
            warn!("WebSocket send task failed during shutdown: {error}");
        }
        Ok(Err(_)) => {}
        Err(_) => {
            send_task.abort();
            let _ = send_task.await;
            warn!("WebSocket send task did not stop within one second");
        }
    }
}

/// The failure frame for a request that failed on this node. It carries the
/// typed kind and cause, and the call id when the request had one.
fn remote_failure(request: String, error: &anyhow::Error) -> RemoteRequestFailed {
    RemoteRequestFailed {
        kind: classify(error),
        request,
        call_id: error
            .downcast_ref::<FailedCall>()
            .map(|failed| failed.call_id),
        detail: format!("{error:#}"),
        cause: request_failure_cause(error),
    }
}

#[cfg(test)]
mod failure_tests {
    use super::{FailedCall, remote_failure, wait_for_send_task};
    use std::time::Duration;
    use tokio::time::timeout;

    #[test]
    fn failure_frame_names_the_failed_call() {
        let error = anyhow::anyhow!("search failed").context(FailedCall { call_id: 41 });
        assert_eq!(
            remote_failure("SearchForRequest".into(), &error).call_id,
            Some(41)
        );
        let plain = anyhow::anyhow!("pause failed");
        assert_eq!(remote_failure("PauseRequest".into(), &plain).call_id, None);
    }

    #[tokio::test]
    async fn stalled_send_task_is_cancelled_during_shutdown() {
        let send_task = tokio::spawn(std::future::pending());
        timeout(Duration::from_secs(3), wait_for_send_task(send_task))
            .await
            .expect("send task shutdown must be bounded");
    }
}
