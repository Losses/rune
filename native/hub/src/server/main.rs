use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::Result;
use clap::{Arg, Command};
use discovery::{
    utils::{DeviceInfo, DeviceType},
    DiscoveryParams,
};
use log::{error, info};
use prost::Message;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use hub::{
    for_all_request_pairs2, handle_server_response, listen_server_event,
    messages::*,
    process_server_handlers, register_single_handler,
    server::{handlers::serve_combined, AppState, LocalFileProvider, WebSocketService},
    utils::{
        initialize_databases, player::initialize_player, GlobalParams, ParamsExtractor,
        RinfRustSignal, TaskTokens,
    },
    Signal,
};

use ::database::actions::cover_art::COVER_TEMP_DIR;
use ::database::connection::{MainDbConnection, RecommendationDbConnection};
use ::playback::{player::Player, sfx_player::SfxPlayer};
use ::scrobbling::manager::ScrobblingManager;

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

    let discovery_paramaters = DiscoveryParams {
        device_info: DeviceInfo {
            alias: "Rune Server".to_string(),
            version: "TP".to_string(),
            device_model: Some("Rune Server".to_string()),
            device_type: Some(DeviceType::Server),
            fingerprint: "FINGERPRINT".to_string(),
            port: 7863,
            protocol: "http".to_string(),
            download: Some(false),
        },
        pin: None,
        file_provider: Arc::new(LocalFileProvider {
            root_dir: app_state.lib_path.clone(),
        }),
    };

    serve_combined(app_state, websocket_service, addr, discovery_paramaters).await;

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
