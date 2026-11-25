use std::fs;
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use directories::ProjectDirs;
use once_cell::sync::OnceCell;

static CONFIG_DIR: OnceCell<PathBuf> = OnceCell::new();

pub fn get_config_dir() -> Result<&'static PathBuf> {
    CONFIG_DIR.get_or_try_init(|| {
        let proj_dirs = ProjectDirs::from("ci", "not", "rune")
            .ok_or_else(|| anyhow!("Failed to get project directories"))?;

        let config_dir = proj_dirs.config_dir();
        let config_path = config_dir.to_path_buf();

        if config_path.exists() {
            if config_path.is_file() {
                return Err(anyhow!("Config directory path is a file: {config_path:?}"));
            }
        } else {
            fs::create_dir_all(&config_path)?;
        }

        Ok(config_path)
    })
}

// Alternative: If you prefer to return owned PathBuf and handle the error differently
pub fn get_config_dir_owned() -> Result<PathBuf> {
    get_config_dir().cloned()
}
