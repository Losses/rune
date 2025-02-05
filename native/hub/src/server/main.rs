use std::{
    net::SocketAddr,
    sync::{Arc, OnceLock},
    time::Duration,
};

use anyhow::Result;
use clap::{Arg, Command};
use discovery::{
    utils::{DeviceInfo, DeviceType},
    DiscoveryParams,
};
use log::info;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use hub::{
    server::{ServerManager, WebSocketService},
    utils::{
        device_scanner::DeviceScanner, initialize_databases, player::initialize_player,
        GlobalParams, TaskTokens,
    },
};

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
        .arg(
            Arg::new("config_path")
                .value_name("CONFIG_PATH")
                .help("Configuration path")
                .required(true)
                .index(1),
        )
        .get_matches();

    let addr: SocketAddr = matches.get_one::<String>("addr").unwrap().parse()?;
    let lib_path = matches.get_one::<String>("lib_path").unwrap();
    let config_path = matches.get_one::<String>("config_path").unwrap();

    let global_params = initialize_global_params(lib_path, config_path).await?;
    let server_manager = Arc::new(ServerManager::new(global_params));

    let discovery_params = DiscoveryParams {
        device_info: DeviceInfo {
            alias: "Rune Server".to_string(),
            version: "TP".to_string(),
            device_model: Some("Rune Server".to_string()),
            device_type: Some(DeviceType::Server),
            fingerprint: "FINGERPRINT".to_string(),
            api_port: 7863,
            protocol: "http".to_string(),
        },
    };

    server_manager
        .start(addr, discovery_params, ".known-clients")
        .await?;

    // Keep the main thread alive
    tokio::signal::ctrl_c().await?;
    server_manager.stop().await?;

    Ok(())
}

async fn initialize_global_params(lib_path: &str, config_path: &str) -> Result<Arc<GlobalParams>> {
    let db_path = format!("{}/.rune", lib_path);
    let db_connections = initialize_databases(lib_path, Some(&db_path)).await?;

    let main_db: Arc<MainDbConnection> = db_connections.main_db;
    let recommend_db: Arc<RecommendationDbConnection> = db_connections.recommend_db;
    let lib_path: Arc<String> = Arc::new(lib_path.to_string());
    let config_path: Arc<String> = Arc::new(config_path.to_string());

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

    let broadcaster = Arc::new(WebSocketService::new());
    let device_scanner = Arc::new(DeviceScanner::new(broadcaster.clone()));

    info!("Initializing Player events");
    tokio::spawn(initialize_player(
        lib_path.clone(),
        main_db.clone(),
        player.clone(),
        scrobbler.clone(),
        broadcaster.clone(),
    ));

    let global_params = Arc::new(GlobalParams {
        lib_path,
        config_path,
        main_db,
        recommend_db,
        main_token: main_cancel_token,
        task_tokens,
        player,
        sfx_player,
        scrobbler,
        broadcaster,
        device_scanner,
        server_manager: OnceLock::new(),
    });

    let server_manager = Arc::new(ServerManager::new(global_params.clone()));
    global_params
        .server_manager
        .set(server_manager.clone())
        .expect("Failed to set server manager in global params");

    Ok(global_params)
}
