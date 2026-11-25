use std::path::Path;

use anyhow::Result;

use crate::utils::sync_settings::SyncSettings;

/// Serializes tests that bind the fixed paired sync port.
pub(crate) static SYNC_PORT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Fixtures that need a paired listener start their manager on a loopback port.
pub(crate) fn disable_sync_listener(config_dir: &Path) -> Result<()> {
    SyncSettings {
        listen: false,
        ..SyncSettings::default()
    }
    .save(config_dir)
}
