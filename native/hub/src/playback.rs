use dunce::canonicalize;
use log::error;
use rinf::DartSignal;
use sea_orm::DatabaseConnection;
use std::path::Path;
use std::sync::{Arc, Mutex};

use database::actions::file::get_file_by_id;
use database::actions::file::get_files_by_ids;
use database::actions::recommendation::get_recommendation;
use database::connection::{MainDbConnection, RecommendationDbConnection};
use playback::player::Player;

use crate::common::Result;
use crate::messages::playback::{
    NextRequest, PauseRequest, PlayFileRequest, PlayRequest, PreviousRequest, RemoveRequest,
    SeekRequest, SwitchRequest,
};
use crate::messages::recommend::{PlaybackRecommendation, RecommendAndPlayRequest};
use crate::{connection, MovePlaylistItemRequest};

pub async fn play_file_request(
    main_db: Arc<DatabaseConnection>,
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<PlayFileRequest>,
) -> Result<()> {
    let play_file_request = dart_signal.message;
    let file_id = play_file_request.file_id;
    let lib_path = connection::get_media_library_path().await;

    play_file_by_id(main_db, player, file_id, Path::new(&lib_path.unwrap())).await;

    Ok(())
}

pub async fn play_file_by_id(
    db: Arc<DatabaseConnection>,
    player: Arc<Mutex<Player>>,
    file_id: i32,
    canonicalized_path: &Path,
) {
    match get_file_by_id(&db, file_id).await {
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

pub async fn recommend_and_play_request(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    lib_path: Arc<String>,
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<RecommendAndPlayRequest>,
) -> Result<()> {
    let file_id = dart_signal.message.file_id;

    // Get recommendations
    let recommendations = match get_recommendation(&recommend_db, file_id, 30) {
        Ok(recs) => recs,
        Err(e) => {
            error!("Error getting recommendations: {:#?}", e);
            let result: Vec<(u32, f32)> = Vec::new();
            result
        }
    };

    let files = get_files_by_ids(
        &main_db,
        &recommendations
            .into_iter()
            .map(|x| x.0 as i32)
            .collect::<Vec<i32>>(),
    )
    .await;

    let requests = match files {
        Ok(files) => files
            .into_iter()
            .map(|file| {
                let file_path = canonicalize(
                    Path::new(&**lib_path)
                        .join(&file.directory)
                        .join(&file.file_name),
                )
                .unwrap();

                (file.id, file_path)
            })
            .collect::<Vec<_>>(),
        Err(e) => {
            error!("Unable to get files: {}", e);
            Vec::new()
        }
    };

    // Clear the playlist and add new recommendations
    player.lock().unwrap().pause();
    player.lock().unwrap().clear_playlist();

    for request in &requests {
        player
            .lock()
            .unwrap()
            .add_to_playlist(request.0, request.1.clone());
    }
    player.lock().unwrap().play();

    // Send the recommendation IDs back to Dart
    let recommended_ids: Vec<i32> = requests.into_iter().map(|(id, _)| id).collect();

    PlaybackRecommendation { recommended_ids }.send_signal_to_dart();

    Ok(())
}

pub async fn play_request(player: Arc<Mutex<Player>>, _: DartSignal<PlayRequest>) {
    player.lock().unwrap().play()
}
pub async fn pause_request(player: Arc<Mutex<Player>>, _: DartSignal<PauseRequest>) {
    player.lock().unwrap().pause()
}
pub async fn next_request(player: Arc<Mutex<Player>>, _: DartSignal<NextRequest>) {
    player.lock().unwrap().next()
}
pub async fn previous_request(player: Arc<Mutex<Player>>, _: DartSignal<PreviousRequest>) {
    player.lock().unwrap().previous()
}
pub async fn switch_request(player: Arc<Mutex<Player>>, dart_signal: DartSignal<SwitchRequest>) {
    player
        .lock()
        .unwrap()
        .switch(dart_signal.message.index.try_into().unwrap())
}
pub async fn seek_request(player: Arc<Mutex<Player>>, dart_signal: DartSignal<SeekRequest>) {
    player
        .lock()
        .unwrap()
        .seek(dart_signal.message.position_seconds)
}
pub async fn remove_request(player: Arc<Mutex<Player>>, dart_signal: DartSignal<RemoveRequest>) {
    player
        .lock()
        .unwrap()
        .remove_from_playlist(dart_signal.message.index as usize)
}

pub async fn move_playlist_item_request(
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<MovePlaylistItemRequest>,
) {
    let request = dart_signal.message;
    let old_index = request.old_index;
    let new_index = request.new_index;

    player
        .lock()
        .unwrap()
        .move_playlist_item(old_index.try_into().unwrap(), new_index.try_into().unwrap());
}
