use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    Extension,
    extract::{
        ConnectInfo, Query, State,
        ws::{CloseFrame, Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use tokio::sync::mpsc;

use crate::{
    Session,
    backends::remote::{decode_message, encode_message},
    server::{ServerState, sync_auth::AuthenticatedFingerprint},
};
use discovery::server::{User, UserStatus};

pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    Extension(AuthenticatedFingerprint(fingerprint)): Extension<AuthenticatedFingerprint>,
    Query(params): Query<HashMap<String, String>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<ServerState>>,
) -> Response {
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
            info!("Connection authorized for {} @ {}", user.alias, addr);
            let host = format!("https://{host}:7863");
            ws.on_upgrade(move |socket| handle_socket(socket, state, user, host))
        }
        None => {
            warn!("Unauthorized connection attempt from {}", addr);
            StatusCode::UNAUTHORIZED.into_response()
        }
    }
}

pub async fn handle_socket(socket: WebSocket, state: Arc<ServerState>, user: User, host: String) {
    let mut changes = state.permission_manager.read().await.changes();
    let permission_manager = state.permission_manager.clone();
    let session_cancel = state.session_cancel.clone();
    let session_fingerprint = user.fingerprint.clone();
    let (mut sender, mut receiver) = socket.split();
    let mut broadcast_rx = state.websocket_service.broadcast_tx.subscribe();
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
    let incoming = async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let WsMessage::Binary(payload) = msg
                && let Some((msg_type, msg_payload, uuid)) = decode_message(&payload)
            {
                debug!("[{incoming_alias}] Received: {msg_type}");

                if let Some((resp_type, response)) = state
                    .websocket_service
                    .handle_message(
                        &msg_type,
                        msg_payload,
                        Some(Session {
                            fingerprint: fingerprint.to_owned(),
                            host: host.to_owned(),
                        }),
                    )
                    .await
                {
                    let response = match response {
                        Ok(response) => response,
                        Err(e) => {
                            error!("[{incoming_alias}] Failed to handle message: {e}");
                            continue;
                        }
                    };

                    let response_payload = encode_message(&resp_type, &response, Some(uuid));
                    if let Err(e) = incoming_tx
                        .send(WsMessage::Binary(response_payload.into()))
                        .await
                    {
                        error!("[{incoming_alias}] Failed to queue response: {e}");
                        continue;
                    }
                }
            }
        }

        drop(incoming_tx);
    };

    // Clone alias for outgoing task
    let broadcast_tx = tx.clone();
    let outgoing_alias = alias.clone();
    let outgoing = async move {
        while let Ok(msg) = broadcast_rx.recv().await {
            if let Err(e) = broadcast_tx.send(WsMessage::Binary(msg.into())).await {
                error!("[{outgoing_alias}] Failed to queue broadcast: {e}");
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
            if changes.changed().await.is_err() {
                break;
            }
        }
    };
    let close = tokio::select! {
        _ = incoming => None,
        _ = outgoing => None,
        _ = revoked => Some((4403, "revoked")),
        _ = session_cancel.cancelled() => Some((1001, "server stopping")),
    };
    if let Some((code, reason)) = close {
        let _ = tx
            .send(WsMessage::Close(Some(CloseFrame {
                code,
                reason: reason.into(),
            })))
            .await;
    }
    drop(tx);

    // Wait for the send task to complete
    let _ = send_task.await;
    info!("[{alias}] WebSocket connection closed");
}
