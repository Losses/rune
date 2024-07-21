use dunce::canonicalize;
use std::path::Path;
use std::sync::{Arc, Mutex};

use playback::player::Player;
use playback::PlayerEvent;

use crate::common::Result;
use crate::messages;

pub async fn handle_playback() -> Result<()> {
    use messages::playback::*;

    let (player, mut events) = Player::new();
    let player = Arc::new(Mutex::new(player));

    let mut receiver = PlayFileRequest::get_dart_signal_receiver()?; // GENERATED

    while let Some(dart_signal) = receiver.recv().await {
        let play_file_request = dart_signal.message;
        let file_id = play_file_request.file_id;

        play_file_by_id(&player, &file_id, Path::new("/path/to/files"));
    }

    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                PlayerEvent::Playing => println!("Playing"),
                PlayerEvent::Paused => println!("Paused"),
                PlayerEvent::Stopped => println!("Stopped"),
                PlayerEvent::Progress { position } => println!("Progress: {:#?} ms", position),
                PlayerEvent::Error { index, path, error } => {
                    println!("Error playing {}, {:?}: {}", index, path, error);
                }
                PlayerEvent::EndOfPlaylist => {
                    println!("Playlist is done");
                    break;
                }
            }
        }
    })
    .await
    .unwrap();

    Ok(())
}

pub fn play_file_by_id(
    player: &Arc<Mutex<playback::player::Player>>,
    file_id: &str,
    canonicalized_path: &Path,
) {
    player.lock().unwrap().pause();
    player.lock().unwrap().clear_playlist();

    let file_path = canonicalize(canonicalized_path.join(file_id)).unwrap();
    player.lock().unwrap().add_to_playlist(file_path);
    player.lock().unwrap().play();
}
