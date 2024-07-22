use database::actions::file::get_file_by_id;
use dunce::canonicalize;
use log::{error, info};
use playback::player::Player;
use playback::PlayerEvent;
use sea_orm::DatabaseConnection;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::common::Result;
use crate::connection;
use crate::messages;

pub async fn handle_playback(db: Arc<DatabaseConnection>) -> Result<()> {
    use messages::playback::*;

    info!("Initializing player.");
    let (player, mut events) = Player::new();
    let player = Arc::new(Mutex::new(player));
    
    info!("Initializing playback receiver.");
    let mut receiver = PlayFileRequest::get_dart_signal_receiver()?; // GENERATED
    
    tokio::spawn({
        let player = Arc::clone(&player);
        let db = Arc::clone(&db);
        async move {
            while let Some(dart_signal) = receiver.recv().await {
                let play_file_request = dart_signal.message;
                let file_id = play_file_request.file_id;
                let lib_path = connection::get_media_library_path().await;
                
                play_file_by_id(&db, &player, file_id, Path::new(&lib_path.unwrap())).await;
            }
        }
    });
    
    info!("Initializing event listeners.");
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                PlayerEvent::Playing => println!("Playing"),
                PlayerEvent::Paused => println!("Paused"),
                PlayerEvent::Stopped => println!("Stopped"),
                PlayerEvent::Progress { position } => println!("Progress: {:#?} ms", position),
                PlayerEvent::Error { index, path, error } => {
                    error!("Error playing {}, {:?}: {}", index, path, error);
                }
                PlayerEvent::EndOfPlaylist => {
                    error!("Playlist is done");
                    break;
                }
            }
        }
    })
    .await
    .unwrap();

    Ok(())
}

pub async fn play_file_by_id(
    db: &DatabaseConnection,
    player: &Mutex<playback::player::Player>,
    file_id: i32,
    canonicalized_path: &Path,
) {
    match get_file_by_id(db, file_id).await {
        Ok(Some(file)) => {
            let player_guard = player.lock().unwrap();
            player_guard.pause();
            player_guard.clear_playlist();

            let file_path = canonicalize(canonicalized_path.join(file.directory).join(file.file_name)).unwrap();
            player_guard.add_to_playlist(file_path);
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
