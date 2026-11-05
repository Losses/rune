use std::collections::HashMap;
use std::slice;
use std::sync::Arc;

use anyhow::{Context, Result};
use log::warn;
use tokio::sync::Mutex;
use uuid::Uuid;

use ::database::{
    actions::{
        file::{file_ids_by_uuid, file_uuids_by_id},
        mixes::query_mix_media_files,
        stats::increase_skipped,
    },
    playing_item::dispatcher::PlayingItemActionDispatcher,
};
use ::playback::{
    player::{LibraryItem, Playable, PlayingItem},
    strategies::AddMode,
};

use crate::request_error::ExpectedRequestError;
use crate::{
    Session, Signal,
    library_pool::{LibraryHandle, LibraryPool},
    messages::*,
    node::NodeContext,
    utils::{
        GlobalParams, ParamsExtractor, files_to_playback_request, find_nearest_index,
        require_library,
    },
};

pub async fn playing_item_from_request(
    request: PlayingItemRequest,
    library: Option<&LibraryHandle>,
) -> Result<PlayingItem> {
    let mut items = playing_items_from_requests(slice::from_ref(&request), library).await?;
    Ok(items.pop().unwrap_or(PlayingItem::Unknown))
}

/// Convert many requests against one library with a single id lookup.
pub async fn playing_items_from_requests(
    requests: &[PlayingItemRequest],
    library: Option<&LibraryHandle>,
) -> Result<Vec<PlayingItem>> {
    let library_items = requests
        .iter()
        .map(|request| {
            request
                .library_item
                .as_ref()
                .map(|item| {
                    Ok(LibraryItem {
                        library_id: Uuid::parse_str(&item.library_id).map_err(|error| {
                            ExpectedRequestError::invalid_input(format!(
                                "Invalid library id in library item: {error}"
                            ))
                        })?,
                        file_uuid: Uuid::parse_str(&item.file_uuid).map_err(|error| {
                            ExpectedRequestError::invalid_input(format!(
                                "Invalid file uuid in library item: {error}"
                            ))
                        })?,
                    })
                })
                .transpose()
        })
        .collect::<Result<Vec<_>>>()?;
    let ids = requests
        .iter()
        .filter(|request| request.library_item.is_none())
        .filter_map(|request| request.in_library.as_ref())
        .map(|in_library| in_library.file_id)
        .filter(|id| *id != 0)
        .collect::<Vec<_>>();
    let uuids = if ids.is_empty() {
        HashMap::new()
    } else {
        let library = library.ok_or(ExpectedRequestError::Unavailable {
            what: "UI library for integer item",
        })?;
        file_uuids_by_id(&library.main_db, &ids).await?
    };
    Ok(requests
        .iter()
        .zip(library_items)
        .map(|(request, library_item)| {
            if let Some(item) = library_item {
                return PlayingItem::InLibrary(item);
            }
            if let Some(in_library) = &request.in_library
                && in_library.file_id != 0
            {
                return uuids
                    .get(&in_library.file_id)
                    .zip(library)
                    .map(|(file_uuid, library)| {
                        PlayingItem::InLibrary(LibraryItem {
                            library_id: library.descriptor.library_id,
                            file_uuid: *file_uuid,
                        })
                    })
                    .unwrap_or(PlayingItem::Unknown);
            }
            match &request.independent_file {
                Some(file) if !file.raw_path.is_empty() => {
                    PlayingItem::IndependentFile(file.raw_path.clone())
                }
                _ => PlayingItem::Unknown,
            }
        })
        .collect())
}

/// Convert many items to requests, acquiring each library and querying it once.
pub async fn playing_item_requests(
    items: &[PlayingItem],
    pool: &Arc<LibraryPool>,
    ui_library_id: Option<Uuid>,
) -> Vec<PlayingItemRequest> {
    let mut by_library: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for item in items {
        if let PlayingItem::InLibrary(identity) = item
            && Some(identity.library_id) == ui_library_id
        {
            by_library
                .entry(identity.library_id)
                .or_default()
                .push(identity.file_uuid);
        }
    }
    let mut ids: HashMap<LibraryItem, i32> = HashMap::new();
    for (library_id, file_uuids) in by_library {
        let resolved = async {
            let library = pool.acquire(library_id).await?;
            file_ids_by_uuid(&library.main_db, &file_uuids).await
        }
        .await;
        match resolved {
            Ok(resolved) => ids.extend(resolved.into_iter().map(|(file_uuid, id)| {
                (
                    LibraryItem {
                        library_id,
                        file_uuid,
                    },
                    id,
                )
            })),
            Err(error) => {
                warn!("Unable to resolve queued items of library {library_id}: {error:#}")
            }
        }
    }
    items
        .iter()
        .map(|item| match item {
            PlayingItem::InLibrary(identity) => PlayingItemRequest {
                in_library: ids
                    .get(identity)
                    .map(|file_id| InLibraryPlayingItem { file_id: *file_id }),
                independent_file: None,
                library_item: (!ids.contains_key(identity))
                    .then(|| LibraryItemRequest::from(*identity)),
            },
            PlayingItem::IndependentFile(path) => PlayingItemRequest {
                in_library: None,
                independent_file: Some(IndependentFilePlayingItem {
                    raw_path: path.clone(),
                }),
                library_item: None,
            },
            PlayingItem::Online(_, _) | PlayingItem::Unknown => PlayingItemRequest {
                in_library: None,
                independent_file: None,
                library_item: None,
            },
        })
        .collect()
}

impl ParamsExtractor for LoadRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.player),)
    }
}

impl Signal for LoadRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);
    type Response = ();

    async fn handle(
        &self,
        (player,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let volume = dart_signal.index;
        player.lock().await.load(volume as usize);
        Ok(Some(()))
    }
}

impl ParamsExtractor for PlayRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.player),)
    }
}

impl Signal for PlayRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);
    type Response = ();

    async fn handle(
        &self,
        (player,): Self::Params,
        _session: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        player.lock().await.play();
        Ok(Some(()))
    }
}

impl ParamsExtractor for PauseRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.player),)
    }
}

impl Signal for PauseRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);
    type Response = ();

    async fn handle(
        &self,
        (player,): Self::Params,
        _session: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        player.lock().await.pause();
        Ok(Some(()))
    }
}

async fn record_skip(node: &NodeContext, player: &Arc<Mutex<dyn Playable>>) -> Result<()> {
    let item = player.lock().await.get_status().item;
    if let Some(PlayingItem::InLibrary(identity)) = item
        && let Ok(library) = node.library_pool.acquire(identity.library_id).await
    {
        let ids = file_ids_by_uuid(&library.main_db, &[identity.file_uuid]).await?;
        if let Some(file_id) = ids.get(&identity.file_uuid) {
            let store = library.library_sync.record_store();
            increase_skipped(&library.main_db, &store, *file_id, store.author())
                .await
                .context("Unable to increase skipped count")?;
        }
    }
    Ok(())
}

impl ParamsExtractor for NextRequest {
    type Params = (Arc<NodeContext>, Arc<Mutex<dyn Playable>>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.node.clone(), Arc::clone(&all_params.player))
    }
}

impl Signal for NextRequest {
    type Params = (Arc<NodeContext>, Arc<Mutex<dyn Playable>>);
    type Response = ();

    async fn handle(
        &self,
        (node, player): Self::Params,
        _session: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        record_skip(&node, &player).await?;

        player.lock().await.next();
        Ok(Some(()))
    }
}

impl ParamsExtractor for PreviousRequest {
    type Params = (Arc<NodeContext>, Arc<Mutex<dyn Playable>>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.node.clone(), Arc::clone(&all_params.player))
    }
}

impl Signal for PreviousRequest {
    type Params = (Arc<NodeContext>, Arc<Mutex<dyn Playable>>);
    type Response = ();

    async fn handle(
        &self,
        (node, player): Self::Params,
        _session: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        record_skip(&node, &player).await?;

        player.lock().await.previous();
        Ok(Some(()))
    }
}

impl ParamsExtractor for SetPlaybackModeRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.player),)
    }
}

impl Signal for SetPlaybackModeRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);
    type Response = ();

    async fn handle(
        &self,
        (player,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let mode = dart_signal.mode;
        player.lock().await.set_playback_mode(mode.into());
        Ok(Some(()))
    }
}

impl ParamsExtractor for SwitchRequest {
    type Params = (Arc<NodeContext>, Arc<Mutex<dyn Playable>>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.node.clone(), Arc::clone(&all_params.player))
    }
}

impl Signal for SwitchRequest {
    type Params = (Arc<NodeContext>, Arc<Mutex<dyn Playable>>);
    type Response = ();

    async fn handle(
        &self,
        (node, player): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        record_skip(&node, &player).await?;

        player
            .lock()
            .await
            .switch(dart_signal.index.try_into().unwrap());

        Ok(Some(()))
    }
}

impl ParamsExtractor for SeekRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.player),)
    }
}

impl Signal for SeekRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);
    type Response = ();

    async fn handle(
        &self,
        (player,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        player.lock().await.seek(dart_signal.position_seconds);
        Ok(Some(()))
    }
}

impl ParamsExtractor for RemoveRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.player),)
    }
}

impl Signal for RemoveRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);
    type Response = ();

    async fn handle(
        &self,
        (player,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        player
            .lock()
            .await
            .remove_from_playlist(dart_signal.index as usize);
        Ok(Some(()))
    }
}

impl ParamsExtractor for VolumeRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.player),)
    }
}

impl Signal for VolumeRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);
    type Response = VolumeResponse;

    async fn handle(
        &self,
        (player,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let volume = dart_signal.volume;
        player.lock().await.set_volume(volume);
        Ok(Some(VolumeResponse { volume }))
    }
}

impl ParamsExtractor for MovePlaylistItemRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.player),)
    }
}

impl Signal for MovePlaylistItemRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);
    type Response = ();

    async fn handle(
        &self,
        (player,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let old_index = request.old_index;
        let new_index = request.new_index;

        player.lock().await.move_playlist_item(
            old_index.try_into().map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid old index: {error}"))
            })?,
            new_index.try_into().map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid new index: {error}"))
            })?,
        );
        Ok(Some(()))
    }
}

impl ParamsExtractor for SetRealtimeFFTEnabledRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.player),)
    }
}

impl Signal for SetRealtimeFFTEnabledRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);
    type Response = ();

    async fn handle(
        &self,
        (player,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let enabled = dart_signal.enabled;
        player.lock().await.set_realtime_fft_enabled(enabled);
        Ok(Some(()))
    }
}

impl ParamsExtractor for SetAdaptiveSwitchingEnabledRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.player),)
    }
}

impl Signal for SetAdaptiveSwitchingEnabledRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);
    type Response = ();

    async fn handle(
        &self,
        (player,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let enabled = dart_signal.enabled;
        player.lock().await.set_adaptive_switching_enabled(enabled);
        Ok(Some(()))
    }
}

impl ParamsExtractor for OperatePlaybackWithMixQueryRequest {
    type Params = (
        Option<LibraryHandle>,
        Arc<String>,
        Arc<Mutex<dyn Playable>>,
        Arc<NodeContext>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library.clone(),
            Arc::clone(&all_params.node_id),
            Arc::clone(&all_params.player),
            all_params.node.clone(),
        )
    }
}

impl Signal for OperatePlaybackWithMixQueryRequest {
    type Params = (
        Option<LibraryHandle>,
        Arc<String>,
        Arc<Mutex<dyn Playable>>,
        Arc<NodeContext>,
    );
    type Response = OperatePlaybackWithMixQueryResponse;

    async fn handle(
        &self,
        (library, _node_id, player, node): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;

        let items: Vec<PlayingItem> =
            playing_items_from_requests(&request.fallback_playing_items, Some(&library)).await?;

        // Retrieve tracks
        let tracks = if request.queries.is_empty() {
            PlayingItemActionDispatcher::new()
                .get_file_handle(&library.fsio, &library.main_db, &items)
                .await?
        } else {
            let queue = node
                .session_queue_for_library(library.descriptor.library_id)
                .await?;
            query_mix_media_files(
                &library.main_db,
                &library.recommend_db,
                request
                    .queries
                    .iter()
                    .map(|x| (x.operator.clone(), x.parameter.clone()))
                    .collect(),
                0,
                4096,
                &queue,
            )
            .await
            .with_context(|| format!("Failed to query tracks: {:?}", request.queries))?
            .into_iter()
            .map(|x| x.into())
            .collect()
        };

        let prepared = files_to_playback_request(
            &library.fsio,
            &library.main_db,
            &library.descriptor.library_id.to_string(),
            &library.media_root,
            &tracks,
        )
        .await?;
        let mut player = player.lock().await;

        let operate_mode = request.operate_mode;
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

        let mut items: Vec<PlayingItem> = prepared.iter().map(|(item, _)| item.clone()).collect();

        // If not required to play instantly, add to playlist and return
        if !request.instantly_play {
            player.add_to_playlist_with_sources(prepared.clone(), add_mode);
            return Ok(Some(OperatePlaybackWithMixQueryResponse {
                playing_items: playing_item_requests(
                    &items,
                    &node.library_pool,
                    Some(library.descriptor.library_id),
                )
                .await,
            }));
        }

        // Find the nearest index
        let initial_item = if let Some(initial_item) = &request.initial_playback_item {
            Some(playing_item_from_request(initial_item.clone(), Some(&library)).await?)
        } else {
            None
        };
        let nearest_index: Option<usize> = if request.hint_position < 0 {
            Some(0)
        } else {
            find_nearest_index(&items, request.hint_position.try_into().unwrap(), |x| {
                if let Some(initial_item) = &initial_item {
                    x == initial_item
                } else {
                    false
                }
            })
        };

        // If no suitable index found, use fallback_media_file_ids
        if nearest_index.is_none() {
            items = playing_items_from_requests(&request.fallback_playing_items, Some(&library))
                .await?;
        }

        let nearest_index = nearest_index.unwrap_or(request.hint_position.try_into().unwrap_or(0));

        // Add to playlist
        if !prepared.is_empty() {
            player.add_to_playlist_with_sources(prepared.clone(), add_mode);
        }

        // Set playback mode
        if request.playback_mode != 99 {
            player.set_playback_mode(request.playback_mode.into());
        }

        // Switch to the nearest index and play
        if !prepared.is_empty() {
            player.switch(nearest_index + playlist_len);
            player.play();
        }

        Ok(Some(OperatePlaybackWithMixQueryResponse {
            playing_items: playing_item_requests(
                &items,
                &node.library_pool,
                Some(library.descriptor.library_id),
            )
            .await,
        }))
    }
}
