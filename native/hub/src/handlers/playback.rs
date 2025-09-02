use std::sync::Arc;

use anyhow::{Context, Result};
use fsio::FsIo;
use tokio::sync::Mutex;

use ::database::{
    actions::{mixes::query_mix_media_files, stats::increase_skipped},
    connection::{MainDbConnection, RecommendationDbConnection},
    playing_item::dispatcher::PlayingItemActionDispatcher,
};
use ::playback::{
    player::{Playable, PlayingItem},
    strategies::AddMode,
};

use crate::{
    Session, Signal,
    messages::*,
    utils::{GlobalParams, ParamsExtractor, files_to_playback_request, find_nearest_index},
};

impl From<PlayingItem> for PlayingItemRequest {
    fn from(x: PlayingItem) -> Self {
        match x {
            PlayingItem::InLibrary(x) => PlayingItemRequest {
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
            return PlayingItem::InLibrary(in_library.file_id);
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

impl ParamsExtractor for NextRequest {
    type Params = (Arc<MainDbConnection>, Arc<Mutex<dyn Playable>>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.player),
        )
    }
}

impl Signal for NextRequest {
    type Params = (Arc<MainDbConnection>, Arc<Mutex<dyn Playable>>);
    type Response = ();

    async fn handle(
        &self,
        (main_db, player): Self::Params,
        _session: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        let item = player.lock().await.get_status().item;

        if let Some(PlayingItem::InLibrary(file_id)) = item {
            increase_skipped(&main_db, file_id)
                .await
                .context("Unable to increase skipped count")?;
        }

        player.lock().await.next();
        Ok(Some(()))
    }
}

impl ParamsExtractor for PreviousRequest {
    type Params = (Arc<MainDbConnection>, Arc<Mutex<dyn Playable>>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.player),
        )
    }
}

impl Signal for PreviousRequest {
    type Params = (Arc<MainDbConnection>, Arc<Mutex<dyn Playable>>);
    type Response = ();

    async fn handle(
        &self,
        (main_db, player): Self::Params,
        _session: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        let item = player.lock().await.get_status().item;

        if let Some(PlayingItem::InLibrary(file_id)) = item {
            increase_skipped(&main_db, file_id)
                .await
                .context("Unable to increase skipped count")?;
        }

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
    type Params = (Arc<MainDbConnection>, Arc<Mutex<dyn Playable>>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.player),
        )
    }
}

impl Signal for SwitchRequest {
    type Params = (Arc<MainDbConnection>, Arc<Mutex<dyn Playable>>);
    type Response = ();

    async fn handle(
        &self,
        (main_db, player): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        if let Some(PlayingItem::InLibrary(file_id)) = player.lock().await.get_status().item {
            increase_skipped(&main_db, file_id)
                .await
                .context("Unable to increase skipped count")?;
        }

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

        player
            .lock()
            .await
            .move_playlist_item(old_index.try_into()?, new_index.try_into()?);
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
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
        Arc<String>,
        Arc<Mutex<dyn Playable>>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.fsio),
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.recommend_db),
            Arc::clone(&all_params.lib_path),
            Arc::clone(&all_params.player),
        )
    }
}

impl Signal for OperatePlaybackWithMixQueryRequest {
    type Params = (
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
        Arc<String>,
        Arc<Mutex<dyn Playable>>,
    );
    type Response = OperatePlaybackWithMixQueryResponse;

    async fn handle(
        &self,
        (fsio, main_db, recommend_db, lib_path, player): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
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
                .get_file_handle(&fsio, &main_db, &items)
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

        let mut items: Vec<PlayingItem> = tracks.iter().map(|x| x.clone().item).collect();

        // If not required to play instantly, add to playlist and return
        if !request.instantly_play {
            player.add_to_playlist(
                files_to_playback_request(&fsio, lib_path.as_ref(), &tracks),
                add_mode,
            );
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
                .clone()
                .into_iter()
                .map(|x| x.into())
                .collect();
        }

        let nearest_index = nearest_index.unwrap_or(request.hint_position.try_into().unwrap_or(0));

        // Add to playlist
        if !tracks.is_empty() {
            player.add_to_playlist(
                files_to_playback_request(&fsio, lib_path.as_ref(), &tracks),
                add_mode,
            );
        }

        // Set playback mode
        if request.playback_mode != 99 {
            player.set_playback_mode(request.playback_mode.into());
        }

        // Switch to the nearest index and play
        if !tracks.is_empty() {
            player.switch(nearest_index + playlist_len);
            player.play();
        }

        Ok(Some(OperatePlaybackWithMixQueryResponse {
            playing_items: items.into_iter().map(|x| x.into()).collect(),
        }))
    }
}
