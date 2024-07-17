use dunce::canonicalize;
use std::path::Path;
use std::sync::{Arc, Mutex};

use database::actions::file::get_random_files;
use database::connection::MainDbConnection;
use playback::player::Player;
use playback::PlayerEvent;

pub async fn play_random(main_db: &MainDbConnection, canonicalized_path: &Path) {
    let (player, mut events) = Player::new();
    let player = Arc::new(Mutex::new(player));

    let files = match get_random_files(main_db, 30).await {
        Ok(files) => files,
        Err(e) => {
            eprintln!("Failed to get random files by: {}", e);
            return;
        }
    };

    for file in files {
        let file_path =
            canonicalize(canonicalized_path.join(file.directory).join(file.file_name)).unwrap();

        player.lock().unwrap().add_to_playlist(file_path);
    }

    player.lock().unwrap().play();

    let player_clone = Arc::clone(&player);
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                PlayerEvent::Playing => println!("Playing"),
                PlayerEvent::Paused => println!("Paused"),
                PlayerEvent::Stopped => println!("Stopped"),
                PlayerEvent::EndOfTrack => {
                    println!("End of Track");
                    player_clone.lock().unwrap().next();
                }
                PlayerEvent::Progress { position } => println!("Progress: {:#?} ms", position),
                PlayerEvent::Error { index, path, error } => {
                    println!("Error playing {}, {:?}: {}", index, path, error);
                }
            }
        }
    });

    // std::thread::sleep(std::time::Duration::from_secs(10));
    // player.lock().unwrap().pause();
    // std::thread::sleep(std::time::Duration::from_secs(5));
    // player.lock().unwrap().play();
}
