use std::path::Path;

use anyhow::{Context, Result};
use log::info;
use uuid::Uuid;

/// The nid file lives in the app's local config directory, so it always goes
/// through the host filesystem directly — never through FsIo, which on
/// Android is rooted at the SAF tree and cannot see local paths.
pub async fn get_or_create_node_id(config_path: &str) -> Result<Uuid> {
    let nid_path = Path::new(config_path).join("nid");

    if let Some(parent) = nid_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if !tokio::fs::try_exists(&nid_path).await? {
        tokio::fs::write(&nid_path, b"").await?;
    }
    info!("Checking nid file at: {nid_path:?}");

    let content = tokio::fs::read_to_string(&nid_path).await?;
    let uuid = match Uuid::parse_str(content.trim()) {
        Ok(uuid) => {
            info!("Found valid UUID: {uuid}");
            uuid
        }
        Err(_) => {
            info!("nid file missing or invalid, generating a new one");
            let new_uuid = Uuid::new_v4();
            tokio::fs::write(&nid_path, new_uuid.to_string())
                .await
                .context("Failed to write nid file")?;
            new_uuid
        }
    };

    Ok(uuid)
}
