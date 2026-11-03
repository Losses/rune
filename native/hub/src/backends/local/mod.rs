use crate::utils::RunningMode;
use crate::{
    Signal,
    server::{ServerManager, utils::device::load_device_info},
    utils::{
        Broadcaster, GlobalParams, ParamsExtractor, TaskTokens, nid::get_or_create_node_id,
        player::initialize_local_player, sync_settings::SyncSettings,
    },
};
use std::net::SocketAddr;
use std::path::Path;
#[macro_use]
mod gui_request;

use std::sync::{Arc, OnceLock};

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

use crate::library_pool::LibraryHandle;
use crate::messages::*;

pub async fn local_player_loop(
    library: LibraryHandle,
    config_path: String,
    scrobbler: Arc<Mutex<ScrobblingManager>>,
    broadcaster: Arc<dyn Broadcaster>,
) {
    info!("Media Library Received, initialize other receivers");

    tokio::spawn(async move {
        info!("Initializing database");

        let main_db: Arc<MainDbConnection> = library.main_db.clone();

        let recommend_db: Arc<RecommendationDbConnection> = library.recommend_db.clone();

        let fsio = library.fsio.clone();
        let lib_path: Arc<String> = Arc::new(library.media_root.clone());
        let config_path: Arc<String> = Arc::new(config_path);

        let node_id = Arc::new(
            get_or_create_node_id(&config_path)
                .await
                .unwrap()
                .to_string(),
        );

        let main_cancel_token = CancellationToken::new();
        let library_sync = library.library_sync.clone();
        let task_tokens: Arc<Mutex<TaskTokens>> = Arc::new(Mutex::new(TaskTokens::default()));

        info!("Initializing player");
        let player = Player::new(Arc::clone(&fsio), Some(main_cancel_token.clone()));
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
            library_sync.record_store(),
            library_sync.record_store().author(),
            player.clone(),
            scrobbler.clone(),
            broadcaster.clone(),
            cert_validator.clone(),
            permission_manager.clone(),
        ));

        info!("Initializing UI events");
        let global_params = GlobalParams {
            sync_settings: Arc::new(RwLock::new(SyncSettings::load(Path::new(&**config_path)))),
            fsio,
            lib_path,
            library_id: Some(library.descriptor.library_id),
            library_pool: library.pool(),
            ui_library: Arc::new(Mutex::new(Some(library))),
            config_path,
            node_id,
            main_db,
            library_sync: Some(library_sync),
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
            running_mode: RunningMode::Client,
        };

        let global_params = Arc::new(global_params);
        let server_manager = Arc::new(ServerManager::new(global_params.clone()).await.unwrap());

        global_params
            .server_manager
            .set(server_manager.clone())
            .expect("Failed to set server manager in global params");

        if let Err(error) = server_manager.start_worker().await {
            log::warn!("Could not start paired sync worker: {error:#}");
        }
        if global_params.sync_settings.read().await.listen {
            match load_device_info(Path::new(&*global_params.config_path)).await {
                Ok(info) => {
                    let addr = SocketAddr::from(([0, 0, 0, 0], 7863));
                    if let Err(error) = server_manager
                        .clone()
                        .start_sync(addr, discovery::DiscoveryParams { device_info: info })
                        .await
                    {
                        log::warn!("Could not start paired sync server: {error}");
                    }
                }
                Err(error) => log::warn!("Could not initialize paired sync identity: {error}"),
            }
        }
        for_all_request_pairs2!(listen_local_gui_event, global_params, main_cancel_token);
    });
}
