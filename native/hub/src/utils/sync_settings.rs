use anyhow::{Context, Result};
use database::actions::metadata::MissingHostFilePolicy;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncSettings {
    /// Whether this device listens for other devices at all. Files written before the server
    /// had one mode call this `listen`.
    #[serde(alias = "listen")]
    pub media_server: bool,
    /// While the server runs, announce this device on the local network.
    pub broadcast: bool,
    /// While the server runs, accept remote-control connections.
    pub accept_remote_control: bool,
    /// While the server runs, accept library sync requests from other devices.
    pub accept_sync: bool,
    pub missing_host_file: MissingHostFilePolicy,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            media_server: true,
            broadcast: true,
            accept_remote_control: false,
            accept_sync: true,
            missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
        }
    }
}

impl SyncSettings {
    pub fn load(config_path: &Path) -> Self {
        let path = config_path.join("sync_settings.json");
        match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(settings) => settings,
                Err(error) => {
                    log::warn!("Invalid sync settings: {error}");
                    Self::default()
                }
            },
            Err(error) => {
                if error.kind() != ErrorKind::NotFound {
                    log::warn!("Could not read sync settings: {error}");
                }
                Self::default()
            }
        }
    }
    pub fn save(&self, config_path: &Path) -> Result<()> {
        let path = config_path.join("sync_settings.json");
        let temp = config_path.join(format!(".sync_settings.{}.tmp", Uuid::new_v4()));
        fs::write(&temp, serde_json::to_vec(self)?).context("write sync settings")?;
        if let Err(error) = fs::rename(&temp, &path) {
            let _ = fs::remove_file(&temp);
            return Err(error).context("replace sync settings");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn defaults_and_atomic_round_trip() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let defaults = SyncSettings::load(dir.path());
        assert!(defaults.media_server);
        assert!(defaults.broadcast);
        assert!(!defaults.accept_remote_control);
        assert!(defaults.accept_sync);
        fs::write(dir.path().join("sync_settings.json"), b"{\"listen\":false}")?;
        let partial = SyncSettings::load(dir.path());
        assert!(!partial.media_server);
        assert!(partial.broadcast);
        assert!(!partial.accept_remote_control);
        assert!(partial.accept_sync);
        assert_eq!(
            partial.missing_host_file,
            MissingHostFilePolicy::RemoveEverywhere
        );
        let updated = SyncSettings {
            media_server: false,
            broadcast: false,
            accept_remote_control: true,
            accept_sync: false,
            missing_host_file: MissingHostFilePolicy::KeepAsRemote,
        };
        updated.save(dir.path())?;
        let loaded = SyncSettings::load(dir.path());
        assert!(!loaded.media_server);
        assert!(!loaded.broadcast);
        assert!(loaded.accept_remote_control);
        assert!(!loaded.accept_sync);
        assert_eq!(
            loaded.missing_host_file,
            MissingHostFilePolicy::KeepAsRemote
        );
        fs::write(dir.path().join("sync_settings.json"), b"invalid")?;
        assert!(SyncSettings::load(dir.path()).media_server);
        Ok(())
    }
}
