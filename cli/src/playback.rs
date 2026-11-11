use database::{
    actions::file::{get_file_by_id, get_random_files},
    connection::MainDbConnection,
    entities::media_files,
};
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
use uuid::Uuid;

use playback::{
    player::{Playable, Player, PlayingItem},
    strategies::AddMode,
};

async fn play_files(
    main_db: &MainDbConnection,
    canonicalized_path: &Path,
    file_ids: Vec<i32>,
    library_id: &str,
) {
    let fsio = Arc::new(fsio::FsIo::new());
    let player = Player::new(fsio, None);
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

    let files: Vec<media_files::Model> = join_all(file_futures)
        .await
        .into_iter()
        .filter_map(|file| file.flatten())
        .collect();

    let library_uuid = match Uuid::parse_str(library_id) {
        Ok(library_uuid) => library_uuid,
        Err(error) => {
            error!("Invalid library id {library_id}: {error}");
            return;
        }
    };

    player.lock().unwrap().add_to_playlist(
        files
            .into_iter()
            .filter_map(|file| {
                if let Err(error) = Uuid::parse_str(&file.hlc_uuid) {
                    error!("Invalid file UUID {}: {error}", file.hlc_uuid);
                    return None;
                }
                Some((
                    PlayingItem::LibraryTrack {
                        library_id: library_uuid,
                        track: file.hlc_uuid,
                    },
                    canonicalize(canonicalized_path.join(file.directory).join(file.file_name))
                        .unwrap(),
                ))
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

pub async fn play_random(main_db: &MainDbConnection, canonicalized_path: &Path, library_id: &str) {
    match get_random_files(main_db, 30).await {
        Ok(files) => {
            let file_ids = files.into_iter().map(|file| file.id).collect();
            play_files(main_db, canonicalized_path, file_ids, library_id).await;
        }
        Err(e) => {
            error!("Failed to get random files: {e}");
        }
    }
}

pub async fn play_by_id(
    main_db: &MainDbConnection,
    canonicalized_path: &Path,
    id: i32,
    library_id: &str,
) {
    play_files(main_db, canonicalized_path, vec![id], library_id).await;
}
