use database::actions::file::get_file_by_id;
use database::actions::metadata::{get_metadata_summary_by_file_id, MetadataSummary};
use dunce::canonicalize;
use log::{info, debug, error};
use playback::player::Player;
use sea_orm::DatabaseConnection;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::task;

use crate::common::Result;
use crate::connection;
use crate::messages;

pub async fn handle_playback(db: Arc<DatabaseConnection>) -> Result<()> {
    use messages::playback::*;

    info!("Initializing player.");
    let _player = Player::new();
    let player = Arc::new(Mutex::new(_player));

    info!("Initializing playback receiver.");
    let mut ui_receiver = PlayFileRequest::get_dart_signal_receiver()?; // GENERATED

    let mut status_receiver = player.lock().unwrap().subscribe();

    tokio::spawn({
        let player = Arc::clone(&player);
        let db = Arc::clone(&db);
        async move {
            while let Some(dart_signal) = ui_receiver.recv().await {
                let play_file_request = dart_signal.message;
                let file_id = play_file_request.file_id;
                let lib_path = connection::get_media_library_path().await;

                play_file_by_id(&db, &player, file_id, Path::new(&lib_path.unwrap())).await;
            }
        }
    });

    info!("Initializing event listeners.");
    task::spawn(async move {
        let db = Arc::clone(&db);
        let mut cached_meta: Option<MetadataSummary> = None;
        let mut last_id: Option<i32> = None;

        while let Ok(status) = status_receiver.recv().await {
            debug!("Player status updated: {:?}", status);

            let meta = match status.id {
                Some(id) => {
                    if last_id != Some(id) {
                        println!("= CACHE NOW!");
                        // Update the cached metadata if the index has changed
                        match get_metadata_summary_by_file_id(&db, id).await {
                            Ok(metadata) => {
                                cached_meta = Some(metadata);
                                last_id = Some(id);
                                info!("{:#?}", cached_meta);
                            }
                            Err(e) => {
                                // Print the error if get_metadata_summary_by_file_id returns an error
                                error!("Error fetching metadata: {:?}", e);
                                cached_meta = None;
                                last_id = Some(id);
                            }
                        }
                    }
                    cached_meta.clone().unwrap_or_default()
                }
                none => {
                    // If the index is None, send empty metadata
                    last_id = none;
                    MetadataSummary::default()
                }
            };

            let position = status.position;
            let duration = meta.duration;
            let progress_percentage = if duration == 0. {
                0.
            } else {
                position.as_secs_f32() / (duration as f32)
            };

            messages::playback::PlaybackStatus {
                state: status.state.to_string(),
                progress_seconds: position.as_secs_f32(),
                progress_percentage,
                artist: meta.artist.clone(),
                album: meta.album.clone(),
                title: meta.title.clone(),
                duration: meta.duration,
            }
            .send_signal_to_dart();
        }
    });

    Ok(())
}

pub async fn play_file_by_id(
    db: &DatabaseConnection,
    player: &Mutex<Player>,
    file_id: i32,
    canonicalized_path: &Path,
) {
    match get_file_by_id(db, file_id).await {
        Ok(Some(file)) => {
            let player_guard = player.lock().unwrap();
            player_guard.pause();
            player_guard.clear_playlist();

            let file_path =
                canonicalize(canonicalized_path.join(file.directory).join(file.file_name)).unwrap();
            player_guard.add_to_playlist(file_id, file_path);
            player_guard.play();
        }
        Ok(_none) => {
            eprintln!("File with ID {} not found", file_id);
        }
        Err(e) => {
            eprintln!("Error retrieving file with ID {}: {}", file_id, e);
        }
    }
}
