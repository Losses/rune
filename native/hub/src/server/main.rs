use std::{
    net::SocketAddr,
    sync::{Arc, OnceLock},
    time::Duration,
};

use anyhow::Result;
use clap::{Arg, Command};
use log::info;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use hub::{
    server::{
        utils::{device::load_device_info, path::init_system_paths},
        ServerManager, WebSocketService,
    },
    utils::{
        device_scanner::DeviceScanner, initialize_databases, player::initialize_player,
        GlobalParams, TaskTokens,
    },
};

use ::database::connection::{MainDbConnection, RecommendationDbConnection};
use ::discovery::{
    permission::PermissionManager, udp_multicast::DiscoveryService, verifier::CertValidator,
    DiscoveryParams,
};
use ::playback::{player::Player, sfx_player::SfxPlayer};
use ::scrobbling::manager::ScrobblingManager;

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::new(
        "symphonia_format_ogg=off,symphonia_core=off,symphonia_bundle_mp3::demuxer=off,tantivy::directory=off,tantivy::indexer=off,sea_orm_migration::migrator=off,info",
    );

    tracing_subscriber::fmt().with_env_filter(filter).init();

    let matches = Command::new("Rune")
        .author("Rune Developers")
        .subcommand(
            Command::new("server")
                .about("Start the server")
                .arg(
                    Arg::new("addr")
                        .short('a')
                        .long("addr")
                        .value_name("ADDRESS")
                        .default_value("127.0.0.1:7863"),
                )
                .arg(
                    Arg::new("lib_path")
                        .value_name("LIB_PATH")
                        .required(true)
                        .index(1),
                ),
        )
        .subcommand(Command::new("broadcast").about("Broadcast presence to the local network"))
        .get_matches();

    match matches.subcommand() {
        Some(("server", sub_matches)) => {
            let addr: SocketAddr = sub_matches.get_one::<String>("addr").unwrap().parse()?;
            let lib_path = sub_matches.get_one::<String>("lib_path").unwrap();

            let config_path = init_system_paths()?;
            let device_info = load_device_info(config_path.clone()).await?;

            let global_params = initialize_global_params(
                lib_path,
                config_path
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("Invalid config path"))?,
            )
            .await?;

            let server_manager = Arc::new(ServerManager::new(global_params).await?);
            server_manager
                .start(addr, DiscoveryParams { device_info })
                .await?;

            tokio::signal::ctrl_c().await?;
            server_manager.stop().await?;
        }
        Some(("broadcast", _)) => {
            let config_path = init_system_paths()?;
            let device_info = load_device_info(config_path).await?;

            let (event_tx, _) = tokio::sync::mpsc::channel(32);
            let discovery_service = Arc::new(DiscoveryService::new(event_tx));

            let cancel_token = CancellationToken::new();
            let handle = tokio::spawn({
                let cancel_token = cancel_token.clone();
                let discovery_service = discovery_service.clone();
                let device_info = device_info.clone();
                async move {
                    loop {
                        if let Err(e) = discovery_service.announce(device_info.clone()).await {
                            eprintln!("Failed to announce: {}", e);
                        }

                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                            _ = cancel_token.cancelled() => break,
                        }
                    }
                }
            });

            tokio::signal::ctrl_c().await?;
            cancel_token.cancel();
            handle.await?;

            discovery_service.shutdown().await;
        }
        _ => {
            Command::new("Rune").print_help()?;
        }
    }

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

    let permission_manager = Arc::new(RwLock::new(PermissionManager::new(config_path.as_str())?));
    let cert_validator = Arc::new(RwLock::new(CertValidator::new(config_path.as_str())?));

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
        cert_validator,
        permission_manager,
        server_manager: OnceLock::new(),
    });

    let server_manager = Arc::new(ServerManager::new(global_params.clone()).await?);
    global_params
        .server_manager
        .set(server_manager.clone())
        .expect("Failed to set server manager in global params");

    Ok(global_params)
}
