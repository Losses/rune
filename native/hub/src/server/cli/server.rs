use std::net::SocketAddr;

use anyhow::Result;
use tokio::signal::ctrl_c;

use hub::server::utils::device::{RunMode, load_device_info};

use crate::initialize_global_params;

use ::discovery::{DiscoveryParams, config::get_config_dir};

pub async fn handle_server(addr: String, lib_path: String) -> Result<()> {
    let config_path = get_config_dir()?;
    let device_info = load_device_info(config_path, RunMode::Server).await?;
    let global_params = initialize_global_params(&lib_path, config_path.to_str().unwrap()).await?;

    let server_manager = global_params.node.manager()?;
    // The headless server exists to serve, whatever the settings saved by the app say, so
    // these apply to this process only and are not written back.
    let broadcast = {
        let mut settings = global_params.node.sync_settings.write().await;
        settings.media_server = true;
        settings.accept_remote_control = true;
        settings.accept_sync = true;
        settings.broadcast
    };
    if server_manager.is_running() {
        server_manager.stop().await?;
    }
    let socket_addr: SocketAddr = addr.parse()?;

    server_manager
        .clone()
        .start(socket_addr, DiscoveryParams { device_info }, broadcast)
        .await?;

    ctrl_c().await?;
    global_params.node.shutdown().await?;
    Ok(())
}
