use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use log::warn;
use tokio::sync::{Mutex, broadcast, broadcast::error::RecvError};
use uuid::Uuid;

use ::database::{
    actions::{
        file::{file_ids_by_uuid, files_by_uuid},
        mixes::query_mix_media_files,
        stats::increase_skipped,
    },
    error::ExpectedError,
    library_registry::{LibraryRegistry, ProtocolAdapter},
    playing_item::{MediaFileHandle, dispatcher::PlayingItemActionDispatcher},
};
use ::playback::player::{
    HandoffBlockId as PlayerHandoffBlockId, LibraryItem, Playable, PlaybackSource, PlaybackState,
    PlayerStatus, PlayingItem, PlaylistEdit, PlaylistLoad, PlaylistPlacement,
};

use crate::backends::remote::{RendererLink, RendererLinkUse};
use crate::node::HandoffPlaybackEvent;
use crate::request_error::ExpectedRequestError;
use crate::server::libraries::{cast_library_state, cast_library_states, parse_library_ids};
use crate::subsonic_binding::SubsonicBinding;
use crate::{
    Session, Signal,
    library_pool::{LibraryHandle, LibraryPool},
    messages::*,
    node::NodeContext,
    rpc::dispatch_gui_request,
    session::{HandoffLoadParams, PlayerSession},
    utils::{
        GlobalParams, ParamsExtractor, files_to_playback_request, find_nearest_index,
        require_library,
    },
};

const HANDOFF_START_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn playing_item_from_request(
    request: PlayingItemRef,
    _library: Option<&LibraryHandle>,
) -> Result<PlayingItem> {
    Ok(match request {
        PlayingItemRef::LibraryTrack { library_id, track } => PlayingItem::LibraryTrack {
            library_id: Uuid::parse_str(&library_id).map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid library id: {error}"))
            })?,
            track,
        },
        PlayingItemRef::IndependentFile { path } if !path.is_empty() => {
            PlayingItem::IndependentFile(path)
        }
        PlayingItemRef::IndependentFile { .. } => PlayingItem::Unknown,
    })
}

pub(crate) fn reject_remote_independent_item(
    session: Option<&Session>,
    item: &PlayingItemRef,
) -> Result<()> {
    if session.is_some() && matches!(item, PlayingItemRef::IndependentFile { .. }) {
        return Err(ExpectedError::InvalidInput {
            detail: "independent files are only available on this device".into(),
        }
        .into());
    }
    Ok(())
}

/// Convert many requests against one library with a single id lookup.
pub async fn playing_items_from_requests(
    requests: &[PlayingItemRef],
    _library: Option<&LibraryHandle>,
) -> Result<Vec<PlayingItem>> {
    requests
        .iter()
        .cloned()
        .map(|request| async move { playing_item_from_request(request, None).await })
        .collect::<futures::future::JoinAll<_>>()
        .await
        .into_iter()
        .collect()
}

async fn resolve_open_subsonic_tracks(
    requests: &[PlayingItemRef],
    _binding: &SubsonicBinding,
) -> Result<Vec<PlayingItem>> {
    let mut resolved = Vec::with_capacity(requests.len());
    for request in requests {
        resolved.extend(playing_items_from_requests(std::slice::from_ref(request), None).await?);
    }
    Ok(resolved)
}

struct PlaylistEditOptions {
    mode: PlaylistOperateMode,
    start_index: Option<usize>,
    playback_mode: Option<u32>,
    position: Option<Duration>,
    play: Option<bool>,
}

fn edit_playlist(
    player: &mut dyn Playable,
    prepared: &[(PlayingItem, PlaybackSource)],
    options: PlaylistEditOptions,
) -> bool {
    let PlaylistEditOptions {
        mode,
        start_index,
        playback_mode,
        position,
        play,
    } = options;
    let placement = match mode {
        PlaylistOperateMode::PlayNext => PlaylistPlacement::PlayNext,
        PlaylistOperateMode::AppendToEnd => PlaylistPlacement::AppendToEnd,
        PlaylistOperateMode::Replace => PlaylistPlacement::Replace,
    };
    let load = start_index
        .filter(|_| !prepared.is_empty())
        .map(|start_index| PlaylistLoad {
            start_index,
            position: position.unwrap_or_default(),
            play: if position.is_some() || play.is_some() {
                play.unwrap_or(playback_mode.is_some())
            } else {
                true
            },
        });
    let changed = (mode == PlaylistOperateMode::Replace && !player.get_playlist().is_empty())
        || !prepared.is_empty();
    player.edit_playlist(PlaylistEdit {
        tracks: prepared.to_vec(),
        placement,
        playback_mode: playback_mode.filter(|mode| *mode != 99).map(Into::into),
        load,
        block: None,
    });
    changed
}

/// Convert many items to requests, acquiring each library and querying it once.
pub async fn playing_item_requests(
    items: &[PlayingItem],
    _pool: &Arc<LibraryPool>,
    _ui_library_id: Option<Uuid>,
) -> Vec<PlayingItemRef> {
    items
        .iter()
        .map(|item| match item {
            PlayingItem::LibraryTrack { library_id, track } => PlayingItemRef::LibraryTrack {
                library_id: library_id.to_string(),
                track: track.clone(),
            },
            PlayingItem::IndependentFile(path) => {
                PlayingItemRef::IndependentFile { path: path.clone() }
            }
            PlayingItem::Online(_, _) | PlayingItem::Unknown => PlayingItemRef::IndependentFile {
                path: String::new(),
            },
        })
        .collect()
}

struct ResolvedMixQuery {
    playing_items: Vec<PlayingItemRef>,
    start_index: Option<usize>,
}

async fn resolve_mix_query(
    request: &ResolveMixQueryRequest,
    library: &LibraryHandle,
    params: &GlobalParams,
) -> Result<ResolvedMixQuery> {
    let fallback_items =
        playing_items_from_requests(&request.fallback_playing_items, Some(library)).await?;

    let tracks = if request.queries.is_empty() {
        PlayingItemActionDispatcher::new()
            .get_file_handle(&library.fsio, &library.main_db, &fallback_items)
            .await?
    } else {
        let queue = if request.exclude_current_queue {
            let snapshot = dispatch_gui_request(params, &SessionSnapshotRequest { call_id: None })
                .await?
                .context("Current player did not return a session snapshot")?;
            let track_uuids = snapshot
                .items
                .into_iter()
                .filter_map(|item| match item {
                    PlayingItemRef::LibraryTrack { library_id, track }
                        if library_id == library.library_id.to_string() =>
                    {
                        Uuid::parse_str(&track).ok()
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            let ids = file_ids_by_uuid(&library.main_db, &track_uuids).await?;
            track_uuids
                .iter()
                .filter_map(|uuid| ids.get(uuid).copied())
                .collect()
        } else {
            Vec::new()
        };
        let excluded_tracks = queue.iter().copied().collect::<HashSet<_>>();
        query_mix_media_files(
            &library.main_db,
            &library.recommend_db,
            request
                .queries
                .iter()
                .map(|query| (query.operator.clone(), query.parameter.clone()))
                .collect(),
            0,
            4096,
            &queue,
        )
        .await
        .with_context(|| format!("Failed to query tracks: {:?}", request.queries))?
        .into_iter()
        .filter(|track| !excluded_tracks.contains(&track.id))
        .map(Into::into)
        .collect()
    };

    let prepared = files_to_playback_request(
        &library.fsio,
        &library.main_db,
        &library.library_id.to_string(),
        &library.media_root,
        &tracks,
    )
    .await?;
    let mut items: Vec<PlayingItem> = prepared.iter().map(|(item, _)| item.clone()).collect();
    let start_index = if !request.instantly_play {
        None
    } else {
        let initial_item = match &request.initial_playback_item {
            Some(initial_item) => {
                Some(playing_item_from_request(initial_item.clone(), Some(library)).await?)
            }
            None => None,
        };
        let nearest_index = if request.hint_position < 0 {
            Some(0)
        } else {
            let hint_position = usize::try_from(request.hint_position).map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid hint position: {error}"))
            })?;
            find_nearest_index(&items, hint_position, |item| {
                initial_item
                    .as_ref()
                    .is_some_and(|initial_item| item == initial_item)
            })
        };

        if nearest_index.is_none() {
            items = fallback_items;
            Some(if request.hint_position < 0 {
                0
            } else {
                usize::try_from(request.hint_position).unwrap_or_default()
            })
        } else {
            nearest_index
        }
    };
    let playing_items = playing_item_requests(&items, &params.library_pool, None).await;

    Ok(ResolvedMixQuery {
        playing_items,
        start_index,
    })
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
        player.lock().await.load_applied(volume as usize).await?;
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
        player.lock().await.play_applied().await?;
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
        player.lock().await.pause_applied().await?;
        Ok(Some(()))
    }
}

async fn record_skip(node: &NodeContext, player: &Arc<Mutex<dyn Playable>>) -> Result<()> {
    let item = player.lock().await.get_status().item;
    record_skip_item(node, item).await
}

async fn record_skip_item(node: &NodeContext, item: Option<PlayingItem>) -> Result<()> {
    if let Some(PlayingItem::LibraryTrack { library_id, track }) = item
        && let Ok(file_uuid) = Uuid::parse_str(&track)
        && let Ok(library) = node.library_pool.acquire(library_id).await
    {
        let ids = file_ids_by_uuid(&library.main_db, &[file_uuid]).await?;
        if let Some(file_id) = ids.get(&file_uuid) {
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

        player.lock().await.next_applied().await?;
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

        player.lock().await.previous_applied().await?;
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
        player
            .lock()
            .await
            .set_playback_mode_applied(mode.into())
            .await?;
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
        (node, _player): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let index: usize = dart_signal.index.try_into().map_err(|error| {
            ExpectedRequestError::invalid_input(format!("Invalid switch index: {error}"))
        })?;
        let mut previous_item = None;
        let mut selected_item: Option<PlayingItem> = None;
        let mut invalid_index = false;
        node.session()?
            .edit_queue(dart_signal.expected_revision, |player| {
                let playlist = player.get_playlist();
                if index >= playlist.len() {
                    invalid_index = true;
                    return false;
                }
                previous_item = player.get_status().item;
                selected_item = Some(playlist[index].clone());
                player.switch(index);
                false
            })
            .await?;
        if invalid_index {
            return Err(
                ExpectedRequestError::invalid_input("Invalid switch index: out of range").into(),
            );
        }
        if let Some(item) = selected_item {
            node.session()?
                .player
                .lock()
                .await
                .wait_for_item_applied(item, PlaybackState::Playing)
                .await?;
        }
        record_skip_item(&node, previous_item).await?;

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
        player
            .lock()
            .await
            .seek_applied(dart_signal.position_seconds)
            .await?;
        Ok(Some(()))
    }
}

impl ParamsExtractor for RemoveRequest {
    type Params = (Arc<NodeContext>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.node.clone(),)
    }
}

impl Signal for RemoveRequest {
    type Params = (Arc<NodeContext>,);
    type Response = ();

    async fn handle(
        &self,
        (node,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        node.session()?
            .edit_queue(dart_signal.expected_revision, |player| {
                let changed = (dart_signal.index as usize) < player.get_playlist().len();
                player.remove_from_playlist(dart_signal.index as usize);
                changed
            })
            .await?;
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
        player.lock().await.set_volume_applied(volume).await?;
        Ok(Some(VolumeResponse {
            call_id: None,
            volume,
        }))
    }
}

impl ParamsExtractor for MovePlaylistItemRequest {
    type Params = (Arc<NodeContext>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.node.clone(),)
    }
}

impl Signal for MovePlaylistItemRequest {
    type Params = (Arc<NodeContext>,);
    type Response = ();

    async fn handle(
        &self,
        (node,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let old_index = request.old_index;
        let new_index = request.new_index;

        let old_index = old_index.try_into().map_err(|error| {
            ExpectedRequestError::invalid_input(format!("Invalid old index: {error}"))
        })?;
        let new_index = new_index.try_into().map_err(|error| {
            ExpectedRequestError::invalid_input(format!("Invalid new index: {error}"))
        })?;
        node.session()?
            .edit_queue(dart_signal.expected_revision, |player| {
                let len = player.get_playlist().len();
                let changed = old_index != new_index && old_index < len && new_index < len;
                player.move_playlist_item(old_index, new_index);
                changed
            })
            .await?;
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
        player
            .lock()
            .await
            .set_realtime_fft_enabled_applied(enabled)
            .await?;
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
        player
            .lock()
            .await
            .set_adaptive_switching_enabled_applied(enabled)
            .await?;
        Ok(Some(()))
    }
}

impl ParamsExtractor for SetMixWithOthersRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.player),)
    }
}

impl Signal for SetMixWithOthersRequest {
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
            .set_mix_with_others_applied(dart_signal.enabled)
            .await?;
        Ok(Some(()))
    }
}

impl ParamsExtractor for ResolveMixQueryRequest {
    type Params = GlobalParams;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.clone()
    }
}

impl Signal for ResolveMixQueryRequest {
    type Params = GlobalParams;
    type Response = ResolveMixQueryResponse;

    async fn handle(
        &self,
        params: Self::Params,
        session: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        if let Some(item) = &request.initial_playback_item {
            reject_remote_independent_item(session.as_ref(), item)?;
        }
        for item in &request.fallback_playing_items {
            reject_remote_independent_item(session.as_ref(), item)?;
        }
        let library = require_library(params.library.clone())?;
        let resolved = resolve_mix_query(request, &library, &params).await?;
        let start_index = resolved
            .start_index
            .map(u32::try_from)
            .transpose()
            .map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid start index: {error}"))
            })?;
        Ok(Some(ResolveMixQueryResponse {
            call_id: None,
            playing_items: resolved.playing_items,
            start_index,
        }))
    }
}

impl ParamsExtractor for LoadResolvedItemsRequest {
    type Params = (Option<LibraryHandle>, Arc<NodeContext>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.library.clone(), all_params.node.clone())
    }
}

impl ParamsExtractor for HandoffLoadRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for HandoffLoadRequest {
    type Params = Arc<NodeContext>;
    type Response = HandoffLoadResponse;

    async fn handle(
        &self,
        node: Self::Params,
        session: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        if session.is_none() {
            return Err(ExpectedRequestError::Unavailable {
                what: "renderer session",
            }
            .into());
        }
        let handoff_item = apply_handoff_load(
            &node,
            PlayerHandoffBlockId::try_from(&request.block)?,
            &request.load,
        )
        .await?;
        Ok(Some(HandoffLoadResponse {
            call_id: None,
            handoff_item,
        }))
    }
}

impl ParamsExtractor for TakeBackHandoffBlockRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for TakeBackHandoffBlockRequest {
    type Params = Arc<NodeContext>;
    type Response = SessionSnapshotResponse;

    async fn handle(
        &self,
        node: Self::Params,
        session: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        if session.is_none() {
            return Err(ExpectedRequestError::Unavailable {
                what: "renderer session",
            }
            .into());
        }
        let (result, revision) = node
            .session()?
            .take_back_handoff_block(PlayerHandoffBlockId::try_from(&request.block)?)
            .await?;
        let items = result
            .items
            .into_iter()
            .map(|entry| match entry.item {
                PlayingItem::LibraryTrack { library_id, track } => {
                    Ok(PlayingItemRef::LibraryTrack {
                        library_id: library_id.to_string(),
                        track,
                    })
                }
                _ => Err(ExpectedRequestError::Unavailable {
                    what: "handoff block item",
                }
                .into()),
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(SessionSnapshotResponse {
            call_id: None,
            items,
            index: result
                .index
                .map(u32::try_from)
                .transpose()
                .map_err(|error| {
                    ExpectedRequestError::invalid_input(format!(
                        "Invalid handoff block index: {error}"
                    ))
                })?,
            position_seconds: result.position.as_secs_f64(),
            playback_mode: result.playback_mode.into(),
            playing: result.playing,
            revision,
        }))
    }
}

impl Signal for LoadResolvedItemsRequest {
    type Params = (Option<LibraryHandle>, Arc<NodeContext>);
    type Response = LoadResolvedItemsResponse;

    async fn handle(
        &self,
        (library, node): Self::Params,
        session: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        if session.is_some() {
            let result = apply_cast_load(&node, request).await?;
            return Ok(Some(LoadResolvedItemsResponse {
                call_id: None,
                result,
            }));
        } else {
            if let Some(binding) = node.open_subsonic_binding()?
                && binding.play_on_this_device
            {
                let items = resolve_open_subsonic_tracks(&request.items, &binding).await?;
                let prepared = items
                    .into_iter()
                    .map(|item| (item, PlaybackSource::LibraryStream))
                    .collect::<Vec<_>>();
                let start_index = request
                    .start_index
                    .map(usize::try_from)
                    .transpose()
                    .map_err(|error| {
                        ExpectedRequestError::invalid_input(format!("Invalid start index: {error}"))
                    })?;
                node.session()?
                    .edit_queue(request.expected_revision, |player| {
                        edit_playlist(
                            player,
                            &prepared,
                            PlaylistEditOptions {
                                mode: request.mode,
                                start_index,
                                playback_mode: request.playback_mode,
                                position: request
                                    .position_seconds
                                    .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok()),
                                play: request.play,
                            },
                        )
                    })
                    .await?;
                return Ok(Some(LoadResolvedItemsResponse {
                    call_id: None,
                    result: ResolvedItemsLoadResult::default(),
                }));
            }
            let library = require_library(library)?;
            let items = playing_items_from_requests(&request.items, Some(&library)).await?;
            let mut items_by_library: HashMap<Uuid, Vec<PlayingItem>> = HashMap::new();
            let mut local_items = Vec::new();
            let mut prepared_by_item = HashMap::new();
            for item in &items {
                match item {
                    PlayingItem::LibraryTrack { library_id, .. } => {
                        let item_library = node
                            .library_pool
                            .acquire_for_request(*library_id)
                            .await
                            .map_err(|error| {
                                anyhow::Error::new(ExpectedRequestError::Unavailable {
                                    what: "resolved library item",
                                })
                                .context(error)
                            })?;
                        if matches!(
                            &item_library.descriptor.protocol,
                            ProtocolAdapter::SubsonicTranslate { .. }
                        ) {
                            prepared_by_item.insert(
                                item.clone(),
                                (item.clone(), PlaybackSource::LibraryStream),
                            );
                        } else {
                            items_by_library
                                .entry(*library_id)
                                .or_default()
                                .push(item.clone());
                        }
                    }
                    _ => local_items.push(item.clone()),
                }
            }
            let dispatcher = PlayingItemActionDispatcher::new();
            for (library_id, library_tracks) in items_by_library {
                let item_library = node
                    .library_pool
                    .acquire_for_request(library_id)
                    .await
                    .map_err(|error| {
                        anyhow::Error::new(ExpectedRequestError::Unavailable {
                            what: "resolved library item",
                        })
                        .context(error)
                    })?;
                let handles = dispatcher
                    .get_file_handle(&item_library.fsio, &item_library.main_db, &library_tracks)
                    .await?;
                for (item, source) in files_to_playback_request(
                    &item_library.fsio,
                    &item_library.main_db,
                    &item_library.library_id.to_string(),
                    &item_library.media_root,
                    &handles,
                )
                .await?
                {
                    prepared_by_item.insert(item.clone(), (item, source));
                }
            }
            if !local_items.is_empty() {
                let handles = dispatcher
                    .get_file_handle(&library.fsio, &library.main_db, &local_items)
                    .await?;
                for (item, source) in files_to_playback_request(
                    &library.fsio,
                    &library.main_db,
                    &library.library_id.to_string(),
                    &library.media_root,
                    &handles,
                )
                .await?
                {
                    prepared_by_item.insert(item.clone(), (item, source));
                }
            }
            let prepared = items
                .iter()
                .filter_map(|item| prepared_by_item.get(item).cloned())
                .collect::<Vec<_>>();
            let position = request
                .position_seconds
                .map(Duration::try_from_secs_f64)
                .transpose()
                .map_err(|_| {
                    ExpectedRequestError::invalid_input(
                        "Invalid playback position: expected a finite, non-negative number of seconds",
                    )
                })?;
            node.session()?
                .edit_queue(request.expected_revision, |player| {
                    edit_playlist(
                        player,
                        &prepared,
                        PlaylistEditOptions {
                            mode: request.mode,
                            start_index: request.start_index.map(|index| index as usize),
                            playback_mode: request.playback_mode,
                            position,
                            play: request.play,
                        },
                    )
                })
                .await?;
        }
        Ok(Some(LoadResolvedItemsResponse {
            call_id: None,
            result: ResolvedItemsLoadResult::default(),
        }))
    }
}

impl ParamsExtractor for CastResolvedItemsRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}

impl Signal for CastResolvedItemsRequest {
    type Params = Arc<NodeContext>;
    type Response = CastResolvedItemsResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let (link, _) = node
            .connect_renderer(&request.fingerprint, RendererLinkUse::Cast)
            .await?;
        let result = link.call(&request.load).await;
        link.close();
        let response = result?;
        Ok(Some(CastResolvedItemsResponse {
            call_id: None,
            result: response.result,
        }))
    }
}

impl ParamsExtractor for CastPreflightRequest {
    type Params = (Arc<NodeContext>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.node.clone(),)
    }
}

impl Signal for CastPreflightRequest {
    type Params = (Arc<NodeContext>,);
    type Response = CastPreflightResponse;

    async fn handle(
        &self,
        (node,): Self::Params,
        _session: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let library_ids = parse_library_ids(&request.library_ids)?;
        let libraries = cast_library_states(&node, &library_ids)
            .await
            .into_iter()
            .map(|(library_id, state)| CastLibraryStatus {
                library_id: library_id.to_string(),
                state,
            })
            .collect();
        Ok(Some(CastPreflightResponse {
            call_id: None,
            libraries,
        }))
    }
}

async fn apply_cast_load(
    node: &NodeContext,
    request: &LoadResolvedItemsRequest,
) -> Result<ResolvedItemsLoadResult> {
    let (result, _) = apply_resolved_items_load(node, request, None).await?;
    Ok(result)
}

async fn apply_handoff_load(
    node: &NodeContext,
    block: PlayerHandoffBlockId,
    request: &LoadResolvedItemsRequest,
) -> Result<PlayingItemRef> {
    if request.mode != PlaylistOperateMode::PlayNext {
        return Err(ExpectedRequestError::invalid_input("Handoff load must use PlayNext").into());
    }
    let (_, handoff_item) = apply_resolved_items_load(node, request, Some(block)).await?;
    handoff_item.ok_or_else(|| {
        ExpectedRequestError::Unavailable {
            what: "handoff block item",
        }
        .into()
    })
}

async fn apply_resolved_items_load(
    node: &NodeContext,
    request: &LoadResolvedItemsRequest,
    block: Option<PlayerHandoffBlockId>,
) -> Result<(ResolvedItemsLoadResult, Option<PlayingItemRef>)> {
    let position = request
        .position_seconds
        .map(Duration::try_from_secs_f64)
        .transpose()
        .map_err(|_| {
            ExpectedRequestError::invalid_input(
                "Invalid cast position: expected a finite, non-negative number of seconds",
            )
        })?;
    let items = request
        .items
        .iter()
        .map(|item| {
            let PlayingItemRef::LibraryTrack { library_id, track } = item else {
                return Err(ExpectedRequestError::Unavailable {
                    what: "casting files outside a library",
                }
                .into());
            };
            Ok(LibraryItem {
                library_id: Uuid::parse_str(library_id).map_err(|error| {
                    anyhow::Error::new(ExpectedRequestError::invalid_input(format!(
                        "Invalid cast library id: {error}"
                    )))
                })?,
                file_uuid: Uuid::parse_str(track).map_err(|error| {
                    anyhow::Error::new(ExpectedRequestError::invalid_input(format!(
                        "Invalid cast file uuid: {error}"
                    )))
                })?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut library_ids = Vec::new();
    for item in &items {
        if !library_ids.contains(&item.library_id) {
            library_ids.push(item.library_id);
        }
    }
    for library_id in &library_ids {
        let state = cast_library_state(node, *library_id).await;
        if state != CastLibraryState::Ready {
            return Err(ExpectedRequestError::CastNotPossible {
                library_id: library_id.to_string(),
                state,
            }
            .into());
        }
    }

    let mut prepared_by_item = HashMap::new();
    for library_id in library_ids {
        let library = node
            .library_pool
            .acquire_for_request(library_id)
            .await
            .map_err(|error| {
                anyhow::Error::new(ExpectedRequestError::Unavailable {
                    what: "cast library",
                })
                .context(error)
            })?;
        let file_uuids = items
            .iter()
            .filter(|item| item.library_id == library_id)
            .map(|item| item.file_uuid)
            .collect::<Vec<_>>();
        let rows_by_uuid = files_by_uuid(&library.main_db, &file_uuids).await?;
        let mut handles = Vec::new();
        for item in items.iter().filter(|item| item.library_id == library_id) {
            let Some(row) = rows_by_uuid.get(&item.file_uuid) else {
                continue;
            };
            let mut handle = MediaFileHandle::from(row.clone());
            handle.item = PlayingItem::LibraryTrack {
                library_id: item.library_id,
                track: item.file_uuid.to_string(),
            };
            handles.push(handle);
        }
        for (item, source) in files_to_playback_request(
            &library.fsio,
            &library.main_db,
            &library_id.to_string(),
            &library.media_root,
            &handles,
        )
        .await?
        {
            if let PlayingItem::LibraryTrack { library_id, track } = item
                && let Ok(file_uuid) = Uuid::parse_str(&track)
            {
                let identity = LibraryItem {
                    library_id,
                    file_uuid,
                };
                prepared_by_item.insert(
                    identity,
                    (PlayingItem::LibraryTrack { library_id, track }, source),
                );
            }
        }
    }
    let prepared = items
        .iter()
        .filter_map(|item| prepared_by_item.get(item).cloned())
        .collect::<Vec<_>>();
    let requested_start_index = request
        .start_index
        .map(usize::try_from)
        .transpose()
        .map_err(|error| {
            ExpectedRequestError::invalid_input(format!("Invalid cast start index: {error}"))
        })?;
    let left_out = items
        .iter()
        .filter(|item| !prepared_by_item.contains_key(item))
        .copied()
        .collect::<Vec<_>>();
    if block.is_some() && !left_out.is_empty() {
        return Err(ExpectedError::NotFound { what: "cast file" }.into());
    }
    if !items.is_empty() && prepared_by_item.is_empty() {
        return Err(ExpectedRequestError::Unavailable {
            what: "no cast items are available on destination",
        }
        .into());
    }
    let start_policy = if block.is_some() {
        CastStartItemPolicy::Required
    } else {
        CastStartItemPolicy::NextThenPrevious
    };
    let start_index = remap_cast_start_index(
        &items,
        &prepared_by_item,
        requested_start_index,
        start_policy,
    )?;
    let result = ResolvedItemsLoadResult {
        left_out_items: left_out.into_iter().map(Into::into).collect(),
    };
    let handoff_item = match block {
        Some(_) => Some(
            start_index
                .and_then(|index| prepared.get(index))
                .and_then(|(item, _)| playing_item_to_ref(item))
                .ok_or(ExpectedRequestError::Unavailable {
                    what: "handoff block item",
                })?,
        ),
        None => None,
    };
    if let Some(block) = block {
        let position = position.unwrap_or_default();
        let play = request.play.unwrap_or(true);
        node.session()?
            .handoff_load_queue(HandoffLoadParams {
                expected_revision: request.expected_revision,
                tracks: prepared,
                block,
                start_index,
                position,
                play,
                playback_mode: request
                    .playback_mode
                    .filter(|mode| *mode != 99)
                    .map(Into::into),
            })
            .await?;
    } else {
        node.session()?
            .edit_queue(request.expected_revision, |player| {
                edit_playlist(
                    player,
                    &prepared,
                    PlaylistEditOptions {
                        mode: request.mode,
                        start_index,
                        playback_mode: request.playback_mode,
                        position,
                        play: request.play,
                    },
                )
            })
            .await?;
    }
    Ok((result, handoff_item))
}

fn playing_item_to_ref(item: &PlayingItem) -> Option<PlayingItemRef> {
    match item {
        PlayingItem::LibraryTrack { library_id, track } => Some(PlayingItemRef::LibraryTrack {
            library_id: library_id.to_string(),
            track: track.clone(),
        }),
        PlayingItem::IndependentFile(path) => {
            Some(PlayingItemRef::IndependentFile { path: path.clone() })
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
enum CastStartItemPolicy {
    Required,
    NextThenPrevious,
}

fn remap_cast_start_index(
    items: &[LibraryItem],
    prepared_by_item: &HashMap<LibraryItem, (PlayingItem, PlaybackSource)>,
    requested_start_index: Option<usize>,
    policy: CastStartItemPolicy,
) -> Result<Option<usize>> {
    if !items.is_empty() && prepared_by_item.is_empty() {
        return Err(ExpectedRequestError::Unavailable {
            what: "no cast items are available on destination",
        }
        .into());
    }
    let Some(index) = requested_start_index else {
        return Ok(None);
    };
    if index >= items.len() {
        return Err(
            ExpectedRequestError::invalid_input("Invalid cast start index: out of range").into(),
        );
    }
    let current_item = items[index];
    let selected_index = if prepared_by_item.contains_key(&current_item) {
        index
    } else {
        match policy {
            CastStartItemPolicy::Required => {
                return Err(ExpectedRequestError::Unavailable {
                    what: "current cast item is unavailable on destination",
                }
                .into());
            }
            CastStartItemPolicy::NextThenPrevious => (index + 1..items.len())
                .find(|candidate| prepared_by_item.contains_key(&items[*candidate]))
                .or_else(|| {
                    (0..index)
                        .rev()
                        .find(|candidate| prepared_by_item.contains_key(&items[*candidate]))
                })
                .ok_or(ExpectedRequestError::Unavailable {
                    what: "no cast items are available on destination",
                })?,
        }
    };
    Ok(Some(
        items[..selected_index]
            .iter()
            .filter(|item| prepared_by_item.contains_key(item))
            .count(),
    ))
}

impl ParamsExtractor for RefreshSessionRequest {
    type Params = (Arc<NodeContext>,);

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        (params.node.clone(),)
    }
}

impl Signal for RefreshSessionRequest {
    type Params = (Arc<NodeContext>,);
    type Response = ();

    async fn handle(
        &self,
        (node,): Self::Params,
        _session: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        node.session()?.reemit_to(&node.broadcaster).await?;
        Ok(Some(()))
    }
}

impl ParamsExtractor for SessionSnapshotRequest {
    type Params = (Arc<NodeContext>,);

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        (params.node.clone(),)
    }
}

impl Signal for SessionSnapshotRequest {
    type Params = (Arc<NodeContext>,);
    type Response = SessionSnapshotResponse;

    async fn handle(
        &self,
        (node,): Self::Params,
        _session: Option<Session>,
        _request: &Self,
    ) -> Result<Option<Self::Response>> {
        let session = node.session()?;
        Ok(Some(session_snapshot(&node, &session).await?))
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod cast_index_tests {
    use std::collections::HashMap;

    use playback::player::{LibraryItem, PlaybackSource, PlayingItem};
    use uuid::Uuid;

    use crate::{
        handlers::playback::{CastStartItemPolicy, remap_cast_start_index},
        messages::RequestErrorKind,
        request_error::classify,
    };

    #[test]
    fn dropped_items_before_the_selected_item_remap_the_index() {
        let items = [identity(), identity(), identity()];
        let prepared = HashMap::from([
            (
                items[0],
                (PlayingItem::from(items[0]), PlaybackSource::LibraryStream),
            ),
            (
                items[2],
                (PlayingItem::from(items[2]), PlaybackSource::LibraryStream),
            ),
        ]);

        assert_eq!(
            remap_cast_start_index(
                &items,
                &prepared,
                Some(2),
                CastStartItemPolicy::NextThenPrevious,
            )
            .unwrap(),
            Some(1)
        );
    }

    #[test]
    fn missing_selected_item_starts_at_the_next_then_previous_available_item() {
        let items = [identity(), identity(), identity()];
        let prepared = HashMap::from([(
            items[2],
            (PlayingItem::from(items[2]), PlaybackSource::LibraryStream),
        )]);

        assert_eq!(
            remap_cast_start_index(
                &items,
                &prepared,
                Some(1),
                CastStartItemPolicy::NextThenPrevious,
            )
            .unwrap(),
            Some(0)
        );

        let previous = HashMap::from([(
            items[0],
            (PlayingItem::from(items[0]), PlaybackSource::LibraryStream),
        )]);
        assert_eq!(
            remap_cast_start_index(
                &items,
                &previous,
                Some(1),
                CastStartItemPolicy::NextThenPrevious,
            )
            .unwrap(),
            Some(0)
        );
    }

    #[test]
    fn all_unplayable_items_reject_the_cast_load_without_a_start_index() {
        let items = [identity(), identity()];
        let prepared = HashMap::new();

        let error = remap_cast_start_index(
            &items,
            &prepared,
            None,
            CastStartItemPolicy::NextThenPrevious,
        )
        .unwrap_err();
        assert_eq!(classify(&error), RequestErrorKind::Unavailable);
    }

    fn identity() -> LibraryItem {
        LibraryItem {
            library_id: Uuid::new_v4(),
            file_uuid: Uuid::new_v4(),
        }
    }
}

impl ParamsExtractor for PlaylistRevisionRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}

impl Signal for PlaylistRevisionRequest {
    type Params = Arc<NodeContext>;
    type Response = PlaylistRevisionResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        _request: &Self,
    ) -> Result<Option<Self::Response>> {
        let revision = node.session()?.queue_revision().await;
        Ok(Some(PlaylistRevisionResponse {
            call_id: None,
            revision,
        }))
    }
}

impl ParamsExtractor for RemotePlaylistRevisionRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}

impl Signal for RemotePlaylistRevisionRequest {
    type Params = Arc<NodeContext>;
    type Response = RemotePlaylistRevisionResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let (link, _) = node
            .connect_renderer(&request.fingerprint, RendererLinkUse::Cast)
            .await?;
        let forwarded = PlaylistRevisionRequest { call_id: None };
        let result = link.call(&forwarded).await;
        link.close();
        Ok(Some(RemotePlaylistRevisionResponse {
            call_id: None,
            revision: result?.revision,
        }))
    }
}

async fn session_snapshot(
    node: &NodeContext,
    session: &PlayerSession,
) -> Result<SessionSnapshotResponse> {
    let (status, revision) = session.playback_snapshot().await;
    let registry = LibraryRegistry::load(Path::new(&*node.config_path))?;
    let subsonic_library_ids = registry
        .libraries
        .iter()
        .filter_map(|library| {
            matches!(library.protocol, ProtocolAdapter::SubsonicTranslate { .. })
                .then_some(library.record_id.0)
        })
        .collect();
    session_snapshot_from_status(status, revision, &subsonic_library_ids)
}

pub(crate) fn session_snapshot_from_status(
    status: PlayerStatus,
    revision: u64,
    subsonic_library_ids: &HashSet<Uuid>,
) -> Result<SessionSnapshotResponse> {
    let items = status
        .playlist
        .into_iter()
        .map(|entry| match entry.item {
            PlayingItem::LibraryTrack { library_id, track }
                if !subsonic_library_ids.contains(&library_id) =>
            {
                Ok(PlayingItemRef::LibraryTrack {
                    library_id: library_id.to_string(),
                    track,
                })
            }
            PlayingItem::LibraryTrack { .. } => {
                Err(anyhow::Error::new(ExpectedRequestError::Unavailable {
                    what: "handing off files outside a library",
                }))
            }
            _ => Err(anyhow::Error::new(ExpectedRequestError::Unavailable {
                what: "handing off files outside a library",
            })),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(SessionSnapshotResponse {
        call_id: None,
        items,
        index: status
            .index
            .map(u32::try_from)
            .transpose()
            .map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid session index: {error}"))
            })?,
        position_seconds: status.position.as_secs_f64(),
        playback_mode: status.playback_mode.into(),
        playing: status.state == PlaybackState::Playing,
        revision,
    })
}

impl ParamsExtractor for HandoffRequest {
    type Params = (Arc<NodeContext>,);

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        (params.node.clone(),)
    }
}

impl Signal for HandoffRequest {
    type Params = (Arc<NodeContext>,);
    type Response = HandoffResponse;

    async fn handle(
        &self,
        (node,): Self::Params,
        _session: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let generation = node.begin_handoff().await;
        let attached = node.renderer_link()?;
        let target = request.target.as_deref();
        let already_there = match (target, &attached) {
            (Some(target), Some(link)) => link.fingerprint() == target,
            (None, None) => true,
            _ => false,
        };
        if already_there {
            return Ok(Some(HandoffResponse { call_id: None }));
        }
        let paused = pause_handoff_source(&node, attached.clone(), generation).await?;
        if target.is_none()
            && let PausedSource::Renderer(source) = &paused.source
            && let Some(block) = node.take_handoff_block(source.fingerprint()).await
        {
            let take_back = source
                .call(&TakeBackHandoffBlockRequest {
                    block: block.into(),
                    call_id: None,
                })
                .await;
            match take_back {
                Ok(snapshot) => {
                    let resume_renderer = paused.was_playing
                        && snapshot.index.is_none()
                        && paused
                            .snapshot
                            .items
                            .iter()
                            .any(|item| !snapshot.items.contains(item));
                    let mut snapshot = snapshot;
                    let restore_snapshot = copy_session_snapshot(&snapshot);
                    snapshot.playing = paused.was_playing;
                    let load = handoff_load(snapshot, PlaylistOperateMode::Replace);
                    if let Err(error) = apply_cast_load(&node, &load).await {
                        let restore = handoff_load(restore_snapshot, PlaylistOperateMode::PlayNext);
                        source
                            .call(&HandoffLoadRequest {
                                block: block.into(),
                                load: restore,
                                call_id: None,
                            })
                            .await?;
                        node.remember_handoff_block(source.fingerprint().to_owned(), block)
                            .await;
                        resume_handoff_source(&node, paused).await?;
                        return Err(error);
                    }
                    if resume_renderer {
                        source.call(&PlayRequest { call_id: None }).await?;
                    }
                    node.detach_renderer().await?;
                    return Ok(Some(HandoffResponse { call_id: None }));
                }
                Err(error)
                    if error.chain().any(|cause| {
                        cause
                            .downcast_ref::<ExpectedRequestError>()
                            .is_some_and(|expected| {
                                matches!(
                                    expected,
                                    ExpectedRequestError::Unavailable { .. }
                                        | ExpectedRequestError::Remote {
                                            kind: RequestErrorKind::Unavailable,
                                            ..
                                        }
                                )
                            })
                    }) =>
                {
                    node.remember_handoff_block(source.fingerprint().to_owned(), block)
                        .await;
                    log::info!("Handoff block is unavailable; using the existing return path");
                }
                Err(error) => {
                    node.remember_handoff_block(source.fingerprint().to_owned(), block)
                        .await;
                    resume_handoff_source(&node, paused).await?;
                    return Err(error);
                }
            }
        }
        let load_mode = if target.is_none() && attached.is_some() {
            PlaylistOperateMode::Replace
        } else {
            PlaylistOperateMode::PlayNext
        };
        let result = async {
            let mut snapshot = copy_session_snapshot(&paused.snapshot);
            snapshot.playing = paused.was_playing;
            let expected_item = snapshot
                .index
                .and_then(|index| snapshot.items.get(index as usize))
                .cloned();
            let expected_position = snapshot.position_seconds;
            let load = handoff_load(snapshot, load_mode);
            match target {
                Some(target) => {
                    let block = PlayerHandoffBlockId(Uuid::new_v4());
                    expected_item.context("handoff current item")?;
                    hand_off_to_renderer(&node, target, block, load, expected_position).await?;
                }
                None => {
                    let expected_item = expected_item.context("handoff current item")?;
                    hand_off_to_this_device(
                        &node,
                        attached.clone(),
                        &load,
                        &expected_item,
                        expected_position,
                    )
                    .await?
                }
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            if let Some(target) = target
                && node
                    .renderer_link()?
                    .is_some_and(|link| link.fingerprint() == target)
            {
                if let Some(source) = &attached {
                    node.attach_renderer(source.fingerprint()).await?;
                } else {
                    node.detach_renderer().await?;
                }
            }
            resume_handoff_source(&node, paused).await?;
            return Err(error);
        }
        Ok(Some(HandoffResponse { call_id: None }))
    }
}

struct PausedHandoffSource {
    snapshot: SessionSnapshotResponse,
    source: PausedSource,
    was_playing: bool,
}

enum PausedSource {
    Renderer(Arc<RendererLink>),
    Local(Arc<PlayerSession>),
}

async fn pause_handoff_source(
    node: &Arc<NodeContext>,
    attached: Option<Arc<RendererLink>>,
    generation: u64,
) -> Result<PausedHandoffSource> {
    if let Some(source) = attached {
        let before = source
            .call(&SessionSnapshotRequest { call_id: None })
            .await?;
        let paused = match node
            .pause_handoff_source(source.fingerprint(), &source, generation)
            .await
        {
            Ok(paused) => paused,
            Err(error) => {
                if before.playing {
                    source.call(&PlayRequest { call_id: None }).await?;
                }
                return Err(error);
            }
        };
        if !paused {
            return Err(ExpectedRequestError::HandoffSuperseded.into());
        }
        let snapshot = source.call(&SessionSnapshotRequest { call_id: None }).await;
        let snapshot = match snapshot {
            Ok(snapshot) => snapshot,
            Err(error) => {
                if before.playing {
                    source.call(&PlayRequest { call_id: None }).await?;
                }
                return Err(error);
            }
        };
        if snapshot.playing {
            if before.playing {
                source.call(&PlayRequest { call_id: None }).await?;
            }
            // Something resumed the source while it was being handed off.
            return Err(ExpectedRequestError::HandoffSuperseded.into());
        }
        Ok(PausedHandoffSource {
            snapshot,
            source: PausedSource::Renderer(source),
            was_playing: before.playing,
        })
    } else {
        let session = node.session()?;
        let was_playing = session.player.lock().await.get_status().state == PlaybackState::Playing;
        session.player.lock().await.pause_applied().await?;
        let snapshot = match session_snapshot(node, &session).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                if was_playing {
                    session.player.lock().await.play_applied().await?;
                }
                return Err(error);
            }
        };
        if snapshot.playing {
            if was_playing {
                session.player.lock().await.play_applied().await?;
            }
            // Something resumed the source while it was being handed off.
            return Err(ExpectedRequestError::HandoffSuperseded.into());
        }
        Ok(PausedHandoffSource {
            snapshot,
            source: PausedSource::Local(session),
            was_playing,
        })
    }
}

async fn resume_handoff_source(node: &Arc<NodeContext>, paused: PausedHandoffSource) -> Result<()> {
    if !paused.was_playing {
        return Ok(());
    }
    match paused.source {
        PausedSource::Renderer(source) if source.is_closed() => {
            let link = node
                .renderer_link()?
                .filter(|link| link.fingerprint() == source.fingerprint())
                .context("source renderer was not restored after failed handoff")?;
            link.call(&PlayRequest { call_id: None }).await?;
        }
        PausedSource::Renderer(source) => {
            source.call(&PlayRequest { call_id: None }).await?;
        }
        PausedSource::Local(session) => {
            session.player.lock().await.play_applied().await?;
        }
    }
    Ok(())
}

fn copy_session_snapshot(snapshot: &SessionSnapshotResponse) -> SessionSnapshotResponse {
    SessionSnapshotResponse {
        call_id: snapshot.call_id,
        items: snapshot.items.clone(),
        index: snapshot.index,
        position_seconds: snapshot.position_seconds,
        playback_mode: snapshot.playback_mode,
        playing: snapshot.playing,
        revision: snapshot.revision,
    }
}

/// Step 2: the load that recreates the snapshot on the target.
fn handoff_load(
    snapshot: SessionSnapshotResponse,
    mode: PlaylistOperateMode,
) -> LoadResolvedItemsRequest {
    LoadResolvedItemsRequest {
        call_id: None,
        items: snapshot.items.into_iter().collect(),
        start_index: snapshot.index,
        mode,
        expected_revision: None,
        playback_mode: Some(snapshot.playback_mode),
        position_seconds: Some(snapshot.position_seconds),
        play: Some(snapshot.playing),
    }
}

/// Load the paused snapshot onto a renderer target.
async fn hand_off_to_renderer(
    node: &Arc<NodeContext>,
    target: &str,
    block: PlayerHandoffBlockId,
    load: LoadResolvedItemsRequest,
    expected_position: f64,
) -> Result<()> {
    let (connected_link, alias) = node
        .connect_renderer(target, RendererLinkUse::Control)
        .await?;
    let mut events = node.subscribe_handoff_playback_events();
    let target_before = connected_link
        .call(&SessionSnapshotRequest { call_id: None })
        .await?;
    let handoff = HandoffLoadRequest {
        block: block.into(),
        load,
        call_id: None,
    };
    let response = match connected_link.call(&handoff).await {
        Ok(response) => response,
        Err(error) => {
            connected_link.close();
            return Err(error);
        }
    };
    let target_item = response.handoff_item;
    node.install_renderer_link(connected_link, alias)?;
    let link = node
        .renderer_link()?
        .context("renderer link disappeared after handoff load")?;
    node.remember_handoff_block(target.to_owned(), block).await;
    node.publish_capabilities()?;
    if let Some(link) = node.renderer_link()?
        && let Err(error) = link.call(&RefreshSessionRequest { call_id: None }).await
    {
        warn!("Handoff succeeded, but renderer state refresh failed: {error:#}");
    }
    if let Err(error) = wait_for_handoff_start(&mut events, &target_item, expected_position).await {
        let rollback = link
            .call(&TakeBackHandoffBlockRequest {
                block: block.into(),
                call_id: None,
            })
            .await;
        match rollback {
            Err(rollback_error) => {
                warn!("Could not roll back failed handoff block: {rollback_error:#}");
                return Err(error.context(format!("handoff rollback failed: {rollback_error:#}")));
            }
            Ok(snapshot) if snapshot.items != target_before.items => {
                return Err(error.context("handoff rollback retained the unavailable item"));
            }
            Ok(_) => {
                node.take_handoff_block(link.fingerprint()).await;
            }
        }
        return Err(error);
    }
    Ok(())
}

/// Load the paused snapshot locally, then detach from its renderer source.
async fn hand_off_to_this_device(
    node: &Arc<NodeContext>,
    attached: Option<Arc<RendererLink>>,
    load: &LoadResolvedItemsRequest,
    expected_item: &PlayingItemRef,
    expected_position: f64,
) -> Result<()> {
    let session = node.session()?;
    let original = session_snapshot(node, &session).await?;
    let mut events = node.subscribe_handoff_playback_events();
    apply_cast_load(node, load).await?;
    if let Err(error) = wait_for_handoff_start(&mut events, expected_item, expected_position).await
    {
        if original.items.is_empty() {
            if let Err(restore_error) = node
                .session()?
                .edit_queue(None, |player| {
                    player.clear_playlist();
                    true
                })
                .await
            {
                warn!("Could not clear local queue after failed handoff: {restore_error:#}");
            }
        } else {
            let restore = handoff_load(original, PlaylistOperateMode::Replace);
            if let Err(restore_error) = apply_cast_load(node, &restore).await {
                warn!("Could not restore local queue after failed handoff: {restore_error:#}");
            }
        }
        return Err(error);
    }
    if attached.is_some() {
        node.detach_renderer().await?;
    } else {
        node.session()?.clear_handoff_blocks().await?;
    }
    Ok(())
}

async fn wait_for_handoff_start(
    events: &mut broadcast::Receiver<HandoffPlaybackEvent>,
    expected_item: &PlayingItemRef,
    expected_position: f64,
) -> Result<()> {
    tokio::time::timeout(HANDOFF_START_TIMEOUT, async {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => {
                    return Err(ExpectedRequestError::Unavailable {
                        what: "handoff playback status",
                    }
                    .into());
                }
            };
            match event {
                HandoffPlaybackEvent::Playlist(items) => {
                    if items.iter().any(|item| {
                        same_handoff_item(&item.item, expected_item) && item.unavailable.is_some()
                    }) {
                        return Err(ExpectedRequestError::Unavailable {
                            what: "handed-off track",
                        }
                        .into());
                    }
                }
                HandoffPlaybackEvent::Status(status) => {
                    if status
                        .item_request
                        .as_ref()
                        .is_some_and(|item| same_handoff_item(item, expected_item))
                        && status.state.eq_ignore_ascii_case("playing")
                        && status.ready
                        && (f64::from(status.progress_seconds) - expected_position).abs() <= 1.5
                    {
                        return Ok(());
                    }
                }
            }
        }
    })
    .await
    .map_err(|_| {
        anyhow::Error::new(ExpectedRequestError::Unavailable {
            what: "handoff playback start",
        })
        .context(format!(
            "expected item {expected_item:?} at {expected_position}s"
        ))
    })?
}

fn same_handoff_item(left: &PlayingItemRef, right: &PlayingItemRef) -> bool {
    match (left, right) {
        (
            PlayingItemRef::LibraryTrack { track: left, .. },
            PlayingItemRef::LibraryTrack { track: right, .. },
        ) => left == right,
        _ => left == right,
    }
}

#[cfg(test)]
mod handoff_confirmation_tests {
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    async fn slow_handoff_playback_confirmation_completes_when_status_arrives() -> Result<()> {
        let expected_item = PlayingItemRef::IndependentFile {
            path: "slow-start.ogg".into(),
        };
        let (sender, mut events) = broadcast::channel(1);
        let delayed_sender = sender.clone();
        let delayed_item = expected_item.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = delayed_sender.send(HandoffPlaybackEvent::Status(Box::new(PlaybackStatus {
                session_id: "slow-start".into(),
                state: "Playing".into(),
                progress_seconds: 42.0,
                progress_percentage: 0.0,
                artist: None,
                album: None,
                title: None,
                duration: 0.0,
                index: Some(0),
                item: None,
                item_request: Some(delayed_item),
                playback_mode: 0,
                ready: true,
                cover_art_path: None,
                lib_path: String::new(),
            })));
        });

        let started = Instant::now();
        wait_for_handoff_start(&mut events, &expected_item, 42.0).await?;
        assert!(started.elapsed() >= Duration::from_millis(100));
        Ok(())
    }
}
