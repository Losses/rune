use dunce::canonicalize;
use log::{debug, error, info};
use sea_orm::DatabaseConnection;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::task;

use database::actions::file::get_file_by_id;
use database::actions::metadata::{
    get_metadata_summary_by_file_id, get_metadata_summary_by_file_ids, MetadataSummary,
};
use database::actions::recommendation::get_recommendation;
use database::connection::{MainDbConnection, RecommendationDbConnection};
use playback::player::{Player, PlaylistStatus};

use crate::common::Result;
use crate::connection;
use crate::messages;

pub async fn handle_playback(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    lib_path: Arc<String>,
) -> Result<()> {
    info!("Initializing player.");
    let player = Player::new();
    let player = Arc::new(Mutex::new(player));

    info!("Initializing playback receiver.");
    match play_file_request(&main_db, &player) {
        Ok(r) => r,
        Err(e) => error!("Error occured while binding play file request: {:#?}", e),
    };

    match recommend_request(&main_db, &recommend_db, &lib_path, &player).await {
        Ok(r) => r,
        Err(e) => error!("Error occured while binding recommend request: {:#?}", e),
    };

    match playback_control_request(&player).await {
        Ok(r) => r,
        Err(e) => error!(
            "Error occured while binding playback contorl request: {:#?}",
            e
        ),
    };

    match move_playlist_item_request(&player) {
        Ok(r) => r,
        Err(e) => error!(
            "Error occured while binding move playlist item request: {:#?}",
            e
        ),
    };

    let mut status_receiver = player.lock().unwrap().subscribe_status();
    let mut playlist_receiver = player.lock().unwrap().subscribe_playlist();
    let mut realtime_fft_receiver = player.lock().unwrap().subscribe_realtime_fft();

    // Clone main_db for each task
    let main_db_for_status = Arc::clone(&main_db);
    let main_db_for_playlist = Arc::clone(&main_db);

    info!("Initializing event listeners.");
    task::spawn(async move {
        let main_db = Arc::clone(&main_db_for_status);
        let mut cached_meta: Option<MetadataSummary> = None;
        let mut last_id: Option<i32> = None;

        while let Ok(status) = status_receiver.recv().await {
            debug!("Player status updated: {:?}", status);

            let meta = match status.id {
                Some(id) => {
                    if last_id != Some(id) {
                        // Update the cached metadata if the index has changed
                        match get_metadata_summary_by_file_id(&main_db, id).await {
                            Ok(metadata) => {
                                cached_meta = Some(metadata);
                                last_id = Some(id);
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
                id: status.id.unwrap_or(0).try_into().unwrap(),
                index: status.index.unwrap_or(0).try_into().unwrap(),
            }
            .send_signal_to_dart();
        }
    });

    task::spawn(async move {
        let main_db = Arc::clone(&main_db_for_playlist);

        while let Ok(playlist) = playlist_receiver.recv().await {
            send_playlist_update(&main_db, &playlist).await;
        }
    });

    task::spawn(async move {
        while let Ok(value) = realtime_fft_receiver.recv().await {
            send_realtime_fft(value).await;
        }
    });

    Ok(())
}

fn play_file_request(main_db: &Arc<DatabaseConnection>, player: &Arc<Mutex<Player>>) -> Result<()> {
    use messages::playback::*;
    let mut ui_receiver = PlayFileRequest::get_dart_signal_receiver()?; // GENERATED

    tokio::spawn({
        let player = Arc::clone(player);
        let main_db = Arc::clone(main_db);
        async move {
            while let Some(dart_signal) = ui_receiver.recv().await {
                let play_file_request = dart_signal.message;
                let file_id = play_file_request.file_id;
                let lib_path = connection::get_media_library_path().await;

                play_file_by_id(&main_db, &player, file_id, Path::new(&lib_path.unwrap())).await;
            }
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

pub async fn recommend_request(
    main_db: &Arc<MainDbConnection>,
    recommend_db: &Arc<RecommendationDbConnection>,
    lib_path: &Arc<String>,
    player: &Arc<Mutex<Player>>,
) -> Result<()> {
    use messages::recommend::*;
    let mut receiver = RecommendAndPlayRequest::get_dart_signal_receiver()?; // GENERATED

    tokio::spawn({
        let player = Arc::clone(player);
        let main_db = Arc::clone(main_db);
        let recommend_db = Arc::clone(recommend_db);
        let lib_path = Arc::clone(lib_path);
        async move {
            while let Some(dart_signal) = receiver.recv().await {
                let recommended_ids = recommend_and_play(
                    &main_db,
                    &recommend_db,
                    &lib_path,
                    &player,
                    dart_signal.message.file_id,
                )
                .await;

                match recommended_ids {
                    Ok(recommended_ids) => {
                        PlaybackRecommendation { recommended_ids }.send_signal_to_dart()
                        // GENERATED
                    }
                    Err(e) => {
                        error!("Recommendation error: {:#?}", e);
                    }
                }
            }
        }
    });

    Ok(())
}

async fn recommend_and_play(
    main_db: &MainDbConnection,
    recommend_db: &RecommendationDbConnection,
    lib_path: &Arc<String>,
    player: &Mutex<Player>,
    file_id: i32,
) -> Result<Vec<i32>> {
    // Get recommendations
    let recommendations = match get_recommendation(recommend_db, file_id, 30) {
        Ok(recs) => recs,
        Err(e) => {
            error!("Error getting recommendations: {:#?}", e);
            let result: Vec<i32> = Vec::new();
            return Ok(result);
        }
    };

    // Clear the playlist and add new recommendations
    player.lock().unwrap().pause();
    player.lock().unwrap().clear_playlist();

    for (_rec_id, _) in &recommendations {
        let rec_id = (*_rec_id).try_into().unwrap();
        let file = match get_file_by_id(main_db, rec_id).await {
            Ok(Some(file)) => file,
            Ok(None) => continue,
            Err(e) => {
                error!("Error getting file by id {}: {}", rec_id, e);
                continue;
            }
        };
        let file_path = canonicalize(
            Path::new(&**lib_path)
                .join(&file.directory)
                .join(&file.file_name),
        )
        .unwrap();
        player.lock().unwrap().add_to_playlist(rec_id, file_path);
    }
    player.lock().unwrap().play();

    // Send the recommendation IDs back to Dart
    let recommended_ids: Vec<i32> = recommendations
        .into_iter()
        .map(|(id, _)| id as i32)
        .collect();

    Ok(recommended_ids)
}

pub async fn playback_control_request(player: &Arc<Mutex<Player>>) -> Result<()> {
    use messages::playback::*;

    let mut play_receiver = PlayRequest::get_dart_signal_receiver()?; // GENERATED
    let mut pause_receiver = PauseRequest::get_dart_signal_receiver()?; // GENERATED
    let mut next_receiver = NextRequest::get_dart_signal_receiver()?; // GENERATED
    let mut previous_receiver = PreviousRequest::get_dart_signal_receiver()?; // GENERATED
    let mut switch_receiver = SwitchRequest::get_dart_signal_receiver()?; // GENERATED
    let mut seek_receiver = SeekRequest::get_dart_signal_receiver()?; // GENERATED
    let mut remove_receiver = RemoveRequest::get_dart_signal_receiver()?; // GENERATED

    // Handle Play Request
    tokio::spawn({
        let player = Arc::clone(player);
        async move {
            while (play_receiver.recv().await).is_some() {
                let player_guard = player.lock().unwrap();
                player_guard.play();
            }
        }
    });

    // Handle Pause Request
    tokio::spawn({
        let player = Arc::clone(player);
        async move {
            while (pause_receiver.recv().await).is_some() {
                let player_guard = player.lock().unwrap();
                player_guard.pause();
            }
        }
    });

    // Handle Next Request
    tokio::spawn({
        let player = Arc::clone(player);
        async move {
            while (next_receiver.recv().await).is_some() {
                let player_guard = player.lock().unwrap();
                player_guard.next();
            }
        }
    });

    // Handle Previous Request
    tokio::spawn({
        let player = Arc::clone(player);
        async move {
            while (previous_receiver.recv().await).is_some() {
                let player_guard = player.lock().unwrap();
                player_guard.previous();
            }
        }
    });

    // Handle Seek Request
    tokio::spawn({
        let player = Arc::clone(player);
        async move {
            while let Some(dart_signal) = switch_receiver.recv().await {
                let switch_request = dart_signal.message;
                let player_guard = player.lock().unwrap();
                player_guard.switch(switch_request.index.try_into().unwrap());
            }
        }
    });

    // Handle Seek Request
    tokio::spawn({
        let player = Arc::clone(player);
        async move {
            while let Some(dart_signal) = seek_receiver.recv().await {
                let seek_request = dart_signal.message;
                let player_guard = player.lock().unwrap();
                player_guard.seek(seek_request.position_seconds);
            }
        }
    });

    // Handle Remove Request
    tokio::spawn({
        let player = Arc::clone(player);
        async move {
            while let Some(dart_signal) = remove_receiver.recv().await {
                let remove_request = dart_signal.message;
                let player_guard = player.lock().unwrap();
                player_guard.remove_from_playlist(remove_request.index as usize);
            }
        }
    });

    Ok(())
}

pub async fn send_playlist_update(db: &DatabaseConnection, playlist: &PlaylistStatus) {
    use messages::playback::*;

    let file_ids: Vec<i32> = playlist.items.clone();

    match get_metadata_summary_by_file_ids(db, file_ids).await {
        Ok(summaries) => {
            let items = summaries
                .into_iter()
                .map(|item| PlaylistItem {
                    id: item.id,
                    artist: item.artist,
                    album: item.album,
                    title: item.title,
                    duration: item.duration,
                })
                .collect();
            PlaylistUpdate { items }.send_signal_to_dart(); // GENERATED
        }
        Err(e) => {
            error!("Error happened while updating playlist: {:?}", e)
        }
    }
}

pub async fn send_realtime_fft(value: Vec<f32>) {
    use messages::playback::*;

    RealtimeFft { value }.send_signal_to_dart(); // GENERATED
}

fn move_playlist_item_request(player: &Arc<Mutex<Player>>) -> Result<()> {
    use messages::playback::*;
    let mut ui_receiver = MovePlaylistItemRequest::get_dart_signal_receiver()?; // GENERATED

    tokio::spawn({
        let player = Arc::clone(player);
        async move {
            while let Some(dart_signal) = ui_receiver.recv().await {
                let request = dart_signal.message;
                let old_index = request.old_index;
                let new_index = request.new_index;

                player.lock().unwrap().move_playlist_item(
                    old_index.try_into().unwrap(),
                    new_index.try_into().unwrap(),
                );
            }
        }
    });

    Ok(())
}
