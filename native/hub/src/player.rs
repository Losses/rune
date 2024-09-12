use std::collections::HashMap;
use std::sync::Arc;

use log::{debug, error, info};
use sea_orm::DatabaseConnection;
use tokio::sync::Mutex;
use tokio::task;

use database::actions::metadata::get_metadata_summary_by_file_id;
use database::actions::metadata::get_metadata_summary_by_file_ids;
use database::actions::metadata::MetadataSummary;
use database::connection::MainDbConnection;
use playback::player::Player;
use playback::player::PlaylistStatus;

use crate::common::Result;
use crate::{PlaybackStatus, PlaylistItem, PlaylistUpdate, RealtimeFft};

pub async fn initialize_player(
    main_db: Arc<MainDbConnection>,
    player: Arc<Mutex<Player>>,
) -> Result<()> {
    let mut status_receiver = player.lock().await.subscribe_status();
    let mut playlist_receiver = player.lock().await.subscribe_playlist();
    let mut realtime_fft_receiver = player.lock().await.subscribe_realtime_fft();

    // Clone main_db for each task
    let main_db_for_status = Arc::clone(&main_db);
    let main_db_for_playlist = Arc::clone(&main_db);

    info!("Initializing event listeners");
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

            PlaybackStatus {
                state: status.state.to_string(),
                progress_seconds: position.as_secs_f32(),
                progress_percentage,
                artist: Some(meta.artist.clone()),
                album: Some(meta.album.clone()),
                title: Some(meta.title.clone()),
                duration: Some(meta.duration),
                id: status.id,
                index: status.index.map(|i| i as i32),
                playback_mode: status.playback_mode.into(),
                ready: status.ready,
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

pub async fn send_playlist_update(db: &DatabaseConnection, playlist: &PlaylistStatus) {
    let file_ids: Vec<i32> = playlist.items.clone();

    match get_metadata_summary_by_file_ids(db, file_ids.clone()).await {
        Ok(summaries) => {
            // Create a HashMap to store summaries by their id
            let summary_map: HashMap<i32, _> =
                summaries.into_iter().map(|item| (item.id, item)).collect();

            // Reorder items according to file_ids
            let items: Vec<PlaylistItem> = file_ids
                .into_iter()
                .filter_map(|id| summary_map.get(&id))
                .map(|item| PlaylistItem {
                    id: item.id,
                    artist: item.artist.clone(),
                    album: item.album.clone(),
                    title: item.title.clone(),
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
    RealtimeFft { value }.send_signal_to_dart(); // GENERATED
}
