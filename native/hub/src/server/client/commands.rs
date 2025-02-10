use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clean_path::Clean;
use colored::*;
use futures::future::OptionFuture;
use tokio::sync::Mutex;
use unicode_width::UnicodeWidthStr;

use discovery::verifier::CertValidator;

use crate::api::{
    operate_playback_with_mix_query_request, send_next_request, send_pause_request,
    send_play_request, send_previous_request, send_set_playback_mode_request,
};
use crate::cli::{Command, DiscoveryCmd, RemoteCmd};
use crate::discovery::DiscoveryRuntime;
use crate::utils::{load_device_info, print_device_details, print_device_table, AppState};

pub async fn execute(command: Command, state: Arc<AppState>) -> Result<bool> {
    match command {
        Command::Ls { long } => {
            let mut fs = state.fs.write().await;
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
            let fs = state.fs.read().await;
            println!("Current directory: {}", fs.current_dir().to_string_lossy());
            Ok(true)
        }
        Command::Cd { path, id } => {
            let mut fs = state.fs.write().await;

            let new_path = if id {
                // Resolve the path using ID mode
                fs.resolve_path_with_ids(&path).await?
            } else {
                // Original path resolution logic
                match path.as_str() {
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
                    path => fs.current_path.join(path).clean(),
                }
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
        Command::Opq {
            path,
            playback_mode,
            instant_play,
            operate_mode,
            id,
        } => {
            let mut fs = state.fs.write().await;
            let mut path = fs.current_path.join(path).clean();

            if id {
                path = fs.resolve_path_with_ids(&path.to_string_lossy()).await?;
            }

            match fs.path_to_query(&path).await {
                Ok(queries) => match operate_playback_with_mix_query_request(
                    queries,
                    playback_mode,
                    instant_play,
                    operate_mode,
                    &fs.connection,
                )
                .await
                {
                    Ok(_) => println!("Successfully updated playback queue"),
                    Err(e) => eprintln!("Failed to update playback queue: {}", e),
                },
                Err(e) => eprintln!("Error creating query from path: {}", e),
            }
            Ok(true)
        }
        Command::Play => {
            let fs = state.fs.read().await;
            send_play_request(&fs.connection).await?;

            Ok(true)
        }
        Command::Pause => {
            let fs = state.fs.read().await;
            send_pause_request(&fs.connection).await?;

            Ok(true)
        }
        Command::Next => {
            let fs = state.fs.read().await;
            send_next_request(&fs.connection).await?;

            Ok(true)
        }
        Command::Previous => {
            let fs = state.fs.read().await;
            send_previous_request(&fs.connection).await?;

            Ok(true)
        }
        Command::SetMode { mode } => {
            let fs = state.fs.read().await;
            send_set_playback_mode_request(mode, &fs.connection).await?;

            Ok(true)
        }
        Command::Discovery(subcmd) => {
            handle_discovery(subcmd, state.discovery.clone(), &state.config_dir).await
        }
        Command::Remote(subcmd) => {
            handle_remote(subcmd, state.discovery.clone(), &state.validator).await
        }

        Command::Quit => Ok(false),
        Command::Exit => todo!(),
        Command::Cdi { path: _ } => todo!(),
        Command::Opqi {
            path: _,
            playback_mode: _,
            instant_play: _,
            operate_mode: _,
        } => todo!(),
    }
}

async fn handle_discovery(
    cmd: DiscoveryCmd,
    discovery: Arc<Mutex<Option<DiscoveryRuntime>>>,
    config_path: &Path,
) -> Result<bool> {
    match cmd {
        DiscoveryCmd::Scan { duration } => {
            let device_info = load_device_info(config_path.to_path_buf()).await?;
            let rt = DiscoveryRuntime::new(config_path).await?;

            rt.start_service(device_info, Duration::from_secs(1))
                .await?;
            tokio::time::sleep(Duration::from_secs(duration)).await;
            rt.cancel_token.cancel();

            Ok(true)
        }
        DiscoveryCmd::List => {
            let rt = discovery.lock().await;
            let devices = OptionFuture::from(rt.as_ref().map(|r| r.store.load()))
                .await
                .transpose()?;

            print_device_table(devices.unwrap_or_default());
            Ok(true)
        }
        DiscoveryCmd::Stop => {
            let mut lock = discovery.lock().await;
            if let Some(rt) = lock.take() {
                rt.cancel_token.cancel();
                rt.service.shutdown().await;
            }
            Ok(true)
        }
    }
}

async fn handle_remote(
    cmd: RemoteCmd,
    discovery: Arc<Mutex<Option<DiscoveryRuntime>>>,
    validator: &CertValidator,
) -> Result<bool> {
    let lock = discovery.lock().await;
    let rt = lock
        .as_ref()
        .with_context(|| "Discovery service not running")?;
    let devices = rt.store.load().await?;

    match cmd {
        RemoteCmd::Inspect { index } => {
            let dev = devices.get(index - 1).with_context(|| "Invalid index")?;
            print_device_details(dev);
            Ok(true)
        }
        RemoteCmd::Trust { index, domains } => {
            let dev = devices.get(index - 1).with_context(|| "Invalid index")?;
            let hosts: Vec<String> = domains
                .map(|d| d.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_else(|| dev.ips.clone().into_iter().map(|x| x.to_string()).collect());

            validator.add_trusted_domains(hosts, &dev.fingerprint)?;
            Ok(true)
        }
        RemoteCmd::Untrust { fingerprint } => {
            validator.remove_fingerprint(&fingerprint)?;
            Ok(true)
        }
        RemoteCmd::Edit { fingerprint, hosts } => {
            let new_hosts: Vec<_> = hosts.split(',').map(|s| s.trim().to_string()).collect();
            validator.replace_hosts_for_fingerprint(&fingerprint, new_hosts)?;
            Ok(true)
        }
    }
}
