use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use database::actions::mixes::query_mix_media_files;
use dunce::canonicalize;
use log::{debug, error, info};
use rinf::DartSignal;
use sea_orm::DatabaseConnection;
use tokio::sync::Mutex;

use database::actions::file::get_file_by_id;
use database::actions::stats::increase_skipped;
use database::connection::MainDbConnection;
use database::connection::RecommendationDbConnection;
use playback::player::Player;

use crate::OperatePlaybackWithMixQueryRequest;
use crate::{
    MovePlaylistItemRequest, NextRequest, PauseRequest, PlayFileRequest, PlayRequest,
    PreviousRequest, RemoveRequest, SeekRequest, SetPlaybackModeRequest, SwitchRequest,
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
            player_guard.add_to_playlist([(file_id, file_path)].to_vec());
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

pub fn files_to_playback_request(
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

pub async fn play_request(player: Arc<Mutex<Player>>, _: DartSignal<PlayRequest>) {
    player.lock().await.play()
}

pub async fn pause_request(player: Arc<Mutex<Player>>, _: DartSignal<PauseRequest>) {
    player.lock().await.pause()
}

pub async fn next_request(
    main_db: Arc<MainDbConnection>,
    player: Arc<Mutex<Player>>,
    _: DartSignal<NextRequest>,
) {
    let file_id = player.lock().await.get_status().id;

    if let Some(file_id) = file_id {
        match increase_skipped(&main_db, file_id)
            .await
            .context("Unable to increase skipped count")
        {
            Ok(_) => {}
            Err(e) => error!("{:?}", e),
        }
    }
    player.lock().await.next()
}

pub async fn previous_request(
    main_db: Arc<MainDbConnection>,
    player: Arc<Mutex<Player>>,
    _: DartSignal<PreviousRequest>,
) {
    let file_id = player.lock().await.get_status().id;

    if let Some(file_id) = file_id {
        match increase_skipped(&main_db, file_id)
            .await
            .context("Unable to increase skipped count")
        {
            Ok(_) => {}
            Err(e) => error!("{:?}", e),
        }
    }
    player.lock().await.previous()
}

pub async fn set_playback_mode_request(
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<SetPlaybackModeRequest>,
) {
    let mode = dart_signal.message.mode;
    debug!("Setting playback mode to: {}", mode);
    player.lock().await.set_playback_mode(mode.into())
}

pub async fn switch_request(
    main_db: Arc<MainDbConnection>,
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<SwitchRequest>,
) {
    let file_id = player.lock().await.get_status().id;

    if let Some(file_id) = file_id {
        match increase_skipped(&main_db, file_id)
            .await
            .context("Unable to increase skipped count")
        {
            Ok(_) => {}
            Err(e) => error!("{:?}", e),
        }
    }
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

fn find_nearest_index<T, F>(vec: &[T], hint_position: usize, predicate: F) -> Option<usize>
where
    F: Fn(&T) -> bool,
{
    if vec.is_empty() {
        return None;
    }

    let len = vec.len();
    let mut left = hint_position;
    let mut right = hint_position;

    loop {
        if left < len && predicate(&vec[left]) {
            return Some(left);
        }

        if right < len && predicate(&vec[right]) {
            return Some(right);
        }

        if left == 0 && right >= len - 1 {
            break;
        }

        left = left.saturating_sub(1);

        if right < len - 1 {
            right += 1;
        }
    }

    None
}

pub async fn operate_playback_with_mix_query_request(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    lib_path: Arc<String>,
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<OperatePlaybackWithMixQueryRequest>,
) {
    let request = dart_signal.message;

    info!("Handling mix operators: {:?}", request.queries);

    let tracks = query_mix_media_files(
        &main_db,
        &recommend_db,
        request
            .queries
            .into_iter()
            .map(|x| (x.operator, x.parameter))
            .collect(),
        0,
        20480,
    )
    .await
    .with_context(|| "Failed to query tracks");

    match tracks {
        Ok(files) => {
            if request.replace_playlist {
                player.lock().await.clear_playlist();
            }

            let playlist_len = if request.replace_playlist {
                0
            } else {
                player.lock().await.get_playlist().len()
            };

            player
                .lock()
                .await
                .add_to_playlist(files_to_playback_request(&lib_path, files.clone()));

            let file_ids: Vec<i32> = files.into_iter().map(|x| x.id).collect();

            if request.instantly_play {
                let nearest_index: Option<usize> = if request.hint_position < 0 {
                    Some(0)
                } else {
                    find_nearest_index(&file_ids, request.hint_position.try_into().unwrap(), |x| {
                        *x == request.initial_playback_id
                    })
                };

                if let Some(nearest_id) = nearest_index {
                    if request.playback_mode != 99 {
                        player
                            .lock()
                            .await
                            .set_playback_mode(request.playback_mode.into());
                    }

                    player.lock().await.switch(nearest_id + playlist_len);
                    player.lock().await.play();
                } else {
                    error!("Failed to find the neareat playback item based on the hint");
                }
            }
        }
        Err(e) => error!("{:?}", e),
    }
}
