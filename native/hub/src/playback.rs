use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use dunce::canonicalize;
use rinf::DartSignal;
use tokio::sync::Mutex;

use database::actions::mixes::query_mix_media_files;
use database::actions::stats::increase_skipped;
use database::connection::MainDbConnection;
use database::connection::RecommendationDbConnection;
use database::playing_item::dispatcher::PlayingItemActionDispatcher;
use database::playing_item::MediaFileHandle;
use playback::player::Player;
use playback::player::PlayingItem;
use playback::strategies::AddMode;

use crate::{
    InLibraryPlayingItem, IndependentFilePlayingItem, LoadRequest, MovePlaylistItemRequest,
    NextRequest, OperatePlaybackWithMixQueryRequest, OperatePlaybackWithMixQueryResponse,
    PauseRequest, PlayRequest, PlayingItemRequest, PlaylistOperateMode, PreviousRequest,
    RemoveRequest, SeekRequest, SetAdaptiveSwitchingEnabledRequest, SetPlaybackModeRequest,
    SetRealtimeFftEnabledRequest, SwitchRequest, VolumeRequest, VolumeResponse,
};

impl From<PlayingItem> for PlayingItemRequest {
    fn from(x: PlayingItem) -> Self {
        match x {
            PlayingItem::InLibrary(x) => PlayingItemRequest {
                in_library: Some(InLibraryPlayingItem { file_id: x }),
                independent_file: None,
            },
            PlayingItem::IndependentFile(path_buf) => PlayingItemRequest {
                in_library: None,
                independent_file: Some(IndependentFilePlayingItem {
                    path: path_buf.to_string_lossy().to_string(),
                }),
            },
            PlayingItem::Unknown => PlayingItemRequest {
                in_library: None,
                independent_file: None,
            },
        }
    }
}

impl From<PlayingItemRequest> for PlayingItem {
    fn from(x: PlayingItemRequest) -> Self {
        if let Some(in_library) = x.in_library {
            if in_library.file_id != 0 {
                return PlayingItem::InLibrary(in_library.file_id);
            }
        }

        if let Some(independent_file) = x.independent_file {
            if !independent_file.path.is_empty() {
                return PlayingItem::IndependentFile(PathBuf::from(independent_file.path));
            }
        }

        PlayingItem::Unknown
    }
}

pub fn files_to_playback_request(
    lib_path: &String,
    files: &[MediaFileHandle],
) -> Vec<(PlayingItem, PathBuf)> {
    files
        .iter()
        .filter_map(|file| {
            let file_path = match &file.item {
                PlayingItem::InLibrary(_) => Path::new(lib_path)
                    .join(&file.directory)
                    .join(&file.file_name),
                PlayingItem::IndependentFile(path_buf) => path_buf.to_path_buf(),
                PlayingItem::Unknown => Path::new("/").to_path_buf(),
            };

            match canonicalize(&file_path) {
                Ok(canonical_path) => Some((file.item.clone(), canonical_path)),
                Err(_) => None,
            }
        })
        .collect()
}

pub async fn load_request(
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<LoadRequest>,
) -> Result<()> {
    let volume = dart_signal.message.index;
    player.lock().await.load(volume as usize);
    Ok(())
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
    let item = player.lock().await.get_status().item;

    if let Some(item) = item {
        match item {
            playback::player::PlayingItem::InLibrary(file_id) => {
                increase_skipped(&main_db, file_id)
                    .await
                    .context("Unable to increase skipped count")?;
            }
            playback::player::PlayingItem::IndependentFile(_) => {}
            playback::player::PlayingItem::Unknown => {}
        };
    }

    player.lock().await.next();

    Ok(())
}

pub async fn previous_request(
    main_db: Arc<MainDbConnection>,
    player: Arc<Mutex<Player>>,
    _: DartSignal<PreviousRequest>,
) -> Result<()> {
    let item = player.lock().await.get_status().item;

    if let Some(item) = item {
        match item {
            playback::player::PlayingItem::InLibrary(file_id) => {
                increase_skipped(&main_db, file_id)
                    .await
                    .context("Unable to increase skipped count")?;
            }
            playback::player::PlayingItem::IndependentFile(_) => {}
            playback::player::PlayingItem::Unknown => {}
        };
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
    let item = player.lock().await.get_status().item;

    if let Some(item) = item {
        match item {
            playback::player::PlayingItem::InLibrary(file_id) => {
                increase_skipped(&main_db, file_id)
                    .await
                    .context("Unable to increase skipped count")?;
            }
            playback::player::PlayingItem::IndependentFile(_) => {}
            playback::player::PlayingItem::Unknown => {}
        };
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

pub async fn set_realtime_fft_enabled_request(
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<SetRealtimeFftEnabledRequest>,
) -> Result<()> {
    let request = dart_signal.message;
    let enabled = request.enabled;

    player.lock().await.set_realtime_fft_enabled(enabled);

    Ok(())
}

pub async fn set_adaptive_switching_enabled_request(
    player: Arc<Mutex<Player>>,
    dart_signal: DartSignal<SetAdaptiveSwitchingEnabledRequest>,
) -> Result<()> {
    let request = dart_signal.message;
    let enabled = request.enabled;

    player.lock().await.set_adaptive_switching_enabled(enabled);

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

    let items: Vec<PlayingItem> = request
        .fallback_playing_items
        .clone()
        .into_iter()
        .map(|x| x.into())
        .collect();

    // Retrieve tracks
    let tracks = if request.queries.is_empty() {
        PlayingItemActionDispatcher::new()
            .get_file_handle(&main_db, &items)
            .await?
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
        .into_iter()
        .map(|x| x.into())
        .collect()
    };

    let mut player = player.lock().await;

    let operate_mode = PlaylistOperateMode::try_from(request.operate_mode)?;
    // Clear the playlist if requested
    if operate_mode == PlaylistOperateMode::Replace {
        player.clear_playlist();
    }

    let add_mode = if operate_mode == PlaylistOperateMode::PlayNext {
        AddMode::PlayNext
    } else {
        AddMode::AppendToEnd
    };

    let playlist_len = if operate_mode == PlaylistOperateMode::Replace {
        0
    } else {
        player.get_playlist().len()
    };

    let mut items: Vec<PlayingItem> = tracks.iter().map(|x| x.clone().item).collect();

    // If not required to play instantly, add to playlist and return
    if !request.instantly_play {
        player.add_to_playlist(files_to_playback_request(&lib_path, &tracks), add_mode);
        OperatePlaybackWithMixQueryResponse {
            playing_items: items.into_iter().map(|x| x.into()).collect(),
        }
        .send_signal_to_dart();
        return Ok(());
    }

    // Find the nearest index
    let nearest_index: Option<usize> = if request.hint_position < 0 {
        Some(0)
    } else {
        find_nearest_index(&items, request.hint_position.try_into().unwrap(), |x| {
            if let Some(initial_item) = &request.initial_playback_item {
                *x == PlayingItem::from(initial_item.clone())
            } else {
                false
            }
        })
    };

    // If no suitable index found, use fallback_media_file_ids
    if nearest_index.is_none() {
        items = request
            .fallback_playing_items
            .into_iter()
            .map(|x| x.into())
            .collect();
    }

    let nearest_index = nearest_index.unwrap_or(request.hint_position.try_into().unwrap_or(0));

    // Add to playlist
    if !tracks.is_empty() {
        player.add_to_playlist(files_to_playback_request(&lib_path, &tracks), add_mode);
    }
    OperatePlaybackWithMixQueryResponse {
        playing_items: items.into_iter().map(|x| x.into()).collect(),
    }
    .send_signal_to_dart();

    // Set playback mode
    if request.playback_mode != 99 {
        player.set_playback_mode(request.playback_mode.into());
    }

    // Switch to the nearest index and play
    if !tracks.is_empty() {
        player.switch(nearest_index + playlist_len);
        player.play();
    }

    Ok(())
}
