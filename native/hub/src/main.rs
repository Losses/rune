use std::{
    collections::HashMap, env, future::Future, io::Error as IoError, net::SocketAddr, pin::Pin,
    sync::Arc, time::Duration,
};

use anyhow::{Context, Result};
use futures_channel::mpsc::{unbounded, UnboundedSender};
use futures_util::{future, pin_mut, stream::TryStreamExt, StreamExt};
use log::{error, info};
use prost::Message;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Mutex,
};
use tokio_tungstenite::tungstenite::protocol::Message as TungsteniteMessage;
use tokio_util::sync::CancellationToken;

use hub::{
    for_all_requests2,
    messages::*,
    player::initialize_player,
    utils::{
        initialize_databases, Broadcaster, GlobalParams, ParamsExtractor, RinfRustSignal,
        TaskTokens,
    },
    Signal,
};

use ::database::connection::{MainDbConnection, RecommendationDbConnection};
use ::playback::{player::Player, sfx_player::SfxPlayer};
use ::scrobbling::manager::ScrobblingManager;

type HandlerFn = Box<dyn Fn(Vec<u8>) -> BoxFuture<'static, Vec<u8>> + Send + Sync>;
type HandlerMap = Arc<Mutex<HashMap<String, HandlerFn>>>;
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

type Tx = UnboundedSender<TungsteniteMessage>;
type PeerMap = Arc<Mutex<HashMap<SocketAddr, Tx>>>;

pub trait RequestHandler: Send + Sync + 'static {
    type Params;
    type Response;

    fn handle(&self, params: Self::Params) -> Result<Option<Self::Response>, anyhow::Error>;
}

pub trait WebSocketMessage {
    fn get_type() -> &'static str;
}

#[macro_export]
macro_rules! listen_server_event {
    ($server:expr, $global_params:expr, $($req:tt)*) => {
        process_server_handlers!(@internal $server, $global_params, $($req)*);
    };
}

#[macro_export]
macro_rules! process_server_handlers {
    (@internal $server:expr, $global_params:expr, ($request:ty, $response:ty) $(, $rest:tt)*) => {
        register_single_handler!($server, $global_params, $request, with_response);
        process_server_handlers!(@internal $server, $global_params $(, $rest)*);
    };
    (@internal $server:expr, $global_params:expr, $request:ty $(, $rest:tt)*) => {
        register_single_handler!($server, $global_params, $request, without_response);
        process_server_handlers!(@internal $server, $global_params $(, $rest)*);
    };
    (@internal $server:expr, $global_params:expr $(,)?) => {};
}

#[macro_export]
macro_rules! register_single_handler {
    ($server:expr, $global_params:expr, $request:ty, $response_type:tt) => {
        paste::paste! {
            let global_params = $global_params.clone();
            $server.register_handler(stringify!($request), move |payload| {
                let global_params = global_params.clone();
                async move {
                    let buf = payload.as_slice();
                    let request = match $request::decode(buf) {
                        Ok(req) => req,
                        Err(e) => {
                            error!("Failed to deserialize request: {:?}", e);
                            return CrashResponse {
                                detail: format!("Failed to deserialize request: {:?}", e),
                            }.encode_to_vec();
                        }
                    };

                    let params = request.extract_params(&global_params);
                    match request.handle(params, &request).await {
                        Ok(_response) => {
                            handle_server_response!(_response, $response_type)
                        }
                        Err(e) => {
                            error!("Error handling request: {:?}", e);
                            CrashResponse {
                                detail: format!("{:#?}", e),
                            }.encode_to_vec()
                        }
                    }
                }
            }).await;
        }
    };
}

#[macro_export]
macro_rules! handle_server_response {
    ($response:expr, with_response) => {
        if let Some(response) = $response {
            response.encode_to_vec()
        } else {
            Vec::new()
        }
    };
    ($response:expr, without_response) => {
        Vec::new()
    };
}

impl Broadcaster for Server {
    fn broadcast(&self, message: &dyn RinfRustSignal) {
        let peer_map = self.peer_map.clone();

        let type_name = message.name();
        let payload = message.encode_message();

        let type_len = type_name.len() as u8;
        let mut message_data = vec![type_len];
        message_data.extend_from_slice(type_name.as_bytes());
        message_data.extend_from_slice(&payload);

        let ws_message = TungsteniteMessage::Binary(message_data.into());

        tokio::spawn(async move {
            let peers = peer_map.lock().await;
            for (addr, tx) in peers.iter() {
                if let Err(e) = tx.unbounded_send(ws_message.clone()) {
                    error!("Failed to broadcast message to {}: {}", addr, e);
                }
            }
        });
    }
}

struct Server {
    handlers: HandlerMap,
    peer_map: PeerMap,
}

impl Server {
    fn new() -> Self {
        Server {
            handlers: Arc::new(Mutex::new(HashMap::new())),
            peer_map: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn register_handler<F, Fut>(&self, msg_type: &str, handler: F)
    where
        F: Fn(Vec<u8>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Vec<u8>> + Send + 'static,
    {
        self.handlers.lock().await.insert(
            msg_type.to_string(),
            Box::new(move |payload| Box::pin(handler(payload))),
        );
    }

    async fn handle_message(&self, msg_type: &str, payload: Vec<u8>) -> Option<Vec<u8>> {
        let handlers = self.handlers.lock().await;
        let handler = handlers.get(msg_type)?;
        Some(handler(payload).await)
    }

    async fn handle_connection(&self, raw_stream: TcpStream, addr: SocketAddr) {
        info!("Incoming TCP connection from: {}", addr);

        let ws_stream = tokio_tungstenite::accept_async(raw_stream)
            .await
            .expect("Error during the websocket handshake occurred");
        info!("WebSocket connection established: {}", addr);

        let (tx, rx) = unbounded();
        self.peer_map.lock().await.insert(addr, tx);

        let (outgoing, incoming) = ws_stream.split();

        let broadcast_incoming = incoming.try_for_each_concurrent(None, |msg| async move {
            let payload = msg.into_data();

            // The format of the message is: [type_length(1 byte)][type_string][payload]
            if payload.is_empty() {
                return Ok(());
            }

            let type_len = payload[0] as usize;
            if payload.len() < 1 + type_len {
                return Ok(());
            }

            let msg_type = String::from_utf8_lossy(&payload[1..1 + type_len]).to_string();
            let msg_payload = payload[1 + type_len..].to_vec();

            info!("Received message type: {} from: {}", msg_type, addr);

            if let Some(response) = self.handle_message(&msg_type, msg_payload).await {
                // Building response
                let mut response_payload = vec![type_len as u8];
                response_payload.extend_from_slice(msg_type.as_bytes());
                response_payload.extend_from_slice(&response);

                if let Some(peer_tx) = self.peer_map.lock().await.get(&addr) {
                    peer_tx
                        .unbounded_send(TungsteniteMessage::Binary(response_payload.into()))
                        .unwrap_or_else(|e| error!("Failed to send response: {}", e));
                }
            }

            Ok(())
        });

        let receive_from_others = rx.map(Ok).forward(outgoing);

        pin_mut!(broadcast_incoming, receive_from_others);
        future::select(broadcast_incoming, receive_from_others).await;

        info!("{} disconnected", &addr);
        self.peer_map.lock().await.remove(&addr);
    }

    async fn run(&self, addr: &str) -> Result<(), IoError> {
        let try_socket = TcpListener::bind(addr).await;
        let listener = try_socket.expect("Failed to bind");
        info!("Listening on: {}", addr);

        while let Ok((stream, addr)) = listener.accept().await {
            let server = self.clone();
            tokio::spawn(async move {
                server.handle_connection(stream, addr).await;
            });
        }

        Ok(())
    }
}

impl Clone for Server {
    fn clone(&self) -> Self {
        Server {
            handlers: self.handlers.clone(),
            peer_map: self.peer_map.clone(),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8080".to_string());

    let server = Server::new();

    let lib_path = "/";
    let database_path = "/";
    let db_connections = initialize_databases(lib_path, Some(database_path)).await?;

    let main_db: Arc<MainDbConnection> = db_connections.main_db;

    let recommend_db: Arc<RecommendationDbConnection> = db_connections.recommend_db;

    let lib_path: Arc<String> = Arc::new(lib_path.to_string());

    let main_cancel_token = CancellationToken::new();
    let task_tokens: Arc<Mutex<TaskTokens>> = Arc::new(Mutex::new(TaskTokens::default()));

    info!("Initializing player");
    let player = Player::new(Some(main_cancel_token.clone()));
    let player: Arc<Mutex<Player>> = Arc::new(Mutex::new(player));

    let sfx_player = SfxPlayer::new(Some(main_cancel_token.clone()));
    let sfx_player: Arc<Mutex<SfxPlayer>> = Arc::new(Mutex::new(sfx_player));

    let main_cancel_token = Arc::new(main_cancel_token);

    let scrobbler = ScrobblingManager::new(10, Duration::new(5, 0));
    let scrobbler = Arc::new(Mutex::new(scrobbler));

    let server = Arc::new(server);
    let broadcaster = Arc::clone(&server);

    info!("Initializing Player events");
    tokio::spawn(initialize_player(
        lib_path.clone(),
        main_db.clone(),
        player.clone(),
        scrobbler.clone(),
        broadcaster.clone(),
    ));

    info!("Initializing UI events");

    let global_params = Arc::new(GlobalParams {
        lib_path,
        main_db,
        recommend_db,
        main_token: main_cancel_token,
        task_tokens,
        player,
        sfx_player,
        scrobbler,
        broadcaster,
    });

    for_all_requests2!(listen_server_event, server, global_params);

    server
        .run(&addr)
        .await
        .with_context(|| "Failed to start the server")
}
