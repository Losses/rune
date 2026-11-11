use std::sync::Arc;
use std::{collections::HashMap, process::exit};

use anyhow::{Context, Result, anyhow};
use futures::{SinkExt, StreamExt};
use log::error;
use rinf::DartSignal;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::{connect_async, tungstenite};
use tungstenite::Message;
use uuid::Uuid;

use hub::{
    backends::remote::{decode_message, encode_message, encode_request},
    messages::RemoteRequestFailed,
    request_error::ExpectedRequestError,
    rpc::{RequestScope, TargetRequest},
};

type ResponseFrame = (String, Vec<u8>);
type ResponseChannels = Arc<RwLock<HashMap<Uuid, mpsc::Sender<ResponseFrame>>>>;

pub struct WSConnection {
    tx: mpsc::Sender<Vec<u8>>,
    response_channels: ResponseChannels,
    library_id: Uuid,
}

impl WSConnection {
    pub async fn connect(url: String, library_id: Uuid) -> Result<Self> {
        let (ws_stream, _) = connect_async(url).await?;
        let (write, mut read) = ws_stream.split();

        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(32);
        let response_channels = Arc::new(RwLock::new(
            HashMap::<Uuid, mpsc::Sender<ResponseFrame>>::new(),
        ));
        let response_channels_clone = response_channels.clone();

        // Handle outgoing messages
        tokio::spawn(async move {
            let mut write = write;
            while let Some(message) = rx.recv().await {
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
                        if let Some((type_name, payload, uuid)) = decode_message(&payload) {
                            let channels = response_channels_clone.read().await;
                            if let Some(channel) = channels.get(&uuid) {
                                let _ = channel.send((type_name, payload)).await;
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
            library_id,
        })
    }

    pub async fn request<R>(&self, request: &R) -> Result<R::Response>
    where
        R: TargetRequest + DartSignal + Serialize,
        R::Response: for<'a> Deserialize<'a>,
    {
        let (response_tx, mut response_rx) = mpsc::channel(1);
        let uuid = Uuid::new_v4();

        {
            let mut channels = self.response_channels.write().await;
            channels.insert(uuid, response_tx);
        }

        let payload = rinf::serialize(&request).with_context(|| "Failed to serialize request")?;
        let frame = encode_client_request::<R>(R::wire_name(), &payload, uuid, self.library_id);
        self.tx.send(frame).await?;

        let (name, response) = response_rx
            .recv()
            .await
            .ok_or_else(|| anyhow!("No response received"))?;
        self.response_channels.write().await.remove(&uuid);
        if name == "RemoteRequestFailed" {
            let failure = rinf::deserialize::<RemoteRequestFailed>(&response)?;
            return Err(ExpectedRequestError::Remote {
                kind: failure.kind,
                detail: failure.detail,
                cause: failure.cause,
            }
            .into());
        }
        if name != R::response_name() {
            return Err(ExpectedRequestError::UnexpectedResponse {
                expected: R::response_name().to_owned(),
                actual: name,
            }
            .into());
        }
        let decoded = rinf::deserialize::<R::Response>(&response[..])?;

        {
            let mut channels = self.response_channels.write().await;
            channels.remove(&uuid);
        }

        Ok(decoded)
    }

    pub async fn request_simple<R>(&self, request: &R) -> Result<()>
    where
        R: TargetRequest<Response = ()> + DartSignal + Serialize,
    {
        self.request(request).await
    }
}

fn encode_client_request<R: TargetRequest>(
    name: &str,
    payload: &[u8],
    request_id: Uuid,
    library_id: Uuid,
) -> Vec<u8> {
    if R::SCOPE == RequestScope::Library {
        encode_request(name, payload, request_id, library_id)
    } else {
        encode_message(name, payload, Some(request_id))
    }
}

#[cfg(test)]
mod tests {
    use super::encode_client_request;
    use hub::{
        TypedRequest,
        backends::remote::decode_message_with_library_id,
        messages::{FetchAllPlaylistsRequest, PauseRequest},
    };
    use uuid::Uuid;

    #[test]
    fn client_requests_address_library_scope_and_leave_session_scope_unaddressed() {
        let library_id = Uuid::new_v4();
        let request_id = Uuid::new_v4();
        let library_frame = encode_client_request::<FetchAllPlaylistsRequest>(
            FetchAllPlaylistsRequest::wire_name(),
            &[],
            request_id,
            library_id,
        );
        assert_eq!(
            decode_message_with_library_id(&library_frame),
            Some((
                FetchAllPlaylistsRequest::wire_name().to_owned(),
                Vec::new(),
                request_id,
                Some(library_id),
            ))
        );

        let session_frame = encode_client_request::<PauseRequest>(
            PauseRequest::wire_name(),
            &[],
            request_id,
            library_id,
        );
        assert_eq!(
            decode_message_with_library_id(&session_frame),
            Some((
                PauseRequest::wire_name().to_owned(),
                Vec::new(),
                request_id,
                None,
            ))
        );
    }
}
