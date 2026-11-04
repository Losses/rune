use crate::request_error::ExpectedRequestError;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::Mutex;

use ::database::{
    actions::{mixes::query_mix_media_files, stats::increase_skipped},
    playing_item::dispatcher::PlayingItemActionDispatcher,
};
use ::playback::{
    player::{Playable, PlayingItem},
    strategies::AddMode,
};

use crate::{
    Session, Signal,
    library_pool::LibraryHandle,
    messages::*,
    node::NodeContext,
    utils::{
        GlobalParams, ParamsExtractor, files_to_playback_request, find_nearest_index,
        require_library,
    },
};

impl From<PlayingItem> for PlayingItemRequest {
    fn from(x: PlayingItem) -> Self {
        match x {
            PlayingItem::InLibrary(x, _) => PlayingItemRequest {
                in_library: Some(InLibraryPlayingItem { file_id: x }),
                independent_file: None,
            },
            PlayingItem::IndependentFile(path_str) => PlayingItemRequest {
                in_library: None,
                independent_file: Some(IndependentFilePlayingItem { raw_path: path_str }),
            },
            PlayingItem::Online(_, _) => todo!(),
            PlayingItem::Unknown => PlayingItemRequest {
                in_library: None,
                independent_file: None,
            },
        }
    }
}

impl From<PlayingItemRequest> for PlayingItem {
    fn from(x: PlayingItemRequest) -> Self {
        if let Some(in_library) = x.in_library
            && in_library.file_id != 0
        {
            return PlayingItem::InLibrary(in_library.file_id, String::new());
        }

        if let Some(independent_file) = x.independent_file
            && !independent_file.raw_path.is_empty()
        {
            return PlayingItem::IndependentFile(independent_file.raw_path);
        }

        PlayingItem::Unknown
    }
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
    if let Some(PlayingItem::InLibrary(file_id, library_id)) = item
        && let Ok(library) = node.library_pool.acquire(library_id.parse()?).await
    {
        let store = library.library_sync.record_store();
        increase_skipped(&library.main_db, &store, file_id, store.author())
            .await
            .context("Unable to increase skipped count")?;
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
    type Params = (Option<LibraryHandle>, Arc<String>, Arc<Mutex<dyn Playable>>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library.clone(),
            Arc::clone(&all_params.node_id),
            Arc::clone(&all_params.player),
        )
    }
}

impl Signal for OperatePlaybackWithMixQueryRequest {
    type Params = (Option<LibraryHandle>, Arc<String>, Arc<Mutex<dyn Playable>>);
    type Response = OperatePlaybackWithMixQueryResponse;

    async fn handle(
        &self,
        (library, _node_id, player): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;

        let items: Vec<PlayingItem> = request
            .fallback_playing_items
            .clone()
            .into_iter()
            .map(|x| x.into())
            .collect();

        // Retrieve tracks
        let tracks = if request.queries.is_empty() {
            PlayingItemActionDispatcher::new()
                .get_file_handle(&library.fsio, &library.main_db, &items)
                .await?
        } else {
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
                playing_items: items.into_iter().map(|x| x.into()).collect(),
            }));
        }

        // Find the nearest index
        let nearest_index: Option<usize> = if request.hint_position < 0 {
            Some(0)
        } else {
            find_nearest_index(&items, request.hint_position.try_into().unwrap(), |x| {
                if let Some(initial_item) = &request.initial_playback_item {
                    let mut initial = PlayingItem::from(initial_item.clone());
                    if let PlayingItem::InLibrary(_, library_id) = &mut initial {
                        *library_id = library.descriptor.library_id.to_string();
                    }
                    *x == initial
                } else {
                    false
                }
            })
        };

        // If no suitable index found, use fallback_media_file_ids
        if nearest_index.is_none() {
            items = request
                .fallback_playing_items
                .clone()
                .into_iter()
                .map(|x| x.into())
                .collect();
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
            playing_items: items.into_iter().map(|x| x.into()).collect(),
        }))
    }
}
