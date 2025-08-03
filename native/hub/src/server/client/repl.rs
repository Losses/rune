use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use clean_path::Clean;
use colored::*;
use unicode_width::UnicodeWidthStr;

use crate::api::{
    operate_playback_with_mix_query_request, send_next_request, send_pause_request,
    send_play_request, send_previous_request, send_set_playback_mode_request,
};
use crate::fs::VirtualEntry;
use crate::utils::AppState;

pub async fn handle_ls(state: Arc<AppState>, long: bool) -> Result<bool> {
    let mut fs = state.fs.write().await;
    match fs.list_current_dir().await {
        Ok(entries) => {
            if long {
                for entry in entries {
                    let entry_type = if entry.is_directory { "DIR" } else { "FILE" };
                    let id_str = entry.id.map(|id| format!(" [{id}]")).unwrap_or_default();
                    println!("{:<4} {}{}", entry_type, id_str, entry.name);
                }
            } else {
                let mut entries = entries;
                entries.sort_by(|a, b| a.name.cmp(&b.name));
                print_entries_grid(entries);
            }
        }
        Err(e) => eprintln!("Error listing directory: {e}"),
    }
    Ok(true)
}

fn print_entries_grid(entries: Vec<VirtualEntry>) {
    let term_width = term_size::dimensions().map(|(w, _)| w).unwrap_or(80);
    let column_spacing = 2;

    let entry_displays: Vec<(String, usize)> = entries
        .iter()
        .map(|e| {
            let id_str =
                e.id.map(|id| format!("[{id}] ").yellow().to_string())
                    .unwrap_or_default();
            let name = if e.is_directory {
                e.name.blue().bold().to_string()
            } else {
                e.name.clone()
            };
            let display = format!("{id_str}{name}");
            let width = e.id.map(|id| format!("[{id}] ").width()).unwrap_or(0) + e.name.width();
            (display, width)
        })
        .collect();

    let max_width = entry_displays.iter().map(|(_, w)| *w).max().unwrap_or(0);
    let column_width = max_width + column_spacing;
    let cols = std::cmp::max(1, term_width / column_width);

    let mut current_col = 0;
    for (display, width) in entry_displays {
        print!("{}{}", display, " ".repeat(column_width - width));
        current_col += 1;
        if current_col >= cols {
            println!();
            current_col = 0;
        }
    }
    if current_col != 0 {
        println!();
    }
}

pub async fn handle_pwd(state: Arc<AppState>) -> Result<bool> {
    let fs = state.fs.read().await;
    println!("Current directory: {}", fs.current_dir().to_string_lossy());
    Ok(true)
}

pub async fn handle_cd(state: Arc<AppState>, path: String, id: bool) -> Result<bool> {
    let mut fs = state.fs.write().await;
    let new_path = if id {
        fs.resolve_path_with_ids(&path).await?
    } else {
        resolve_normal_path(&fs.current_path, &path)
    };

    match fs.validate_path(&new_path).await {
        Ok(true) => {
            fs.current_path = new_path;
            Ok(true)
        }
        Ok(false) => {
            println!("Directory not found: {path}");
            Ok(true)
        }
        Err(e) => {
            println!("Error validating path: {e}");
            Ok(true)
        }
    }
}

fn resolve_normal_path(current_path: &Path, path: &str) -> PathBuf {
    match path {
        "." => current_path.to_path_buf(),
        ".." => current_path.parent().unwrap_or(current_path).to_path_buf(),
        "/" => PathBuf::from("/"),
        _ => current_path.join(path).clean(),
    }
}

pub async fn handle_opq(
    state: Arc<AppState>,
    path: String,
    playback_mode: crate::cli::PlaybackMode,
    instant_play: bool,
    operate_mode: crate::cli::OperateMode,
    id: bool,
) -> Result<bool> {
    let mut fs = state.fs.write().await;
    let mut path_obj = fs.current_path.join(&path).clean();

    if id {
        path_obj = fs
            .resolve_path_with_ids(&path_obj.to_string_lossy())
            .await?;
    }

    match fs.path_to_query(&path_obj).await {
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
            Err(e) => eprintln!("Failed to update playback queue: {e}"),
        },
        Err(e) => eprintln!("Error creating query from path: {e}"),
    }
    Ok(true)
}

pub async fn handle_play(state: Arc<AppState>) -> Result<bool> {
    let fs = state.fs.read().await;
    send_play_request(&fs.connection).await?;
    Ok(true)
}

pub async fn handle_pause(state: Arc<AppState>) -> Result<bool> {
    let fs = state.fs.read().await;
    send_pause_request(&fs.connection).await?;
    Ok(true)
}

pub async fn handle_next(state: Arc<AppState>) -> Result<bool> {
    let fs = state.fs.read().await;
    send_next_request(&fs.connection).await?;
    Ok(true)
}

pub async fn handle_previous(state: Arc<AppState>) -> Result<bool> {
    let fs = state.fs.read().await;
    send_previous_request(&fs.connection).await?;
    Ok(true)
}

pub async fn handle_setmode(state: Arc<AppState>, mode: crate::cli::PlaybackMode) -> Result<bool> {
    let fs = state.fs.read().await;
    send_set_playback_mode_request(mode, &fs.connection).await?;
    Ok(true)
}
