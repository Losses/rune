#[macro_use]
mod remote_request;

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use anyhow::{bail, Context, Result};
use futures::{SinkExt, StreamExt};
use log::{debug, error, info};
use prost::Message as ProstMessage;
use rustls::ClientConfig;
use tokio::sync::{Mutex, RwLock};
use tokio_tungstenite::{
    connect_async_tls_with_config, tungstenite::protocol::Message as TungsteniteMessage, Connector,
};
use tokio_util::sync::CancellationToken;
use urlencoding::encode;
use uuid::Uuid;

use ::database::connection::{connect_fake_main_db, connect_fake_recommendation_db};
use ::discovery::{
    client::{select_best_host, CertValidator},
    protocol::DiscoveryService,
    server::PermissionManager,
    url::decode_rnsrv_url,
};
use ::playback::{player::MockPlayer, sfx_player::SfxPlayer};
use ::scrobbling::manager::MockScrobblingManager;

use crate::{
    forward_event_to_remote, implement_rinf_dart_signal_trait,
    messages::*,
    register_remote_handlers,
    server::{api::check_fingerprint, generate_or_load_certificates},
    utils::{
        nid::get_or_create_node_id, GlobalParams, LocalGuiBroadcaster, ParamsExtractor,
        RinfRustSignal, RunningMode, TaskTokens,
    },
    Signal,
};

pub trait RinfDartSignal: ProstMessage {
    fn name(&self) -> String;
}

for_all_requests0!(implement_rinf_dart_signal_trait);
for_all_responses0!(implement_rinf_rust_signal_trait);

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
            error!(
                "No handler registered for message type in the message bridge: {}",
                msg_type
            );
        }
    }

    pub async fn run(
        &mut self,
        rnsrv_url: &str,
        host: &str,
        config_path: &str,
        config: Arc<ClientConfig>,
        fingerprint: &str,
    ) -> Result<()> {
        let url = format!(
            "wss://{}:7863/ws?fingerprint={}&host={}",
            host,
            encode(fingerprint),
            encode(host)
        );

        info!("Connecting to {}", host);

        match connect_async_tls_with_config(
            url.clone(),
            None,
            false,
            Some(Connector::Rustls(config)),
        )
        .await
        {
            Ok((ws_stream, _)) => {
                info!("WebSocket connection established");

                let (write, mut read) = ws_stream.split();
                let write = Arc::new(Mutex::new(write));

                let cancel_token: CancellationToken = CancellationToken::new();

                let sfx_player = SfxPlayer::new(Some(cancel_token.clone()));
                let sfx_player: Arc<Mutex<SfxPlayer>> = Arc::new(Mutex::new(sfx_player));

                let cancel_token = Arc::new(cancel_token);

                let cancel_token_clone = Arc::clone(&cancel_token);
                let handle_event_close_library_request = || async move {
                    let receiver = CloseLibraryRequest::get_dart_signal_receiver();
                    loop {
                        tokio::select! {
                            _ = cancel_token_clone.cancelled() => {
                                break;
                            }
                            Some(_) = receiver.recv() => {
                                cancel_token_clone.cancel();
                            }
                        }
                    }
                };

                tokio::spawn(handle_event_close_library_request());

                let cancel_token_clone = Arc::clone(&cancel_token);
                let sfx_player_clone = Arc::clone(&sfx_player);
                let handle_event_sfx_play_request = || async move {
                    let receiver = SfxPlayRequest::get_dart_signal_receiver();
                    loop {
                        tokio::select! {
                            _ = cancel_token_clone.cancelled() => {
                                break;
                            }
                            Some(dart_signal) = receiver.recv() => {
                                sfx_player_clone
                                .lock()
                                .await
                                .load(dart_signal.message.path.clone().into());
                            }
                        }
                    }
                };

                tokio::spawn(handle_event_sfx_play_request());

                for_all_non_local_requests3!(
                    forward_event_to_remote,
                    self_arc,
                    cancel_token.clone(),
                    write.clone()
                );

                let handlers = self.handlers.clone();
                let write_clone = Arc::clone(&write);
                let cancel_token_clone = Arc::clone(&cancel_token);
                let message_loop = || async move {
                    loop {
                        tokio::select! {
                            message = read.next() => {
                                match message {
                                    Some(Ok(msg)) => {
                                        if let TungsteniteMessage::Binary(payload) = msg {
                                            if let Some((msg_type, msg_payload, _request_id)) = decode_message(&payload) {
                                                debug!("Received message with type: {}", msg_type);
                                                if let Some(handler) = handlers.lock().await.get(&msg_type) {
                                                    handler(msg_payload);
                                                } else {
                                                    error!("No handler registered for message type while receiving response: {}", msg_type);
                                                }
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
                            _ = cancel_token_clone.cancelled() => {
                                info!("Received cancel signal, closing connection");
                                let mut write = write_clone.lock().await;
                                if let Err(e) = write.close().await {
                                    error!("Error closing websocket connection: {}", e);
                                }
                                break;
                            }
                        }
                    }
                };

                tokio::spawn(message_loop());

                let device_scanner = Arc::new(DiscoveryService::without_store());
                let permission_manager =
                    Arc::new(RwLock::new(PermissionManager::new(config_path).unwrap()));
                let cert_validator =
                    Arc::new(RwLock::new(CertValidator::new(config_path).await.unwrap()));

                info!("Initializing UI events");
                let node_id = get_or_create_node_id(config_path).await?.to_string();

                let global_params = GlobalParams {
                    lib_path: Arc::new(rnsrv_url.to_owned()),
                    config_path: Arc::new(config_path.to_string()),
                    node_id: Arc::new(node_id),
                    main_db: Arc::new(connect_fake_main_db().await?),
                    recommend_db: Arc::new(connect_fake_recommendation_db()?),
                    main_token: Arc::clone(&cancel_token),
                    task_tokens: Arc::new(Mutex::new(TaskTokens {
                        scan_token: None,
                        analyze_token: None,
                        deduplicate_token: None,
                    })),
                    player: Arc::new(Mutex::new(MockPlayer {})),
                    sfx_player,
                    scrobbler: Arc::new(Mutex::new(MockScrobblingManager::new())),
                    broadcaster: Arc::new(LocalGuiBroadcaster),
                    device_scanner,
                    cert_validator,
                    permission_manager,
                    server_manager: OnceLock::new(),
                    running_mode: RunningMode::Server,
                };

                let global_params = Arc::new(global_params);

                for_all_local_only_request_pairs2!(
                    listen_local_gui_event,
                    global_params,
                    cancel_token
                );

                Ok(())
            }
            Err(e) => {
                let error_msg = format!("Failed to connect: {}", e);
                error!("{}", error_msg);

                CrashResponse { detail: error_msg }.send_signal_to_dart();
                Err(e.into())
            }
        }
    }
}

pub async fn server_player_loop(url: &str, config_path: &str, alias: &str) -> Result<()> {
    info!("Media Library Received, initialize the server loop");

    let cert_validator = Arc::new(
        CertValidator::new(config_path)
            .await
            .with_context(|| "Failed to create the cert validator")?,
    );
    let client_config = cert_validator.clone().into_client_config();

    let hosts = decode_rnsrv_url(url).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let host = select_best_host(hosts, Arc::new(client_config))
        .await
        .with_context(|| "Failed to select the best host")?;

    let client_config = Arc::new(cert_validator.clone().into_client_config());

    let (fingerprint, _, _) = generate_or_load_certificates(&config_path, alias)
        .await
        .with_context(|| "Failed to kiad certufucates")?;

    let result = check_fingerprint(&host, client_config.clone(), &fingerprint)
        .await
        .with_context(|| "Failed to check fingerprint")?;

    if !result.is_trusted {
        bail!("This client is not trusted by the server");
    }

    let rnsrv_url = url.to_string();
    let config_path = config_path.to_string();
    tokio::spawn(async move {
        info!("Initializing bridge");
        let mut bridge = WebSocketDartBridge::new();

        info!("Registering remote handlers");
        for_all_responses!(register_remote_handlers, bridge);
        register_remote_handlers!(
            bridge,
            ScanAudioLibraryProgress,
            ScanAudioLibraryResponse,
            SetMediaLibraryPathResponse,
            AnalyzeAudioLibraryProgress,
            AnalyzeAudioLibraryResponse,
            PlaybackStatus,
            ScrobbleServiceStatusUpdated,
            CrashResponse,
            RealtimeFft,
            PlaylistUpdate
        );

        bridge
            .run(&rnsrv_url, &host, &config_path, client_config, &fingerprint)
            .await
    });

    Ok(())
}
