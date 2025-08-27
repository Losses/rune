mod cli;

#[cfg(target_os = "android")]
use std::path::Path;
use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use log::info;
use rustls::crypto::ring::default_provider;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use cli::{
    broadcast::handle_broadcast, chpwd::handle_chpwd, permission::handle_permission,
    server::handle_server,
};
use hub::{
    server::{ServerManager, WebSocketService},
    utils::{
        GlobalParams, RunningMode, TaskTokens, initialize_databases, nid::get_or_create_node_id,
        player::initialize_local_player,
    },
};

use ::database::connection::{MainDbConnection, RecommendationDbConnection};
use ::discovery::{client::CertValidator, protocol::DiscoveryService, server::PermissionManager};
use ::fsio::FsIo;
use ::playback::{player::Player, sfx_player::SfxPlayer};
use ::scrobbling::manager::ScrobblingManager;

#[derive(Parser)]
#[command(name = "Rune", author = "Rune Developers", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the server
    Server {
        #[arg(short, long, default_value = "127.0.0.1:7863")]
        addr: String,
        #[arg(required = true, index = 1)]
        lib_path: String,
    },
    /// Initialize or change root password
    Chpwd,
    /// Broadcast presence to the local network
    Broadcast,
    /// Manage device permissions
    Permission {
        #[command(subcommand)]
        action: PermissionAction,
    },
}

#[derive(Subcommand)]
enum PermissionAction {
    /// List all permissions and statuses
    Ls,
    /// Modify user status
    Modify {
        /// User index number
        #[arg(value_name = "INDEX")]
        index: usize,
        /// New status (approved/pending/blocked)
        #[arg(value_name = "STATUS")]
        status: String,
    },
    /// Delete user permission
    Delete {
        /// User index number
        #[arg(value_name = "INDEX")]
        index: usize,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_logging();

    if let Err(e) = default_provider().install_default() {
        bail!(format!("{e:#?}"));
    };

    let cli = Cli::parse();

    match cli.command {
        Commands::Server { addr, lib_path } => handle_server(addr, lib_path).await?,
        Commands::Chpwd => handle_chpwd().await?,
        Commands::Broadcast => handle_broadcast().await?,
        Commands::Permission { action } => handle_permission(action).await?,
    }

    Ok(())
}

fn setup_logging() {
    let filter = EnvFilter::new(
        "symphonia_format_ogg=off,symphonia_core=off,symphonia_bundle_mp3::demuxer=off,\
         tantivy::directory=off,tantivy::indexer=off,sea_orm_migration::migrator=off,info",
    );
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

async fn initialize_global_params(lib_path: &str, config_path: &str) -> Result<Arc<GlobalParams>> {
    #[cfg(not(target_os = "android"))]
    let fsio = Arc::new(FsIo::new());
    #[cfg(target_os = "android")]
    let fsio = Arc::new(FsIo::new(Path::new(".rune/.android-fs.db"), &lib_path)?);

    let db_path = format!("{lib_path}/.rune");
    let node_id = Arc::new(get_or_create_node_id(&fsio, config_path).await?.to_string());

    let db_connections = initialize_databases(&fsio, lib_path, Some(&db_path), &node_id).await?;

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
    let device_scanner = Arc::new(DiscoveryService::without_store());

    let permission_manager = Arc::new(RwLock::new(PermissionManager::new(config_path.as_str())?));
    let cert_validator = Arc::new(RwLock::new(CertValidator::new(config_path.as_str()).await?));

    info!("Initializing Player events");
    tokio::spawn(initialize_local_player(
        fsio.clone(),
        lib_path.clone(),
        main_db.clone(),
        player.clone(),
        scrobbler.clone(),
        broadcaster.clone(),
        cert_validator.clone(),
        permission_manager.clone(),
    ));

    let global_params = Arc::new(GlobalParams {
        fsio,
        lib_path,
        config_path,
        node_id,
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
        running_mode: RunningMode::Server,
    });

    let server_manager = Arc::new(ServerManager::new(global_params.clone()).await?);
    global_params
        .server_manager
        .set(server_manager.clone())
        .expect("Failed to set server manager in global params");

    Ok(global_params)
}
