use std::sync::Arc;
use std::{collections::HashMap, process::exit};

use anyhow::{Context, Result, anyhow};
use futures::{SinkExt, StreamExt};
use log::error;
use rinf::{DartSignal, RustSignal};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::{connect_async, tungstenite};
use tungstenite::Message;
use uuid::Uuid;

use hub::backends::remote::{decode_message, encode_message};

pub struct WSConnection {
    tx: mpsc::Sender<(String, Vec<u8>, Uuid)>,
    response_channels: Arc<RwLock<HashMap<Uuid, mpsc::Sender<Vec<u8>>>>>,
}

impl WSConnection {
    pub async fn connect(url: String) -> Result<Self> {
        let (ws_stream, _) = connect_async(url).await?;
        let (write, mut read) = ws_stream.split();

        let (tx, mut rx) = mpsc::channel::<(String, Vec<u8>, Uuid)>(32);
        let response_channels =
            Arc::new(RwLock::new(HashMap::<Uuid, mpsc::Sender<Vec<u8>>>::new()));
        let response_channels_clone = response_channels.clone();

        // Handle outgoing messages
        tokio::spawn(async move {
            let mut write = write;
            while let Some((type_name, payload, uuid)) = rx.recv().await {
                let message = encode_message(&type_name, &payload, Some(uuid));
                if let Err(e) = write.send(Message::Binary(message.into())).await {
                    eprintln!("Failed to send message: {e}");
                    break;
                }
            }
        });

        // Handle incoming messages
        tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Binary(payload)) => {
                        if let Some((_type_name, payload, uuid)) = decode_message(&payload) {
                            let channels = response_channels_clone.read().await;
                            if let Some(channel) = channels.get(&uuid) {
                                let _ = channel.send(payload).await;
                            }
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Err(e) => {
                        error!("Error receiving message: {e}");
                        exit(0);
                    }
                    _ => {}
                }
            }
        });

        Ok(Self {
            tx,
            response_channels,
        })
    }

    pub async fn request<T: DartSignal + Serialize, U: RustSignal + for<'a> Deserialize<'a>>(
        &self,
        type_name: &str,
        request: T,
    ) -> Result<U> {
        let (response_tx, mut response_rx) = mpsc::channel(1);
        let uuid = Uuid::new_v4();

        {
            let mut channels = self.response_channels.write().await;
            channels.insert(uuid, response_tx);
        }

        let payload = rinf::serialize(&request).with_context(|| "Failed to serialize request")?;
        self.tx.send((type_name.to_string(), payload, uuid)).await?;

        let response = response_rx
            .recv()
            .await
            .ok_or_else(|| anyhow!("No response received"))?;
        let decoded = rinf::deserialize::<U>(&response[..])?;

        {
            let mut channels = self.response_channels.write().await;
            channels.remove(&uuid);
        }

        Ok(decoded)
    }

    pub async fn request_simple<T: DartSignal + Serialize>(
        &self,
        type_name: &str,
        request: T,
    ) -> Result<()> {
        let (response_tx, _) = mpsc::channel(1);
        let uuid = Uuid::new_v4();

        {
            let mut channels = self.response_channels.write().await;
            channels.insert(uuid, response_tx);
        }

        let payload = rinf::serialize(&request).with_context(|| "Failed to serialize request")?;
        self.tx.send((type_name.to_string(), payload, uuid)).await?;

        Ok(())
    }
}
