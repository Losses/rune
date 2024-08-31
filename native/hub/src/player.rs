use log::{debug, error, info};
use sea_orm::DatabaseConnection;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task;

use database::actions::metadata::{
    get_metadata_summary_by_file_id, get_metadata_summary_by_file_ids, MetadataSummary,
};
use database::connection::MainDbConnection;
use playback::player::{Player, PlaylistStatus};

use crate::common::Result;
use crate::messages;

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
