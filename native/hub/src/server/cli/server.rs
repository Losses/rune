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
    if server_manager.is_running() {
        server_manager.stop().await?;
    }
    let socket_addr: SocketAddr = addr.parse()?;

    server_manager
        .clone()
        .start(socket_addr, DiscoveryParams { device_info })
        .await?;

    ctrl_c().await?;
    global_params.node.shutdown().await?;
    Ok(())
}
