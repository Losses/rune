use std::path::PathBuf;
use std::sync::Arc;

use colored::*;
use tokio::sync::RwLock;
use unicode_width::UnicodeWidthStr;

use crate::cli::Command;
use crate::fs::VirtualFS;

pub async fn execute(
    command: Command,
    fs: &Arc<RwLock<VirtualFS>>,
) -> Result<bool, Box<dyn std::error::Error>> {
    match command {
        Command::Ls { long } => {
            let mut fs = fs.write().await;
            match fs.list_current_dir().await {
                Ok(entries) => {
                    if long {
                        // Detailed mode (ls -l)
                        for entry in entries {
                            let entry_type = if entry.is_directory { "DIR" } else { "FILE" };
                            let id_str =
                                entry.id.map(|id| format!(" [{}]", id)).unwrap_or_default();
                            println!("{:<4} {}{}", entry_type, id_str, entry.name);
                        }
                    } else {
                        // Simple mode (ls)
                        let mut entries = entries;
                        entries.sort_by(|a, b| a.name.cmp(&b.name));

                        // Calculate terminal width
                        let term_width = term_size::dimensions().map(|(w, _)| w).unwrap_or(80);

                        let column_spacing = 2;

                        // Calculate full display string and its width for each entry
                        let entry_displays: Vec<(String, usize)> = entries
                            .iter()
                            .map(|e| {
                                let id_str =
                                    e.id.map(|id| format!("[{}] ", id).yellow().to_string())
                                        .unwrap_or_default();

                                let name = if e.is_directory {
                                    e.name.blue().bold().to_string()
                                } else {
                                    e.name.clone()
                                };

                                let display = format!("{}{}", id_str, name);
                                let width =
                                    e.id.map(|id| format!("[{}] ", id).width()).unwrap_or(0)
                                        + e.name.width();

                                (display, width)
                            })
                            .collect();

                        // Calculate the width of the longest entry
                        let max_display_width = entry_displays
                            .iter()
                            .map(|(_, width)| *width)
                            .max()
                            .unwrap_or(0);

                        let column_width = max_display_width + column_spacing;

                        // Calculate the number of entries per line
                        let cols = std::cmp::max(1, term_width / column_width);

                        // Display entries
                        let mut current_col = 0;
                        for (display, width) in entry_displays {
                            print!("{}", display);

                            // Calculate and print padding
                            let padding = column_width.saturating_sub(width);
                            print!("{}", " ".repeat(padding));

                            current_col += 1;
                            if current_col >= cols {
                                println!();
                                current_col = 0;
                            }
                        }

                        // Print a newline if the last line is incomplete
                        if current_col != 0 {
                            println!();
                        }
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
