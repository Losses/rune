use crate::node::NodeContext;
use crate::utils::{GlobalParams, TaskTokens};
#[macro_use]
pub(crate) mod gui_request;

use std::sync::{Arc, OnceLock};

use crate::macros::listen_local_gui;
use log::info;
use tokio::sync::Mutex;

pub use tokio;

use ::playback::sfx_player::SfxPlayer;
use ::scrobbling::manager::ScrobblingManager;

use crate::library_pool::LibraryHandle;

pub async fn local_player_loop(
    library: LibraryHandle,
    node: Arc<NodeContext>,
    _scrobbler: Arc<Mutex<ScrobblingManager>>,
) -> anyhow::Result<()> {
    info!("Media Library Received, initialize other receivers");

    info!("Initializing database");

    let config_path = node.config_path.clone();
    let node_id = node.node_id.clone();
    let broadcaster = node.broadcaster.clone();
    let main_cancel_token = node.token.child_token();
    let task_tokens: Arc<Mutex<TaskTokens>> = node.task_tokens.clone();

    info!("Initializing player");
    let session = node.session()?;
    let player = session.player.clone();
    let scrobbler = session.scrobbler.clone();

    let sfx_player = SfxPlayer::new(Some(main_cancel_token.clone()));
    let sfx_player: Arc<Mutex<SfxPlayer>> = Arc::new(Mutex::new(sfx_player));

    let main_cancel_token = Arc::new(main_cancel_token);

    let device_scanner = node.device_scanner.clone();
    let permission_manager = node.permission_manager.clone();
    let cert_validator = node.cert_validator.clone();
    info!("Initializing UI events");
    let global_params = GlobalParams {
        node: node.clone(),
        sync_settings: node.sync_settings.clone(),
        library_id: Some(library.library_id),
        library_pool: node.library_pool.clone(),
        library: Some(library.clone()),
        ui_library: Arc::new(Mutex::new(Some(library))),
        config_path,
        node_id,
        main_token: Arc::clone(&main_cancel_token),
        task_tokens,
        library_tasks: node.library_tasks.clone(),
        player,
        sfx_player,
        scrobbler,
        broadcaster,
        device_scanner,
        cert_validator,
        permission_manager,
        server_manager: OnceLock::new(),
    };

    let global_params = Arc::new(global_params);
    let server_manager = node.manager()?;
    global_params
        .server_manager
        .set(server_manager.clone())
        .map_err(|_| anyhow::anyhow!("Library server already initialized"))?;
    node.attach_ui(&global_params).await;
    listen_local_gui(&global_params, &main_cancel_token);
    Ok(())
}
