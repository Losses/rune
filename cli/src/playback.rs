use dunce::canonicalize;
use log::{debug, error, info};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::task;

use database::actions::file::get_random_files;
use database::connection::MainDbConnection;
use playback::player::Player;

pub async fn play_random(main_db: &MainDbConnection, canonicalized_path: &Path) {
    let player = Player::new(None);
    let player = Arc::new(Mutex::new(player));

    let files = match get_random_files(main_db, 30).await {
        Ok(files) => files,
        Err(e) => {
            error!("Failed to get random files by: {}", e);
            return;
        }
    };

    player.lock().unwrap().add_to_playlist(
        files
            .into_iter()
            .map(|file| {
                (
                    file.id,
                    canonicalize(canonicalized_path.join(file.directory).join(file.file_name))
                        .unwrap(),
                )
            })
            .collect(),
        playback::strategies::AddMode::AppendToEnd,
    );

    player.lock().unwrap().play();

    let mut status_receiver = player.lock().unwrap().subscribe_status();

    info!("Initializing event listeners");
    task::spawn(async move {
        while let Ok(status) = status_receiver.recv().await {
            debug!("Player status updated: {:?}", status);

            let position = status.position;

            debug!(
                "State: {}, seconds: {}",
                status.state.to_string(),
                position.as_secs_f32()
            );
        }
    });
}
