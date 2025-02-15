use std::{
    net::SocketAddr,
    sync::{Arc, OnceLock},
    time::Duration,
};

use anyhow::Result;
use clap::{Parser, Subcommand};
use log::info;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use hub::{
    server::{
        utils::{
            device::load_device_info, path::get_config_dir, permission::{parse_status, print_permission_table, validate_index},
        },
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
    let cli = Cli::parse();

    match cli.command {
        Commands::Server { addr, lib_path } => handle_server(addr, lib_path).await?,
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

async fn handle_server(addr: String, lib_path: String) -> Result<()> {
    let config_path = get_config_dir()?;
    let device_info = load_device_info(&config_path).await?;
    let global_params = initialize_global_params(&lib_path, config_path.to_str().unwrap()).await?;

    let server_manager = Arc::new(ServerManager::new(global_params).await?);
    let socket_addr: SocketAddr = addr.parse()?;

    server_manager
        .start(socket_addr, DiscoveryParams { device_info })
        .await?;

    tokio::signal::ctrl_c().await?;
    server_manager.stop().await?;
    Ok(())
}

async fn handle_broadcast() -> Result<()> {
    let config_path = get_config_dir()?;
    let device_info = load_device_info(&config_path).await?;

    let (event_tx, _) = broadcast::channel(32);
    let discovery_service = Arc::new(DiscoveryService::new(event_tx));
    let cancel_token = CancellationToken::new();

    let handle = tokio::spawn({
        let cancel_token = cancel_token.clone();
        let discovery_service = discovery_service.clone();
        let device_info = device_info.clone();
        async move {
            info!("Announcing this server as {}", device_info.alias);
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
    Ok(())
}

async fn handle_permission(action: PermissionAction) -> Result<()> {
    let config_path = get_config_dir()?;
    let mut pm = PermissionManager::new(config_path)?;

    match action {
        PermissionAction::Ls => {
            let users = pm.list_users().await;
            print_permission_table(&users);
        }
        PermissionAction::Modify { index, status } => {
            let users = pm.list_users().await;
            validate_index(index, users.len())?;
            let user = &users[index - 1];
            let status = parse_status(&status)?;
            pm.change_user_status(&user.fingerprint, status).await?;
            println!("User status updated successfully");
        }
        PermissionAction::Delete { index } => {
            let users = pm.list_users().await;
            validate_index(index, users.len())?;
            let user = &users[index - 1];
            pm.remove_user(&user.fingerprint).await?;
            println!("User deleted successfully");
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
