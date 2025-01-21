use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info};
use tokio::sync::mpsc;

use crate::{
    remote::{decode_message, encode_message},
    server::ServerState,
};

pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<ServerState>>,
) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

pub async fn handle_socket(socket: WebSocket, state: Arc<ServerState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut broadcast_rx = state.websocket_service.broadcast_tx.subscribe();
    let (tx, mut rx) = mpsc::channel(32);

    // Spawn a task to handle sending messages
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Err(e) = sender.send(msg).await {
                error!("Failed to send message: {}", e);
                break;
            }
        }
    });

    // Handle incoming messages
    let incoming_tx = tx.clone();
    let incoming = async move {
        info!("WebSocket connection established");
        while let Some(Ok(msg)) = receiver.next().await {
            if let WsMessage::Binary(payload) = msg {
                if let Some((msg_type, msg_payload, uuid)) = decode_message(&payload) {
                    debug!("Received message type: {}", msg_type);

                    if let Some((resp_type, response)) = state
                        .websocket_service
                        .handle_message(&msg_type, msg_payload)
                        .await
                    {
                        if !resp_type.is_empty() {
                            let response_payload =
                                encode_message(&resp_type, &response, Some(uuid));
                            if let Err(e) = incoming_tx
                                .send(WsMessage::Binary(response_payload.into()))
                                .await
                            {
                                error!("Failed to queue response: {}", e);
                                break;
                            }
                        }
                    } else {
                        info!("No result returned: {}", msg_type);
                    }
                }
            }
        }

        drop(incoming_tx);
    };

    // Handle broadcast messages
    let broadcast_tx = tx.clone();
    let outgoing = async move {
        while let Ok(msg) = broadcast_rx.recv().await {
            if let Err(e) = broadcast_tx.send(WsMessage::Binary(msg.into())).await {
                error!("Failed to queue broadcast: {}", e);
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

    info!("WebSocket connection closed");
}
