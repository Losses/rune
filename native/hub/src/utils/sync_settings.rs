use anyhow::{Context, Result};
use database::actions::metadata::MissingHostFilePolicy;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncSettings {
    pub listen: bool,
    pub missing_host_file: MissingHostFilePolicy,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            listen: true,
            missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
        }
    }
}

impl SyncSettings {
    pub fn load(config_path: &Path) -> Self {
        let path = config_path.join("sync_settings.json");
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(settings) => settings,
                Err(error) => {
                    log::warn!("Invalid sync settings: {error}");
                    Self::default()
                }
            },
            Err(error) => {
                log::warn!("Could not read sync settings: {error}");
                Self::default()
            }
        }
    }
    pub fn save(&self, config_path: &Path) -> Result<()> {
        let path = config_path.join("sync_settings.json");
        let temp = config_path.join(format!(".sync_settings.{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&temp, serde_json::to_vec(self)?).context("write sync settings")?;
        if let Err(error) = std::fs::rename(&temp, &path) {
            let _ = std::fs::remove_file(&temp);
            return Err(error).context("replace sync settings");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_atomic_round_trip() -> Result<()> {
        let dir = tempfile::tempdir()?;
        assert!(SyncSettings::load(dir.path()).listen);
        std::fs::write(dir.path().join("sync_settings.json"), b"{\"listen\":false}")?;
        let partial = SyncSettings::load(dir.path());
        assert!(!partial.listen);
        assert_eq!(
            partial.missing_host_file,
            MissingHostFilePolicy::RemoveEverywhere
        );
        let updated = SyncSettings {
            listen: true,
            missing_host_file: MissingHostFilePolicy::KeepAsRemote,
        };
        updated.save(dir.path())?;
        let loaded = SyncSettings::load(dir.path());
        assert!(loaded.listen);
        assert_eq!(
            loaded.missing_host_file,
            MissingHostFilePolicy::KeepAsRemote
        );
        std::fs::write(dir.path().join("sync_settings.json"), b"invalid")?;
        assert!(SyncSettings::load(dir.path()).listen);
        Ok(())
    }
}
