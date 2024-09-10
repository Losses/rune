use dunce::canonicalize;
use log::error;
use rinf::DartSignal;
use sea_orm::DatabaseConnection;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

use database::actions::albums::get_media_file_ids_of_album;
use database::actions::analysis::get_centralized_analysis_result;
use database::actions::artists::get_media_file_ids_of_artist;
use database::actions::file::get_file_by_id;
use database::actions::file::get_files_by_ids;
use database::actions::playlists::get_media_file_ids_of_playlist;
use database::actions::recommendation::{
    get_recommendation_by_file_id, get_recommendation_by_parameter,
};
use database::connection::{MainDbConnection, RecommendationDbConnection};
use playback::player::Player;

use crate::common::Result;
use crate::messages::playback::{
    NextRequest, PauseRequest, PlayFileRequest, PlayRequest, PreviousRequest, RemoveRequest,
    SeekRequest, SwitchRequest,
};
use crate::messages::recommend::{PlaybackRecommendation, RecommendAndPlayRequest};
use crate::{
    AddToQueueCollectionRequest, MovePlaylistItemRequest, StartPlayingCollectionRequest,
    StartRoamingCollectionRequest,
};

async fn play_file_by_id(
    db: Arc<DatabaseConnection>,
    player: Arc<Mutex<Player>>,
    lib_path: Arc<String>,
    file_id: i32,
) {
    match get_file_by_id(&db, file_id).await {
        Ok(Some(file)) => {
            let player_guard = player.lock().await;
            player_guard.pause();
            player_guard.clear_playlist();

            let file_path = canonicalize(
                Path::new(&*lib_path)
                    .join(file.directory)
                    .join(file.file_name),
            )
            .unwrap();
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

fn files_to_playback_request(
    lib_path: &String,
    files: Vec<database::entities::media_files::Model>,
) -> std::vec::Vec<(i32, std::path::PathBuf)> {
    files
        .into_iter()
        .map(|file| {
            let file_path = canonicalize(
                Path::new(lib_path)
                    .join(&file.directory)
                    .join(&file.file_name),
            )
            .unwrap();

            (file.id, file_path)
        })
        .collect::<Vec<_>>()
}

pub async fn update_playlist(
    player: &Arc<Mutex<Player>>,
    requests: Vec<(i32, std::path::PathBuf)>,
) {
    let player_guard = player.lock().await;
    for request in requests {
        player_guard.add_to_playlist(request.0, request.1);
    }
    player_guard.play();
}

macro_rules! handle_add_collection_to_playlist_request {
    ($main_db:expr, $lib_path:expr, $player:expr, $dart_signal:expr, $get_media_file_ids_fn:expr) => {{
        let request = $dart_signal.message;
        let media_file_ids = $get_media_file_ids_fn(&$main_db, request.id)
            .await
            .unwrap_or_default();

        let files = get_files_by_ids(&$main_db, &media_file_ids).await?;
        let requests = files_to_playback_request(&$lib_path, files);

        update_playlist(&$player, requests).await;
    }};
}

pub async fn play_file_request(
    main_db: Arc<DatabaseConnection>,
    lib_path: Arc<String>,
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<PlayFileRequest>,
) -> Result<()> {
    let play_file_request = dart_signal.message;
    let file_id = play_file_request.file_id;

    play_file_by_id(main_db, player, lib_path, file_id).await;

    Ok(())
}

pub async fn recommend_and_play_request(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    lib_path: Arc<String>,
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<RecommendAndPlayRequest>,
) -> Result<()> {
    let file_id = dart_signal.message.file_id;

    let recommendations = match get_recommendation_by_file_id(&recommend_db, file_id, 30) {
        Ok(recs) => recs,
        Err(e) => {
            error!("Error getting recommendations: {:#?}", e);
            Vec::new()
        }
    };

    let recommendation_ids: Vec<i32> = recommendations.iter().map(|x| x.0 as i32).collect();

    let files = get_files_by_ids(&main_db, &recommendation_ids).await?;

    // Create a HashMap to store file_id -> file mapping
    let file_map: HashMap<i32, database::entities::media_files::Model> =
        files.into_iter().map(|file| (file.id, file)).collect();

    // Reorder files based on the order of recommendation_ids
    let ordered_files: Vec<database::entities::media_files::Model> = recommendation_ids
        .into_iter()
        .filter_map(|id| file_map.get(&id).cloned())
        .collect();

    let requests = files_to_playback_request(&lib_path, ordered_files);
    update_playlist(&player, requests.clone()).await;

    let recommended_ids: Vec<i32> = requests.into_iter().map(|(id, _)| id).collect();
    PlaybackRecommendation { recommended_ids }.send_signal_to_dart();

    Ok(())
}

pub async fn start_playing_collection_request(
    main_db: Arc<MainDbConnection>,
    lib_path: Arc<String>,
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<StartPlayingCollectionRequest>,
) -> Result<()> {
    player.lock().await.pause();
    player.lock().await.clear_playlist();

    match dart_signal.message.r#type.as_str() {
        "artist" => handle_add_collection_to_playlist_request!(
            main_db,
            lib_path,
            player,
            dart_signal,
            get_media_file_ids_of_artist
        ),
        "album" => handle_add_collection_to_playlist_request!(
            main_db,
            lib_path,
            player,
            dart_signal,
            get_media_file_ids_of_album
        ),
        "playlist" => handle_add_collection_to_playlist_request!(
            main_db,
            lib_path,
            player,
            dart_signal,
            get_media_file_ids_of_playlist
        ),
        _ => {}
    }

    Ok(())
}

pub async fn add_to_queue_collection_request(
    main_db: Arc<MainDbConnection>,
    lib_path: Arc<String>,
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<AddToQueueCollectionRequest>,
) -> Result<()> {
    match dart_signal.message.r#type.as_str() {
        "artist" => handle_add_collection_to_playlist_request!(
            main_db,
            lib_path,
            player,
            dart_signal,
            get_media_file_ids_of_artist
        ),
        "album" => handle_add_collection_to_playlist_request!(
            main_db,
            lib_path,
            player,
            dart_signal,
            get_media_file_ids_of_album
        ),
        "playlist" => handle_add_collection_to_playlist_request!(
            main_db,
            lib_path,
            player,
            dart_signal,
            get_media_file_ids_of_playlist
        ),
        _ => {}
    }

    Ok(())
}

pub async fn start_roaming_collection_request(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    lib_path: Arc<String>,
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<StartRoamingCollectionRequest>,
) -> Result<()> {
    let request = dart_signal.message;
    let media_file_ids = match request.r#type.as_str() {
        "artist" => get_media_file_ids_of_artist(&main_db, request.id).await,
        "album" => get_media_file_ids_of_album(&main_db, request.id).await,
        "playlist" => get_media_file_ids_of_playlist(&main_db, request.id).await,
        _ => Ok(vec![]),
    };

    let aggregated = get_centralized_analysis_result(&main_db, media_file_ids.unwrap()).await;
    let recommendations = get_recommendation_by_parameter(&recommend_db, aggregated.into(), 30).unwrap();

    let files = get_files_by_ids(
        &main_db,
        &recommendations
            .into_iter()
            .map(|x| x.0 as i32)
            .collect::<Vec<i32>>(),
    )
    .await?;

    let requests = files_to_playback_request(&lib_path, files);
    update_playlist(&player, requests).await;

    Ok(())
}

pub async fn play_request(player: Arc<Mutex<Player>>, _: DartSignal<PlayRequest>) {
    player.lock().await.play()
}

pub async fn pause_request(player: Arc<Mutex<Player>>, _: DartSignal<PauseRequest>) {
    player.lock().await.pause()
}

pub async fn next_request(player: Arc<Mutex<Player>>, _: DartSignal<NextRequest>) {
    player.lock().await.next()
}

pub async fn previous_request(player: Arc<Mutex<Player>>, _: DartSignal<PreviousRequest>) {
    player.lock().await.previous()
}

pub async fn switch_request(player: Arc<Mutex<Player>>, dart_signal: DartSignal<SwitchRequest>) {
    player
        .lock()
        .await
        .switch(dart_signal.message.index.try_into().unwrap())
}

pub async fn seek_request(player: Arc<Mutex<Player>>, dart_signal: DartSignal<SeekRequest>) {
    player
        .lock()
        .await
        .seek(dart_signal.message.position_seconds)
}

pub async fn remove_request(player: Arc<Mutex<Player>>, dart_signal: DartSignal<RemoveRequest>) {
    player
        .lock()
        .await
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
        .await
        .move_playlist_item(old_index.try_into().unwrap(), new_index.try_into().unwrap());
}
