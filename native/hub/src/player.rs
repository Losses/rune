use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{bail, Error};
use anyhow::{Context, Result};
use log::{debug, error, info};
use playback::controller::handle_media_control_event;
use sea_orm::DatabaseConnection;
use tokio::sync::Mutex;
use tokio::task;

use database::actions::metadata::get_metadata_summary_by_file_id;
use database::actions::metadata::get_metadata_summary_by_file_ids;
use database::actions::metadata::MetadataSummary;
use database::actions::stats::increase_played_through;
use database::connection::MainDbConnection;
use playback::controller::MediaControlManager;
use playback::player::Player;
use playback::player::PlaylistStatus;
use playback::MediaMetadata;
use playback::MediaPlayback;
use playback::MediaPosition;

use crate::{PlaybackStatus, PlaylistItem, PlaylistUpdate, RealtimeFft};

pub async fn initialize_player(
    main_db: Arc<MainDbConnection>,
    player: Arc<Mutex<Player>>,
) -> Result<()> {
    let mut status_receiver = player.lock().await.subscribe_status();
    let mut played_through_receiver = player.lock().await.subscribe_played_through();
    let mut playlist_receiver = player.lock().await.subscribe_playlist();
    let mut realtime_fft_receiver = player.lock().await.subscribe_realtime_fft();

    // Clone main_db for each task
    let main_db_for_status = Arc::clone(&main_db);
    let main_db_for_played_throudh = Arc::clone(&main_db);
    let main_db_for_playlist = Arc::clone(&main_db);

    let manager = Arc::new(Mutex::new(MediaControlManager::new()?));

    let mut os_controller_receiver = manager.lock().await.subscribe_controller_events();

    manager.lock().await.initialize()?;

    info!("Initializing event listeners");
    task::spawn(async move {
        let main_db = Arc::clone(&main_db_for_status);
        let mut cached_meta: Option<MetadataSummary> = None;
        let mut last_status_id: Option<i32> = None;

        while let Ok(status) = status_receiver.recv().await {
            debug!("Player status updated: {:?}", status);

            let meta = match status.id {
                Some(id) => {
                    if last_status_id != Some(id) {
                        // Update the cached metadata if the index has changed
                        match get_metadata_summary_by_file_id(&main_db, id).await {
                            Ok(metadata) => {
                                cached_meta = Some(metadata.clone());
                                last_status_id = Some(id);

                                let manager = Arc::clone(&manager);
                                match update_media_controls_metadata(manager, &metadata).await {
                                    Ok(_) => {}
                                    Err(e) => {
                                        error!(
                                            "Error updating OS media controller metadata: {:?}",
                                            e
                                        );
                                    }
                                };
                            }
                            Err(e) => {
                                // Print the error if get_metadata_summary_by_file_id returns an error
                                error!("Error fetching metadata: {:?}", e);
                                cached_meta = None;
                                last_status_id = Some(id);
                            }
                        }
                    }

                    cached_meta.clone().unwrap_or_default()
                }
                none => {
                    // If the index is None, send empty metadata
                    last_status_id = none;
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

            let formated_status = PlaybackStatus {
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
            };

            if let Err(e) =
                update_media_controls_progress(Arc::clone(&manager), &formated_status.clone())
                    .await
                    .with_context(|| "Failed to update media controls")
            {
                error!("{}", e);
                e.chain()
                    .skip(1)
                    .for_each(|cause| eprintln!("because: {}", cause));
            }

            formated_status.send_signal_to_dart();
        }
    });

    task::spawn(async move {
        let main_db = Arc::clone(&main_db_for_playlist);

        while let Ok(playlist) = playlist_receiver.recv().await {
            send_playlist_update(&main_db, &playlist).await;
        }
    });

    task::spawn(async move {
        let main_db = Arc::clone(&main_db_for_played_throudh);

        while let Ok(index) = played_through_receiver.recv().await {
            if let Err(e) = increase_played_through(&main_db, index)
                .await
                .with_context(|| "Unable to update played through count")
            {
                error!("{:?}", e);
            };
        }
    });

    task::spawn(async move {
        while let Ok(value) = realtime_fft_receiver.recv().await {
            send_realtime_fft(value).await;
        }
    });

    task::spawn(async move {
        let player = Arc::clone(&player);

        while let Ok(value) = os_controller_receiver.recv().await {
            if let Err(e) = handle_media_control_event(&player, value)
                .await
                .with_context(|| "Unable to handle control event")
            {
                e.chain()
                    .skip(1)
                    .for_each(|cause| eprintln!("because: {}", cause));
            };
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

async fn update_media_controls_metadata(
    manager: Arc<Mutex<MediaControlManager>>,
    status: &MetadataSummary,
) -> Result<()> {
    let mut manager = manager.lock().await;

    let metadata = MediaMetadata {
        title: Some(&status.title),
        album: Some(&status.album),
        artist: Some(&status.artist),
        cover_url: None,
        duration: Some(std::time::Duration::from_secs_f64(status.duration)),
    };

    match manager.controls.set_metadata(metadata) {
        Ok(x) => x,
        Err(e) => bail!(Error::msg(format!("Failed to set media metadata: {:?}", e))),
    };

    Ok(())
}

async fn update_media_controls_progress(
    manager: Arc<Mutex<MediaControlManager>>,
    status: &PlaybackStatus,
) -> Result<()> {
    let mut manager = manager.lock().await;

    let playback = match status.state.as_str() {
        "Playing" => MediaPlayback::Playing {
            progress: Some(MediaPosition(std::time::Duration::from_secs_f32(
                status.progress_seconds,
            ))),
        },
        "Paused" => MediaPlayback::Paused {
            progress: Some(MediaPosition(std::time::Duration::from_secs_f32(
                status.progress_seconds,
            ))),
        },
        _ => MediaPlayback::Stopped,
    };

    match manager.controls.set_playback(playback) {
        Ok(x) => x,
        Err(e) => bail!(Error::msg(format!(
            "Failed to set media playback status: {:?}",
            e
        ))),
    };

    Ok(())
}
