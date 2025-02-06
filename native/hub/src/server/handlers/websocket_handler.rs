use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    extract::{
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
        ConnectInfo, Query, State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use discovery::permission::{User, UserStatus};
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use tokio::sync::mpsc;

use crate::{
    remote::{decode_message, encode_message},
    server::ServerState,
};

pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<HashMap<String, String>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<ServerState>>,
) -> Response {
    let auth_key = params
        .get("auth")
        .or_else(|| params.get("public_key"))
        .or_else(|| params.get("fingerprint"))
        .cloned();

    let auth_result = async {
        let auth_key = auth_key.ok_or(StatusCode::UNAUTHORIZED)?;

        if let Some(user) = state
            .permission_manager
            .read()
            .await
            .verify_by_public_key(&auth_key)
            .await
        {
            return match user.status {
                UserStatus::Approved => Ok(user),
                UserStatus::Blocked => Err(StatusCode::FORBIDDEN),
                UserStatus::Pending => Err(StatusCode::UNAUTHORIZED),
            };
        }

        if let Some(user) = state
            .permission_manager
            .read()
            .await
            .verify_by_fingerprint(&auth_key)
            .await
        {
            return match user.status {
                UserStatus::Approved => Ok(user),
                UserStatus::Blocked => Err(StatusCode::FORBIDDEN),
                UserStatus::Pending => Err(StatusCode::UNAUTHORIZED),
            };
        }

        Err(StatusCode::UNAUTHORIZED)
    }
    .await;

    match auth_result {
        Ok(user) => {
            info!("Connection authorized for {} @ {}", user.alias, addr);
            ws.on_upgrade(|socket| handle_socket(socket, state, user))
        }
        Err(code) => {
            warn!("Unauthorized connection attempt from {}", addr);
            code.into_response()
        }
    }
}

pub async fn handle_socket(socket: WebSocket, state: Arc<ServerState>, user: User) {
    let (mut sender, mut receiver) = socket.split();
    let mut broadcast_rx = state.websocket_service.broadcast_tx.subscribe();
    let (tx, mut rx) = mpsc::channel(32);

    let alias = user.alias.clone();

    info!("[{}] WebSocket connection established", alias);

    // Clone alias for send_task
    let send_task_alias = alias.clone();
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Err(e) = sender.send(msg).await {
                error!("[{}] Failed to send message: {}", send_task_alias, e);
                break;
            }
        }
    });

    // Clone alias for incoming task
    let incoming_tx = tx.clone();
    let incoming_alias = alias.clone();
    let incoming = async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let WsMessage::Binary(payload) = msg {
                if let Some((msg_type, msg_payload, uuid)) = decode_message(&payload) {
                    debug!("[{}] Received: {}", incoming_alias, msg_type);

                    if let Some((resp_type, response)) = state
                        .websocket_service
                        .handle_message(&msg_type, msg_payload)
                        .await
                    {
                        let response_payload = encode_message(&resp_type, &response, Some(uuid));
                        if let Err(e) = incoming_tx
                            .send(WsMessage::Binary(response_payload.into()))
                            .await
                        {
                            error!("[{}] Failed to queue response: {}", incoming_alias, e);
                            break;
                        }
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
                error!("[{}] Failed to queue broadcast: {}", outgoing_alias, e);
                break;
            }
        }

        drop(broadcast_tx);
    };

    // Drop the original tx as we've cloned it for both tasks
    drop(tx);

    // Run tasks concurrently
    tokio::select! {
        _ = incoming => {},
        _ = outgoing => {},
    };

    // Wait for the send task to complete
    let _ = send_task.await;
    info!("[{}] WebSocket connection closed", alias);
}
