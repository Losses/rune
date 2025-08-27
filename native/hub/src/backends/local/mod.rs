#[macro_use]
mod gui_request;

use std::sync::Arc;
use std::sync::OnceLock;

use fsio::FsIo;
use log::{error, info};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use rinf::{DartSignal, RustSignal};

pub use tokio;

use ::database::connection::{MainDbConnection, RecommendationDbConnection};
use ::discovery::client::CertValidator;
use ::discovery::protocol::DiscoveryService;
use ::discovery::server::PermissionManager;
use ::playback::player::Player;
use ::playback::sfx_player::SfxPlayer;
use ::scrobbling::manager::ScrobblingManager;

use crate::Signal;
use crate::listen_local_gui_event;
use crate::messages::*;
use crate::server::ServerManager;
use crate::utils::Broadcaster;
use crate::utils::DatabaseConnections;
use crate::utils::GlobalParams;
use crate::utils::ParamsExtractor;
use crate::utils::TaskTokens;
use crate::utils::nid::get_or_create_node_id;
use crate::utils::player::initialize_local_player;

pub async fn local_player_loop(
    fsio: Arc<FsIo>,
    lib_path: String,
    config_path: String,
    db_connections: DatabaseConnections,
    scrobbler: Arc<Mutex<ScrobblingManager>>,
    broadcaster: Arc<dyn Broadcaster>,
) {
    info!("Media Library Received, initialize other receivers");

    tokio::spawn(async move {
        info!("Initializing database");

        let main_db: Arc<MainDbConnection> = db_connections.main_db;

        let recommend_db: Arc<RecommendationDbConnection> = db_connections.recommend_db;

        let lib_path: Arc<String> = Arc::new(lib_path);
        let config_path: Arc<String> = Arc::new(config_path);

        let node_id = Arc::new(
            get_or_create_node_id(&fsio, &config_path)
                .await
                .unwrap()
                .to_string(),
        );

        let main_cancel_token = CancellationToken::new();
        let task_tokens: Arc<Mutex<TaskTokens>> = Arc::new(Mutex::new(TaskTokens::default()));

        info!("Initializing player");
        let player = Player::new(Some(main_cancel_token.clone()));
        let player: Arc<Mutex<Player>> = Arc::new(Mutex::new(player));

        let sfx_player = SfxPlayer::new(Some(main_cancel_token.clone()));
        let sfx_player: Arc<Mutex<SfxPlayer>> = Arc::new(Mutex::new(sfx_player));

        let main_cancel_token = Arc::new(main_cancel_token);

        let device_scanner = Arc::new(DiscoveryService::without_store());
        let permission_manager =
            Arc::new(RwLock::new(PermissionManager::new(&**config_path).unwrap()));
        let cert_validator = Arc::new(RwLock::new(
            CertValidator::new(&**config_path).await.unwrap(),
        ));

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

        info!("Initializing UI events");
        let global_params = GlobalParams {
            fsio,
            lib_path,
            config_path,
            node_id,
            main_db,
            recommend_db,
            main_token: Arc::clone(&main_cancel_token),
            task_tokens,
            player,
            sfx_player,
            scrobbler,
            broadcaster,
            device_scanner,
            cert_validator,
            permission_manager,
            server_manager: OnceLock::new(),
            running_mode: crate::utils::RunningMode::Client,
        };

        let global_params = Arc::new(global_params);
        let server_manager = Arc::new(ServerManager::new(global_params.clone()).await.unwrap());

        global_params
            .server_manager
            .set(server_manager.clone())
            .expect("Failed to set server manager in global params");

        for_all_request_pairs2!(listen_local_gui_event, global_params, main_cancel_token);
    });
}
