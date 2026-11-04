mod cli;

use std::path::Path;
use std::sync::{Arc, OnceLock};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use log::info;
use rustls::crypto::ring::default_provider;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

use cli::{
    broadcast::handle_broadcast, chpwd::handle_chpwd, permission::handle_permission,
    server::handle_server,
};
use hub::{
    node::initialize_node,
    server::WebSocketService,
    utils::{GlobalParams, RunningMode, TaskTokens},
};

use ::database::connection::check_storage_mode;
use ::database::library_registry::{LibraryRegistry, resolve_library};
use ::fsio::FsIo;
use ::playback::sfx_player::SfxPlayer;

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

    let node = initialize_node(config_path, config_path, Arc::new(WebSocketService::new())).await?;
    let node_id = node.node_id.clone();

    let mode = check_storage_mode(&fsio, lib_path)?;
    let mut registry = LibraryRegistry::load(Path::new(config_path))?;
    let descriptor = resolve_library(&fsio, &mut registry, lib_path, "", mode).await?;
    info!("Resolved library id: {}", descriptor.library_id);

    let pool = node.library_pool.clone();
    let library = pool.acquire_ui(descriptor.library_id).await?;
    let config_path = node.config_path.clone();

    let main_cancel_token = node.token.child_token();
    let task_tokens: Arc<Mutex<TaskTokens>> = Arc::new(Mutex::new(TaskTokens::default()));

    info!("Initializing player");
    let session = node.session()?;
    session.migrate_legacy(&library).await?;
    let player = session.player.clone();
    let scrobbler = session.scrobbler.clone();

    let sfx_player = SfxPlayer::new(Some(main_cancel_token.clone()));
    let sfx_player: Arc<Mutex<SfxPlayer>> = Arc::new(Mutex::new(sfx_player));

    let main_cancel_token = Arc::new(main_cancel_token);

    let broadcaster = node.broadcaster.clone();
    let device_scanner = node.device_scanner.clone();
    let permission_manager = node.permission_manager.clone();
    let cert_validator = node.cert_validator.clone();

    let global_params = Arc::new(GlobalParams {
        node: node.clone(),
        library_id: Some(descriptor.library_id),
        library_pool: pool,
        library: Some(library.clone()),
        ui_library: Arc::new(Mutex::new(Some(library))),
        sync_settings: node.sync_settings.clone(),
        config_path,
        node_id,
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

    let server_manager = node.manager()?;
    node.attach_ui(&global_params).await;
    global_params
        .server_manager
        .set(server_manager.clone())
        .map_err(|_| anyhow::anyhow!("Library server already initialized"))?;

    Ok(global_params)
}
