use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    extract::{
        ConnectInfo, Query, State,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
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
    server::ServerState,
};
use discovery::server::{User, UserStatus};

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

    let host = params
        .get("host")
        .cloned()
        .unwrap_or("127.0.0.1".to_owned());

    let auth_result = async {
        let auth_key = auth_key.ok_or(StatusCode::BAD_REQUEST)?;

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
            let host = format!("https://{host}:7863");
            ws.on_upgrade(move |socket| handle_socket(socket, state, user, host))
        }
        Err(code) => {
            warn!(
                "Unauthorized connection attempt from {}({})",
                addr,
                match params.get("fingerprint") {
                    Some(x) => x,
                    None => "",
                }
            );
            code.into_response()
        }
    }
}

pub async fn handle_socket(socket: WebSocket, state: Arc<ServerState>, user: User, host: String) {
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

    // Drop the original tx as we've cloned it for both tasks
    drop(tx);

    // Run tasks concurrently
    tokio::select! {
        _ = incoming => {},
        _ = outgoing => {},
    };

    // Wait for the send task to complete
    let _ = send_task.await;
    info!("[{alias}] WebSocket connection closed");
}
