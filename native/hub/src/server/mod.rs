use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use futures::SinkExt;
use futures::StreamExt;
use log::{error, info};
use prost::Message as ProstMessage;
use tokio::sync::Mutex;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message as TungsteniteMessage;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::process_server_handlers;
use crate::register_server_handlers;
use crate::utils::RinfRustSignal;
use crate::CrashResponse;

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
        T: ProstMessage + RinfRustSignal + Default + 'static,
    {
        let handler_fn = Box::new(
            move |payload: Vec<u8>| match T::decode(payload.as_slice()) {
                Ok(decoded) => {
                    decoded.send();
                }
                Err(e) => {
                    error!("Failed to decode message: {}", e);
                    CrashResponse {
                        detail: format!("Failed to decode message: {}", e),
                    };
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
            error!("No handler registered for message type: {}", msg_type);
        }
    }

    pub async fn run(&self, url: &str, token: Arc<CancellationToken>) -> Result<()> {
        info!("Connecting to {}", url);

        let (ws_stream, _) = connect_async(url).await?;
        info!("WebSocket connection established");

        let (mut write, mut read) = ws_stream.split();

        loop {
            tokio::select! {
                message = read.next() => {
                    match message {
                        Some(Ok(msg)) => {
                            if let TungsteniteMessage::Binary(payload) = msg {
                                if let Some((msg_type, msg_payload, _request_id)) = decode_message(&payload) {
                                    self.handle_message(&msg_type, msg_payload).await;
                                }
                            }
                        }
                        Some(Err(e)) => {
                            error!("Error receiving message: {}", e);
                            break;
                        }
                        None => break,
                    }
                }
                _ = token.cancelled() => {
                    info!("Received cancel signal, closing connection");
                    if let Err(e) = write.close().await {
                        error!("Error closing websocket connection: {}", e);
                    }
                    break;
                }
            }
        }

        Ok(())
    }
}

pub async fn server_player_loop(url: String) {
    info!("Media Library Received, initialize other receivers");

    tokio::spawn(async move {
        info!("Initializing bridge");
        let bridge = WebSocketDartBridge::new();

        let main_cancel_token = CancellationToken::new();
        let main_cancel_token = Arc::new(main_cancel_token);

        for_all_responses!(register_server_handlers, bridge);

        bridge.run(&url, main_cancel_token).await
    });
}
