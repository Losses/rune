use std::sync::Arc;
use std::{collections::HashMap, process::exit};

use anyhow::{Context, Result, anyhow, bail};
use futures::{SinkExt, StreamExt};
use log::error;
use rinf::{DartSignal, RustSignal};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::{connect_async, tungstenite};
use tungstenite::Message;
use uuid::Uuid;

use hub::{
    backends::remote::{decode_message, encode_message, encode_request},
    messages::{ListLocalLibrariesRequest, ListLocalLibrariesResponse, LocalLibrary},
};

pub struct WSConnection {
    tx: mpsc::Sender<(String, Vec<u8>, Uuid, Option<Uuid>)>,
    response_channels: Arc<RwLock<HashMap<Uuid, mpsc::Sender<Vec<u8>>>>>,
    library_id: Arc<RwLock<Option<Uuid>>>,
}

impl WSConnection {
    pub async fn connect(url: String, library: Option<&str>) -> Result<Self> {
        let (ws_stream, _) = connect_async(url).await?;
        let (write, mut read) = ws_stream.split();

        let (tx, mut rx) = mpsc::channel::<(String, Vec<u8>, Uuid, Option<Uuid>)>(32);
        let response_channels =
            Arc::new(RwLock::new(HashMap::<Uuid, mpsc::Sender<Vec<u8>>>::new()));
        let response_channels_clone = response_channels.clone();

        // Handle outgoing messages
        tokio::spawn(async move {
            let mut write = write;
            while let Some((type_name, payload, uuid, library_id)) = rx.recv().await {
                let message = match library_id {
                    Some(library_id) => encode_request(&type_name, &payload, uuid, library_id),
                    None => encode_message(&type_name, &payload, Some(uuid)),
                };
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

        let connection = Self {
            tx,
            response_channels,
            library_id: Arc::new(RwLock::new(None)),
        };
        let libraries: ListLocalLibrariesResponse = connection
            .request("ListLocalLibrariesRequest", ListLocalLibrariesRequest {})
            .await?;
        let library_id = choose_library(&libraries.libraries, library)?;
        *connection.library_id.write().await = Some(library_id);

        Ok(connection)
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
        let library_id = *self.library_id.read().await;
        self.tx
            .send((type_name.to_string(), payload, uuid, library_id))
            .await?;

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
        let library_id = *self.library_id.read().await;
        self.tx
            .send((type_name.to_string(), payload, uuid, library_id))
            .await?;

        Ok(())
    }
}

/// The library the CLI addresses: the one named on the command line, or the
/// only one the server holds.
fn choose_library(libraries: &[LocalLibrary], requested: Option<&str>) -> Result<Uuid> {
    let chosen = match requested {
        Some(requested) => libraries
            .iter()
            .find(|library| library.library_id == requested || library.alias == requested)
            .with_context(|| format!("The server has no library named {requested}"))?,
        None => match libraries {
            [only] => only,
            [] => bail!("The server has no library to open"),
            _ => bail!(
                "The server holds several libraries; choose one with --library: {}",
                libraries
                    .iter()
                    .map(|library| format!("{} ({})", library.alias, library.library_id))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        },
    };
    chosen
        .library_id
        .parse::<Uuid>()
        .context("The server returned an invalid library id")
}

#[cfg(test)]
mod tests {
    use super::{LocalLibrary, choose_library};

    fn library(id: &str, alias: &str) -> LocalLibrary {
        LocalLibrary {
            library_id: id.to_owned(),
            alias: alias.to_owned(),
            path: String::new(),
            owner_fingerprint: None,
            background_sync: false,
            access_level: None,
        }
    }

    #[test]
    fn chooses_the_only_library_or_the_named_one() {
        let first = "8a4a4f6e-2f0b-4b39-9d0e-0d1f7a3a1b11";
        let second = "1c8e2f40-7b5d-4a2e-8f61-3e9b0c4d5a22";
        assert_eq!(
            choose_library(&[library(first, "Music")], None)
                .unwrap()
                .to_string(),
            first
        );
        let both = [library(first, "Music"), library(second, "Podcasts")];
        assert!(choose_library(&both, None).is_err());
        assert_eq!(
            choose_library(&both, Some("Podcasts")).unwrap().to_string(),
            second
        );
        assert_eq!(
            choose_library(&both, Some(second)).unwrap().to_string(),
            second
        );
        assert!(choose_library(&both, Some("Missing")).is_err());
        assert!(choose_library(&[], None).is_err());
    }
}
