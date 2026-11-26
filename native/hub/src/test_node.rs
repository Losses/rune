use std::path::Path;

use anyhow::Result;

use crate::node::NodeContext;
use crate::utils::sync_settings::SyncSettings;

/// Serializes tests that bind the fixed device port.
pub(crate) static SYNC_PORT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Fixtures that need a server start their manager on a loopback port.
pub(crate) fn disable_media_server(config_dir: &Path) -> Result<()> {
    SyncSettings {
        media_server: false,
        ..SyncSettings::default()
    }
    .save(config_dir)
}

/// Makes the settings of a node whose server a fixture started by hand describe that server:
/// it runs, and it accepts remote control when the test drives the node from a controller.
pub(crate) async fn describe_running_server(node: &NodeContext, accept_remote_control: bool) {
    let mut settings = node.sync_settings.write().await;
    settings.media_server = true;
    settings.accept_remote_control = accept_remote_control;
}
