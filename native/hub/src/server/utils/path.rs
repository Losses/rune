use std::path::PathBuf;

use directories::ProjectDirs;

pub fn init_system_paths() -> anyhow::Result<PathBuf> {
    let proj_dirs = ProjectDirs::from("", "rune.not.ci", "cli")
        .ok_or_else(|| anyhow::anyhow!("Failed to get project directories"))?;

    let config_dir = proj_dirs.config_dir();

    Ok(config_dir.to_path_buf())
}
