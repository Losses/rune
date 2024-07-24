use dunce::canonicalize;
use log::{debug, error, info};
use sea_orm::DatabaseConnection;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::task;

use database::actions::file::get_file_by_id;
use database::actions::metadata::{get_metadata_summary_by_file_id, MetadataSummary};
use database::actions::recommendation::get_recommendation;
use database::connection::{MainDbConnection, RecommendationDbConnection};
use playback::player::Player;

use crate::common::Result;
use crate::connection;
use crate::messages;

pub async fn handle_playback(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
) -> Result<()> {
    info!("Initializing player.");
    let _player = Player::new();
    let player = Arc::new(Mutex::new(_player));

    info!("Initializing playback receiver.");
    let main_db_clone1 = main_db.clone();
    match play_file_request(&main_db_clone1, &player) {
        Ok(r) => r,
        Err(e) => error!("Error occured while binding play file request: {:#?}", e),
    };

    let main_db_clone2 = main_db.clone();
    match recommend_request(&main_db_clone2, &recommend_db, &player).await {
        Ok(r) => r,
        Err(e) => error!("Error occured while binding play file request: {:#?}", e),
    };

    let mut status_receiver = player.lock().unwrap().subscribe();

    info!("Initializing event listeners.");
    task::spawn(async move {
        let main_db = Arc::clone(&main_db);
        let mut cached_meta: Option<MetadataSummary> = None;
        let mut last_id: Option<i32> = None;

        while let Ok(status) = status_receiver.recv().await {
            debug!("Player status updated: {:?}", status);

            let meta = match status.id {
                Some(id) => {
                    if last_id != Some(id) {
                        println!("= CACHE NOW!");
                        // Update the cached metadata if the index has changed
                        match get_metadata_summary_by_file_id(&main_db, id).await {
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
    player: &Arc<Mutex<Player>>,
) -> Result<()> {
    use messages::recommend::*;
    let mut receiver = RecommendAndPlayRequest::get_dart_signal_receiver()?; // GENERATED

    tokio::spawn({
        let player = Arc::clone(player);
        let main_db = Arc::clone(main_db);
        let recommend_db = Arc::clone(recommend_db);
        async move {
            while let Some(dart_signal) = receiver.recv().await {
                let recommended_ids = recommend_and_play(
                    &main_db,
                    &recommend_db,
                    &player,
                    dart_signal.message.file_id,
                )
                .await;

                match recommended_ids {
                    Ok(recommended_ids) => {
                        PlaybackRecommendation { recommended_ids }.send_signal_to_dart() // GENERATED
                    },
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
        let file_path = canonicalize(Path::new(&file.directory).join(&file.file_name)).unwrap();
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
