use std::path::PathBuf;
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
            match fs.list_current_dir().await {
                Ok(entries) => {
                    for entry in entries {
                        let entry_type = if entry.is_directory { "DIR" } else { "FILE" };
                        let id_str = entry.id.map(|id| format!(" [{}]", id)).unwrap_or_default();
                        println!("{:<4} {}{}", entry_type, entry.name, id_str);
                    }
                }
                Err(e) => eprintln!("Error listing directory: {}", e),
            }
            Ok(true)
        }
        Command::Pwd => {
            let fs = fs.read().await;
            println!("Current directory: {}", fs.current_dir().to_string_lossy());
            Ok(true)
        }
        Command::Cd { path } => {
            let mut fs = fs.write().await;
            let new_path = match path.as_str() {
                "." => fs.current_path.clone(),
                ".." => {
                    if fs.current_path != std::path::Path::new("/") {
                        let mut new_path = fs.current_path.clone();
                        new_path.pop();
                        new_path
                    } else {
                        fs.current_path.clone()
                    }
                }
                "/" => PathBuf::from("/"),
                path => fs.current_path.join(path),
            };

            match fs.validate_path(&new_path).await {
                Ok(true) => {
                    fs.current_path = new_path;
                }
                Ok(false) => {
                    println!("Directory not found: {}", path);
                }
                Err(e) => {
                    println!("Error validating path: {}", e);
                }
            }
            Ok(true)
        }
        Command::Quit => Ok(false),
    }
}
