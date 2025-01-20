use std::{
    collections::HashMap, future::Future, net::SocketAddr, path::PathBuf, pin::Pin, sync::Arc,
    time::Duration,
};

use anyhow::Result;
use axum::{
    body::Body,
    extract::{
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use clap::{Arg, Command};
use dunce::canonicalize;
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info};
use prost::Message;
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tower::util::ServiceExt;
use tower_http::services::ServeDir;
use tracing_subscriber::EnvFilter;

use hub::{
    for_all_request_pairs2, handle_server_response, listen_server_event,
    messages::*,
    process_server_handlers, register_single_handler,
    remote::{decode_message, encode_message},
    utils::{
        initialize_databases, player::initialize_player, Broadcaster, GlobalParams,
        ParamsExtractor, RinfRustSignal, TaskTokens,
    },
    Signal,
};

use ::database::actions::cover_art::COVER_TEMP_DIR;
use ::database::connection::{MainDbConnection, RecommendationDbConnection};
use ::playback::{player::Player, sfx_player::SfxPlayer};
use ::scrobbling::manager::ScrobblingManager;

type HandlerFn = Box<dyn Fn(Vec<u8>) -> BoxFuture<'static, (String, Vec<u8>)> + Send + Sync>;
type HandlerMap = Arc<Mutex<HashMap<String, HandlerFn>>>;
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

type BroadcastTx = broadcast::Sender<Vec<u8>>;

pub trait RequestHandler: Send + Sync + 'static {
    type Params;
    type Response;

    fn handle(&self, params: Self::Params) -> Result<Option<Self::Response>, anyhow::Error>;
}

struct WebSocketService {
    handlers: HandlerMap,
    broadcast_tx: BroadcastTx,
}

impl WebSocketService {
    fn new() -> Self {
        let (broadcast_tx, _) = broadcast::channel(100);
        WebSocketService {
            handlers: Arc::new(Mutex::new(HashMap::new())),
            broadcast_tx,
        }
    }

    async fn register_handler<F, Fut>(&self, msg_type: &str, handler: F)
    where
        F: Fn(Vec<u8>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = (String, Vec<u8>)> + Send + 'static,
    {
        self.handlers.lock().await.insert(
            msg_type.to_string(),
            Box::new(move |payload| Box::pin(handler(payload))),
        );
    }

    async fn handle_message(&self, msg_type: &str, payload: Vec<u8>) -> Option<(String, Vec<u8>)> {
        let handlers = self.handlers.lock().await;
        let handler = handlers.get(msg_type)?;
        Some(handler(payload).await)
    }
}

impl Broadcaster for WebSocketService {
    fn broadcast(&self, message: &dyn RinfRustSignal) {
        let type_name = message.name();
        let payload = message.encode_message();
        let message_data = encode_message(&type_name, &payload, None);

        if let Err(e) = self.broadcast_tx.send(message_data) {
            error!("Failed to broadcast message: {}", e);
        }
    }
}

impl Clone for WebSocketService {
    fn clone(&self) -> Self {
        WebSocketService {
            handlers: self.handlers.clone(),
            broadcast_tx: self.broadcast_tx.clone(),
        }
    }
}

struct ServerState {
    app_state: Arc<AppState>,
    websocket_service: Arc<WebSocketService>,
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<ServerState>>,
) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<ServerState>) {
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

async fn serve_file(
    Path(file_path): Path<String>,
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    let lib_path = &state.app_state.lib_path;
    let cover_temp_dir = &state.app_state.cover_temp_dir;

    let requested_path = PathBuf::from(file_path);

    let canonical_path = match canonicalize(&requested_path) {
        Ok(path) => path,
        Err(_) => return StatusCode::FORBIDDEN.into_response(),
    };

    if !canonical_path.starts_with(lib_path) && !canonical_path.starts_with(cover_temp_dir) {
        return StatusCode::FORBIDDEN.into_response();
    }

    let service = ServeDir::new(".");
    let request = Request::builder()
        .uri(format!("/{}", canonical_path.to_string_lossy()))
        .body(axum::body::Body::empty())
        .unwrap();

    match service.oneshot(request).await {
        Ok(response) => {
            let (parts, body) = response.into_parts();
            let boxed_body = Body::new(body);
            Response::from_parts(parts, boxed_body)
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

struct AppState {
    lib_path: PathBuf,
    cover_temp_dir: PathBuf,
}

async fn serve_combined(
    app_state: Arc<AppState>,
    websocket_service: Arc<WebSocketService>,
    addr: SocketAddr,
) {
    let server_state = Arc::new(ServerState {
        app_state: app_state.clone(),
        websocket_service,
    });

    let app = Router::new()
        .route("/ws", get(websocket_handler))
        .route("/files/{*file_path}", get(serve_file))
        .with_state(server_state);

    info!("Starting combined HTTP/WebSocket server on {}", addr);
    axum_server::bind(addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::new(
        "symphonia_format_ogg=off,symphonia_core=off,symphonia_bundle_mp3::demuxer=off,tantivy::directory=off,tantivy::indexer=off,sea_orm_migration::migrator=off,info",
    );

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .init();

    let matches = Command::new("Rune")
        .author("Rune Developers")
        .arg(
            Arg::new("addr")
                .short('a')
                .long("addr")
                .value_name("ADDRESS")
                .default_value("127.0.0.1:7863")
                .help("Address to run the server on"),
        )
        .arg(
            Arg::new("lib_path")
                .value_name("LIB_PATH")
                .help("Library path")
                .required(true)
                .index(1),
        )
        .get_matches();

    let addr: SocketAddr = matches.get_one::<String>("addr").unwrap().parse()?;
    let lib_path = matches.get_one::<String>("lib_path").unwrap();

    let app_state = Arc::new(AppState {
        lib_path: PathBuf::from(lib_path),
        cover_temp_dir: COVER_TEMP_DIR.to_path_buf(),
    });

    let websocket_service = initialize_websocket_service(lib_path).await?;
    serve_combined(app_state, websocket_service, addr).await;

    Ok(())
}

async fn initialize_websocket_service(lib_path: &str) -> Result<Arc<WebSocketService>> {
    let server = WebSocketService::new();

    let db_path = format!("{}/.rune", lib_path);
    let db_connections = initialize_databases(lib_path, Some(&db_path)).await?;

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

    for_all_request_pairs2!(listen_server_event, server, global_params);

    Ok(server)
}
