use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use dunce::canonicalize;
use rinf::DartSignal;
use tokio::sync::Mutex;

use database::actions::file::get_files_by_ids;
use database::actions::mixes::query_mix_media_files;
use database::actions::stats::increase_skipped;
use database::connection::MainDbConnection;
use database::connection::RecommendationDbConnection;
use playback::player::Player;

use crate::OperatePlaybackWithMixQueryRequest;
use crate::OperatePlaybackWithMixQueryResponse;
use crate::VolumeRequest;
use crate::VolumeResponse;
use crate::{
    MovePlaylistItemRequest, NextRequest, PauseRequest, PlayRequest, PreviousRequest,
    RemoveRequest, SeekRequest, SetPlaybackModeRequest, SwitchRequest,
};

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

pub async fn play_request(player: Arc<Mutex<Player>>, _: DartSignal<PlayRequest>) -> Result<()> {
    player.lock().await.play();
    Ok(())
}

pub async fn pause_request(player: Arc<Mutex<Player>>, _: DartSignal<PauseRequest>) -> Result<()> {
    player.lock().await.pause();
    Ok(())
}

pub async fn next_request(
    main_db: Arc<MainDbConnection>,
    player: Arc<Mutex<Player>>,
    _: DartSignal<NextRequest>,
) -> Result<()> {
    let file_id = player.lock().await.get_status().id;

    if let Some(file_id) = file_id {
        increase_skipped(&main_db, file_id)
            .await
            .context("Unable to increase skipped count")?;
    }

    player.lock().await.next();

    Ok(())
}

pub async fn previous_request(
    main_db: Arc<MainDbConnection>,
    player: Arc<Mutex<Player>>,
    _: DartSignal<PreviousRequest>,
) -> Result<()> {
    let file_id = player.lock().await.get_status().id;

    if let Some(file_id) = file_id {
        increase_skipped(&main_db, file_id)
            .await
            .context("Unable to increase skipped count")?;
    }
    player.lock().await.previous();

    Ok(())
}

pub async fn set_playback_mode_request(
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<SetPlaybackModeRequest>,
) -> Result<()> {
    let mode = dart_signal.message.mode;
    player.lock().await.set_playback_mode(mode.into());

    Ok(())
}

pub async fn switch_request(
    main_db: Arc<MainDbConnection>,
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<SwitchRequest>,
) -> Result<()> {
    let file_id = player.lock().await.get_status().id;

    if let Some(file_id) = file_id {
        increase_skipped(&main_db, file_id)
            .await
            .context("Unable to increase skipped count")?;
    }

    player
        .lock()
        .await
        .switch(dart_signal.message.index.try_into().unwrap());

    Ok(())
}

pub async fn seek_request(
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<SeekRequest>,
) -> Result<()> {
    player
        .lock()
        .await
        .seek(dart_signal.message.position_seconds);

    Ok(())
}

pub async fn remove_request(
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<RemoveRequest>,
) -> Result<()> {
    player
        .lock()
        .await
        .remove_from_playlist(dart_signal.message.index as usize);

    Ok(())
}

pub async fn volume_request(
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<VolumeRequest>,
) -> Result<()> {
    let volume = dart_signal.message.volume;
    player.lock().await.set_volume(volume);

    VolumeResponse { volume }.send_signal_to_dart();

    Ok(())
}

pub async fn move_playlist_item_request(
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<MovePlaylistItemRequest>,
) -> Result<()> {
    let request = dart_signal.message;
    let old_index = request.old_index;
    let new_index = request.new_index;

    player
        .lock()
        .await
        .move_playlist_item(old_index.try_into()?, new_index.try_into()?);

    Ok(())
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
) -> Result<()> {
    let request = dart_signal.message;

    // Retrieve tracks
    let tracks = if request.queries.is_empty() {
        get_files_by_ids(&main_db, &request.fallback_media_file_ids).await?
    } else {
        query_mix_media_files(
            &main_db,
            &recommend_db,
            request
                .queries
                .iter()
                .map(|x| (x.operator.clone(), x.parameter.clone()))
                .collect(),
            0,
            4096,
        )
        .await
        .with_context(|| format!("Failed to query tracks: {:?}", request.queries))?
    };

    let mut player = player.lock().await;

    // Clear the playlist if requested
    if request.replace_playlist {
        player.clear_playlist();
    }

    let playlist_len = if request.replace_playlist {
        0
    } else {
        player.get_playlist().len()
    };

    let mut file_ids: Vec<i32> = tracks.iter().map(|x| x.id).collect();

    if request.next_play {
        player.add_to_next(files_to_playback_request(&lib_path, tracks));
        OperatePlaybackWithMixQueryResponse { file_ids }.send_signal_to_dart();
        return Ok(());
    }

    // If not required to play instantly, add to playlist and return
    if !request.instantly_play {
        player.add_to_playlist(files_to_playback_request(&lib_path, tracks), None);
        OperatePlaybackWithMixQueryResponse { file_ids }.send_signal_to_dart();
        return Ok(());
    }

    // Find the nearest index
    let nearest_index: Option<usize> = if request.hint_position < 0 {
        Some(0)
    } else {
        find_nearest_index(&file_ids, request.hint_position.try_into().unwrap(), |x| {
            *x == request.initial_playback_id
        })
    };

    // If no suitable index found, use fallback_media_file_ids
    if nearest_index.is_none() {
        file_ids = request.fallback_media_file_ids.clone();
    }

    let nearest_index = nearest_index.unwrap_or(request.hint_position.try_into().unwrap_or(0));

    // Add to playlist
    player.add_to_playlist(files_to_playback_request(&lib_path, tracks), None);
    OperatePlaybackWithMixQueryResponse {
        file_ids: file_ids.clone(),
    }
    .send_signal_to_dart();

    // Set playback mode
    if request.playback_mode != 99 {
        player.set_playback_mode(request.playback_mode.into());
    }

    // Switch to the nearest index and play
    player.switch(nearest_index + playlist_len);
    player.play();

    Ok(())
}
