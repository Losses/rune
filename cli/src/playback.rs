use std::{
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use dunce::canonicalize;
use futures::future::join_all;
use log::{debug, error, info};
use tokio::task;

use database::{
    actions::file::{get_file_by_id, get_random_files},
    connection::MainDbConnection,
};
use playback::{
    player::{Playable, Player, PlayingItem},
    strategies::AddMode,
};

async fn play_files(main_db: &MainDbConnection, canonicalized_path: &Path, file_ids: Vec<i32>) {
    let player = Player::new(None);
    let player = Arc::new(Mutex::new(player));

    let file_futures = file_ids.into_iter().map(|id| async move {
        match get_file_by_id(main_db, id).await {
            Ok(file) => Some(file),
            Err(e) => {
                error!("Failed to get file by id {id}: {e}");
                None
            }
        }
    });

    let files: Vec<database::entities::media_files::Model> = join_all(file_futures)
        .await
        .into_iter()
        .filter_map(|file| file.flatten())
        .collect();

    player.lock().unwrap().add_to_playlist(
        files
            .into_iter()
            .map(|file| {
                (
                    PlayingItem::InLibrary(file.id),
                    canonicalize(canonicalized_path.join(file.directory).join(file.file_name))
                        .unwrap(),
                )
            })
            .collect(),
        AddMode::AppendToEnd,
    );

    player.lock().unwrap().play();

    let status_receiver = player.lock().unwrap().subscribe_status();

    info!("Initializing event listeners");
    task::spawn(async move {
        while let Ok(status) = status_receiver.recv().await {
            debug!("Player status updated: {status:?}");

            let position = status.position;

            debug!(
                "State: {}, seconds: {}",
                status.state,
                position.as_secs_f32()
            );
        }
    });

    thread::sleep(Duration::from_millis(30000));
}

pub async fn play_random(main_db: &MainDbConnection, canonicalized_path: &Path) {
    match get_random_files(main_db, 30).await {
        Ok(files) => {
            let file_ids = files.into_iter().map(|file| file.id).collect();
            play_files(main_db, canonicalized_path, file_ids).await;
        }
        Err(e) => {
            error!("Failed to get random files: {e}");
        }
    }
}

pub async fn play_by_id(main_db: &MainDbConnection, canonicalized_path: &Path, id: i32) {
    play_files(main_db, canonicalized_path, vec![id]).await;
}
