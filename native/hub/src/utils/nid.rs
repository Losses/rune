use std::{fs, path::Path};

use anyhow::{Context, Result};
use log::info;
use uuid::Uuid;

pub async fn get_or_create_node_id(config_path: &str) -> Result<Uuid> {
    let config_path = Path::new(config_path);
    let nid_path = config_path.join("nid");
    info!("Checking nid file at: {:?}", nid_path);

    let content = fs::read_to_string(&nid_path);
    let uuid = match content {
        Ok(content) => {
            let trimmed = content.trim();
            match Uuid::parse_str(trimmed) {
                Ok(uuid) => {
                    info!("Found valid UUID: {}", uuid);
                    uuid
                }
                Err(_) => {
                    info!("Invalid UUID in nid file, generating new one");
                    let new_uuid = Uuid::new_v4();
                    fs::write(&nid_path, new_uuid.to_string())
                        .context("Failed to write nid file")?;
                    new_uuid
                }
            }
        }
        Err(_) => {
            info!("nid file not found, creating new one");
            let new_uuid = Uuid::new_v4();
            fs::write(&nid_path, new_uuid.to_string()).context("Failed to write nid file")?;
            new_uuid
        }
    };

    Ok(uuid)
}
