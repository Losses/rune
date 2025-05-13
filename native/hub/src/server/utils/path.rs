use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use directories::ProjectDirs;

pub fn get_config_dir() -> Result<PathBuf> {
    let proj_dirs = ProjectDirs::from("ci", "not", "rune")
        .ok_or_else(|| anyhow!("Failed to get project directories"))?;

    let config_dir = proj_dirs.config_dir();
    let config_path = config_dir.to_path_buf();

    if config_path.exists() {
        if config_path.is_file() {
            panic!("Config directory path is a file: {:?}", config_path);
        }
    } else {
        fs::create_dir_all(&config_path)?;
    }

    Ok(config_path)
}
