use std::time::Duration;

use anyhow::Result;
use tokio::signal::ctrl_c;

use hub::server::utils::device::load_device_info;

use ::discovery::{config::get_config_dir, protocol::DiscoveryService};

pub async fn handle_broadcast() -> Result<()> {
    let config_path = get_config_dir()?;
    let device_info = load_device_info(config_path).await?;

    let discovery_service = DiscoveryService::with_store(config_path).await?;

    discovery_service
        .start_announcements(device_info, Duration::from_secs(3), None)
        .await?;

    ctrl_c().await?;
    Ok(())
}
