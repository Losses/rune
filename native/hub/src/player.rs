use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{bail, Error};
use anyhow::{Context, Result};
use log::{debug, error, info};
use sea_orm::DatabaseConnection;
use tokio::sync::Mutex;
use tokio::task;

use database::actions::playback_queue::replace_playback_queue;
use database::actions::stats::increase_played_through;
use database::connection::MainDbConnection;
use database::playing_item::dispatcher::PlayingItemActionDispatcher;
use database::playing_item::library_item::extract_in_library_ids;
use database::playing_item::PlayingItemMetadataSummary;
use playback::controller::get_default_cover_art_path;
use playback::controller::handle_media_control_event;
use playback::controller::MediaControlManager;
use playback::player::PlaylistStatus;
use playback::player::{Player, PlayingItem};
use playback::MediaMetadata;
use playback::MediaPlayback;
use playback::MediaPosition;

use crate::{CrashResponse, PlaybackStatus, PlaylistItem, PlaylistUpdate, RealtimeFft};

pub async fn initialize_player(
    main_db: Arc<MainDbConnection>,
    player: Arc<Mutex<Player>>,
) -> Result<()> {
    let status_receiver = player.lock().await.subscribe_status();
    let played_through_receiver = player.lock().await.subscribe_played_through();
    let playlist_receiver = player.lock().await.subscribe_playlist();
    let realtime_fft_receiver = player.lock().await.subscribe_realtime_fft();
    let crash_receiver = player.lock().await.subscribe_crash();

    // Clone main_db for each task
    let main_db_for_status = Arc::clone(&main_db);
    let main_db_for_played_throudh = Arc::clone(&main_db);
    let main_db_for_playlist = Arc::clone(&main_db);

    let manager = Arc::new(Mutex::new(MediaControlManager::new()?));

    let os_controller_receiver = manager.lock().await.subscribe_controller_events();

    manager.lock().await.initialize()?;

    info!("Initializing event listeners");
    task::spawn(async move {
        let main_db = Arc::clone(&main_db_for_status);
        let mut cached_meta: Option<PlayingItemMetadataSummary> = None;
        let mut cached_cover_art: Option<String> = None;
        let mut last_status_item: Option<PlayingItem> = None;

        while let Ok(status) = status_receiver.recv().await {
            debug!("Player status updated: {:?}", status);

            let item = status.item.clone();

            let meta = match item {
                Some(item) => {
                    let dispatcher = PlayingItemActionDispatcher::new();
                    let item_clone = item.clone();

                    if last_status_item != Some(item) {
                        let item_clone_for_status = item_clone.clone();
                        let item_vec = &[item_clone].to_vec();

                        // Update the cached metadata if the index has changed
                        match dispatcher.bake_cover_art(&main_db, item_vec).await {
                            Ok(data) => {
                                let parsed_data = data.values().collect::<Vec<_>>();
                                cached_cover_art = if parsed_data.is_empty() {
                                    None
                                } else {
                                    Some(parsed_data[0].to_string())
                                }
                            }
                            Err(_) => todo!(),
                        };

                        match dispatcher.get_metadata_summary(&main_db, item_vec).await {
                            Ok(metadata) => match metadata.first() {
                                Some(metadata) => {
                                    cached_meta = Some(metadata.clone());
                                    last_status_item = Some(item_clone_for_status);

                                    let cover_art: Result<Option<String>> =
                                        match dispatcher.bake_cover_art(&main_db, item_vec).await {
                                            Ok(cover_art_map) => {
                                                let values: Vec<&String> =
                                                    cover_art_map.values().collect();

                                                if values.is_empty() {
                                                    Ok(None)
                                                } else {
                                                    Ok(Some(values[0].clone()))
                                                }
                                            }
                                            Err(_) => Ok(None),
                                        };

                                    let manager = Arc::clone(&manager);
                                    match update_media_controls_metadata(
                                        manager,
                                        metadata,
                                        cover_art.unwrap_or(None).as_deref(),
                                    )
                                    .await
                                    {
                                        Ok(_) => {}
                                        Err(e) => {
                                            error!(
                                                "Error updating OS media controller metadata: {:?}",
                                                e
                                            );
                                        }
                                    };
                                }
                                None => {
                                    error!("No metadata found for: {:?}", item_clone_for_status);
                                    cached_meta = None;
                                    last_status_item = Some(item_clone_for_status);
                                }
                            },
                            Err(e) => {
                                // Print the error if get_metadata_summary_by_file_id returns an error
                                error!("Error fetching metadata: {:?}", e);
                                cached_meta = None;
                                last_status_item = Some(item_clone_for_status);
                            }
                        }
                    }

                    cached_meta.clone().unwrap_or_default()
                }
                none => {
                    // If the index is None, send empty metadata
                    last_status_item = none;
                    PlayingItemMetadataSummary::default()
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
                item: status.item.map(Into::into),
                index: status.index.map(|i| i as i32),
                playback_mode: status.playback_mode.into(),
                ready: status.ready,
                cover_art_path: cached_cover_art.clone().unwrap_or_default(),
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
            match replace_playback_queue(&main_db, extract_in_library_ids(playlist.items)).await {
                Ok(_) => {}
                Err(e) => error!("Failed to update playback queue record: {:#?}", e),
            };
        }
    });

    task::spawn(async move {
        let main_db = Arc::clone(&main_db_for_played_throudh);

        while let Ok(item) = played_through_receiver.recv().await {
            match item {
                PlayingItem::InLibrary(id) => {
                    if let Err(e) = increase_played_through(&main_db, id)
                        .await
                        .with_context(|| "Unable to update played through count")
                    {
                        error!("{:?}", e);
                    }
                }
                PlayingItem::IndependentFile(_) => {}
                PlayingItem::Unknown => {}
            }
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

    task::spawn(async move {
        while let Ok(value) = crash_receiver.recv().await {
            CrashResponse { detail: value }.send_signal_to_dart();
        }
    });

    Ok(())
}

pub async fn send_playlist_update(db: &DatabaseConnection, playlist: &PlaylistStatus) {
    let items: Vec<PlayingItem> = playlist.items.clone();

    let dispatcher = PlayingItemActionDispatcher::new();

    match dispatcher.get_metadata_summary(db, &items.clone()).await {
        Ok(summaries) => {
            // Create a HashMap to store summaries by their id
            let summary_map: HashMap<PlayingItem, _> = summaries
                .into_iter()
                .map(|summary| (summary.item.clone(), summary))
                .collect();

            // Reorder items according to file_ids
            let items: Vec<PlaylistItem> = items
                .into_iter()
                .filter_map(|id| summary_map.get(&id))
                .map(|summary| PlaylistItem {
                    item: Some(summary.item.clone().into()),
                    artist: summary.artist.clone(),
                    album: summary.album.clone(),
                    title: summary.title.clone(),
                    duration: summary.duration,
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
    status: &PlayingItemMetadataSummary,
    cover_art_path: Option<&str>,
) -> Result<()> {
    let mut manager = manager.lock().await;

    let cover_url = if cover_art_path.is_none() {
        get_default_cover_art_path().to_str()
    } else {
        cover_art_path
    };

    let metadata = MediaMetadata {
        title: Some(&status.title),
        album: Some(&status.album),
        artist: Some(&status.artist),
        cover_url,
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
