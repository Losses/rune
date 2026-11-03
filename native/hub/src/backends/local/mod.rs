use crate::server::sync_worker::PairedAudioResolver;
use crate::{
    Signal,
    utils::{GlobalParams, ParamsExtractor, TaskTokens, player::initialize_local_player},
};
use crate::{node::NodeContext, utils::RunningMode};
#[macro_use]
mod gui_request;

use std::sync::{Arc, OnceLock};

use crate::request_error::classify;
use log::{error, info, warn};
use tokio::sync::Mutex;

use rinf::{DartSignal, RustSignal};

pub use tokio;

use ::database::connection::MainDbConnection;
use ::playback::player::Player;
use ::playback::sfx_player::SfxPlayer;
use ::scrobbling::manager::ScrobblingManager;

use crate::library_pool::LibraryHandle;
use crate::messages::*;

pub async fn local_player_loop(
    library: LibraryHandle,
    node: Arc<NodeContext>,
    scrobbler: Arc<Mutex<ScrobblingManager>>,
) -> anyhow::Result<()> {
    info!("Media Library Received, initialize other receivers");

    info!("Initializing database");

    let main_db: Arc<MainDbConnection> = library.main_db.clone();

    let fsio = library.fsio.clone();
    let lib_path: Arc<String> = Arc::new(library.media_root.clone());
    let config_path = node.config_path.clone();
    let node_id = node.node_id.clone();
    let broadcaster = node.broadcaster.clone();
    let main_cancel_token = node.token.child_token();
    let library_sync = library.library_sync.clone();
    let task_tokens: Arc<Mutex<TaskTokens>> = Arc::new(Mutex::new(TaskTokens::default()));

    info!("Initializing player");
    let player = Player::new(Arc::clone(&fsio), Some(main_cancel_token.clone()));
    let player: Arc<Mutex<Player>> = Arc::new(Mutex::new(player));

    let sfx_player = SfxPlayer::new(Some(main_cancel_token.clone()));
    let sfx_player: Arc<Mutex<SfxPlayer>> = Arc::new(Mutex::new(sfx_player));

    let main_cancel_token = Arc::new(main_cancel_token);

    let device_scanner = node.device_scanner.clone();
    let permission_manager = node.permission_manager.clone();
    let cert_validator = node.cert_validator.clone();
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
    ));

    info!("Initializing UI events");
    let global_params = GlobalParams {
        node: node.clone(),
        sync_settings: node.sync_settings.clone(),
        library_id: Some(library.descriptor.library_id),
        library_pool: node.library_pool.clone(),
        library: Some(library.clone()),
        ui_library: Arc::new(Mutex::new(Some(library))),
        config_path,
        node_id,
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
    let server_manager = node.manager()?;
    global_params
        .server_manager
        .set(server_manager.clone())
        .map_err(|_| anyhow::anyhow!("Library server already initialized"))?;
    global_params
        .player
        .lock()
        .await
        .set_source_resolver(Arc::new(PairedAudioResolver(Arc::downgrade(
            &global_params,
        ))));
    node.attach_ui(&global_params).await;
    for_all_request_pairs2!(listen_local_gui_event, global_params, main_cancel_token);
    Ok(())
}
