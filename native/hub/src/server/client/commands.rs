use std::sync::Arc;

use tokio::sync::RwLock;

use crate::cli::Command;
use crate::fs::VirtualFS;

pub async fn execute(
    command: Command,
    fs: &Arc<RwLock<VirtualFS>>,
) -> Result<bool, Box<dyn std::error::Error>> {
    match command {
        Command::Ls => {
            let fs = fs.read().await;
            let entries = fs.list_current_dir().await;
            for entry in entries {
                let entry_type = if entry.is_directory { "DIR" } else { "FILE" };
                let id_str = entry.id.map(|id| format!(" [{}]", id)).unwrap_or_default();
                println!("{:<4} {}{}", entry_type, entry.name, id_str);
            }
            Ok(true)
        }
        Command::Pwd => {
            let fs = fs.read().await;
            println!("Current directory: {}", fs.current_dir());
            Ok(true)
        }
        Command::Cd { path } => {
            let mut fs = fs.write().await;
            match path.as_str() {
                ".." => {
                    if fs.current_path != std::path::Path::new("/") {
                        fs.current_path.pop();
                    }
                }
                "/" => {
                    fs.current_path = std::path::PathBuf::from("/");
                }
                _ => {
                    if fs.root_dirs.contains(&path) {
                        fs.current_path = std::path::PathBuf::from("/").join(path);
                    } else {
                        println!("Directory not found: {}", path);
                    }
                }
            }
            Ok(true)
        }
        Command::Quit => Ok(false),
    }
}
