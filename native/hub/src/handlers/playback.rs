use std::{
    collections::HashMap,
    slice,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use log::warn;
use tokio::sync::Mutex;
use uuid::Uuid;

use ::database::{
    actions::{
        file::{file_ids_by_uuid, file_uuids_by_id, files_by_uuid},
        mixes::query_mix_media_files,
        stats::increase_skipped,
    },
    error::ExpectedError,
    playing_item::{MediaFileHandle, dispatcher::PlayingItemActionDispatcher},
};
use ::playback::{
    player::{LibraryItem, Playable, PlaybackSource, PlaybackState, PlayerStatus, PlayingItem},
    strategies::AddMode,
};

use crate::backends::remote::{RendererLink, RendererLinkUse};
use crate::request_error::ExpectedRequestError;
use crate::server::libraries::{cast_library_state, cast_library_states, parse_library_ids};
use crate::{
    Session, Signal,
    library_pool::{LibraryHandle, LibraryPool},
    messages::*,
    node::NodeContext,
    session::PlayerSession,
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

pub(crate) fn reject_remote_independent_item(
    session: Option<&Session>,
    item: &PlayingItemRequest,
) -> Result<()> {
    if session.is_some() && item.independent_file.is_some() {
        return Err(ExpectedError::InvalidInput {
            detail: "independent files are only available on this device".into(),
        }
        .into());
    }
    Ok(())
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
                            library_id: library.library_id,
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
    let queue_changed = mode == PlaylistOperateMode::Replace && !player.get_playlist().is_empty();
    if mode == PlaylistOperateMode::Replace {
        player.clear_playlist();
    }
    let offset = match mode {
        PlaylistOperateMode::Replace => 0,
        PlaylistOperateMode::AppendToEnd => player.get_playlist().len(),
        PlaylistOperateMode::PlayNext => player
            .get_status()
            .index
            .map_or(0, |index| index.saturating_add(1)),
    };
    let add_mode = if mode == PlaylistOperateMode::PlayNext {
        AddMode::PlayNext
    } else {
        AddMode::AppendToEnd
    };
    if !prepared.is_empty() {
        player.add_to_playlist_with_sources(prepared.to_vec(), add_mode);
    }
    if let Some(playback_mode) = playback_mode
        && playback_mode != 99
    {
        player.set_playback_mode(playback_mode.into());
    }
    if let Some(start_index) = start_index.filter(|_| !prepared.is_empty()) {
        let index = start_index + offset;
        if position.is_some() || play.is_some() {
            player.load_at_position(
                index,
                position.unwrap_or_default(),
                play.unwrap_or(playback_mode.is_some()),
            );
        } else {
            player.switch(index);
            player.play();
        }
    }
    queue_changed || !prepared.is_empty()
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

struct ResolvedMixQuery {
    playing_items: Vec<PlayingItemRequest>,
    start_index: Option<usize>,
}

async fn resolve_mix_query(
    request: &ResolveMixQueryRequest,
    library: &LibraryHandle,
    node: &NodeContext,
) -> Result<ResolvedMixQuery> {
    let fallback_items =
        playing_items_from_requests(&request.fallback_playing_items, Some(library)).await?;

    let tracks = if request.queries.is_empty() {
        PlayingItemActionDispatcher::new()
            .get_file_handle(&library.fsio, &library.main_db, &fallback_items)
            .await?
    } else {
        let queue = if request.exclude_local_queue && node.renderer_link()?.is_none() {
            node.session_queue_for_library(library.library_id).await?
        } else {
            Vec::new()
        };
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
    let playing_items = playing_item_requests(&items, &node.library_pool, None).await;

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
    record_skip_item(node, item).await
}

async fn record_skip_item(node: &NodeContext, item: Option<PlayingItem>) -> Result<()> {
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
        (node, _player): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let index = dart_signal.index.try_into().map_err(|error| {
            ExpectedRequestError::invalid_input(format!("Invalid switch index: {error}"))
        })?;
        let mut previous_item = None;
        node.session()?
            .edit_queue(dart_signal.expected_revision, |player| {
                previous_item = player.get_status().item;
                player.switch(index);
                false
            })
            .await?;
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
        player.lock().await.seek(dart_signal.position_seconds);
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
        player.lock().await.set_volume(volume);
        Ok(Some(VolumeResponse { volume }))
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
        player.lock().await.set_mix_with_others(dart_signal.enabled);
        Ok(Some(()))
    }
}

impl ParamsExtractor for ResolveMixQueryRequest {
    type Params = (Option<LibraryHandle>, Arc<NodeContext>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.library.clone(), all_params.node.clone())
    }
}

impl Signal for ResolveMixQueryRequest {
    type Params = (Option<LibraryHandle>, Arc<NodeContext>);
    type Response = ResolveMixQueryResponse;

    async fn handle(
        &self,
        (library, node): Self::Params,
        session: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        if let Some(item) = &request.initial_playback_item {
            reject_remote_independent_item(session.as_ref(), item)?;
        }
        for item in &request.fallback_playing_items {
            reject_remote_independent_item(session.as_ref(), item)?;
        }
        let library = require_library(library)?;
        let resolved = resolve_mix_query(request, &library, &node).await?;
        let start_index = resolved
            .start_index
            .map(u32::try_from)
            .transpose()
            .map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid start index: {error}"))
            })?;
        Ok(Some(ResolveMixQueryResponse {
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
            let items = request
                .items
                .iter()
                .map(|item| {
                    let library_item =
                        item.library_item
                            .clone()
                            .ok_or(ExpectedRequestError::Unavailable {
                                what: "casting files outside a library",
                            })?;
                    Ok(library_item)
                })
                .collect::<Result<Vec<_>>>()?;
            apply_cast_load(
                &node,
                &CastLoadRequest {
                    items,
                    start_index: request.start_index,
                    mode: request.mode,
                    expected_revision: request.expected_revision,
                    playback_mode: request.playback_mode,
                    position_seconds: None,
                    play: None,
                },
            )
            .await?;
        } else {
            let library = require_library(library)?;
            let items = playing_items_from_requests(&request.items, Some(&library)).await?;
            let mut items_by_library: HashMap<Uuid, Vec<PlayingItem>> = HashMap::new();
            let mut local_items = Vec::new();
            for item in &items {
                match item {
                    PlayingItem::InLibrary(identity) => items_by_library
                        .entry(identity.library_id)
                        .or_default()
                        .push(item.clone()),
                    _ => local_items.push(item.clone()),
                }
            }
            let dispatcher = PlayingItemActionDispatcher::new();
            let mut prepared_by_item = HashMap::new();
            for (library_id, library_items) in items_by_library {
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
                    .get_file_handle(&item_library.fsio, &item_library.main_db, &library_items)
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
            node.session()?
                .edit_queue(request.expected_revision, |player| {
                    edit_playlist(
                        player,
                        &prepared,
                        PlaylistEditOptions {
                            mode: request.mode,
                            start_index: request.start_index.map(|index| index as usize),
                            playback_mode: request.playback_mode,
                            position: None,
                            play: None,
                        },
                    )
                })
                .await?;
        }
        Ok(Some(LoadResolvedItemsResponse {}))
    }
}

impl ParamsExtractor for CastResolvedItemsRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
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
        let items = request
            .items
            .iter()
            .map(|item| {
                item.library_item.clone().ok_or_else(|| {
                    anyhow::Error::new(ExpectedRequestError::Unavailable {
                        what: "casting files outside a library",
                    })
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let (link, _) = node
            .connect_renderer(&request.fingerprint, RendererLinkUse::Cast)
            .await?;
        let result = link
            .call(&CastLoadRequest {
                items,
                start_index: request.start_index,
                mode: request.mode,
                expected_revision: request.expected_revision,
                playback_mode: request.playback_mode,
                position_seconds: None,
                play: None,
            })
            .await;
        link.close();
        result?;
        Ok(Some(CastResolvedItemsResponse {}))
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
        Ok(Some(CastPreflightResponse { libraries }))
    }
}

impl ParamsExtractor for CastLoadRequest {
    type Params = (Arc<NodeContext>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.node.clone(),)
    }
}

impl Signal for CastLoadRequest {
    type Params = (Arc<NodeContext>,);
    type Response = CastLoadResponse;

    async fn handle(
        &self,
        (node,): Self::Params,
        _session: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        apply_cast_load(&node, request).await?;
        Ok(Some(CastLoadResponse {}))
    }
}

async fn apply_cast_load(node: &NodeContext, request: &CastLoadRequest) -> Result<()> {
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
            Ok(LibraryItem {
                library_id: Uuid::parse_str(&item.library_id).map_err(|error| {
                    anyhow::Error::new(ExpectedRequestError::invalid_input(format!(
                        "Invalid cast library id: {error}"
                    )))
                })?,
                file_uuid: Uuid::parse_str(&item.file_uuid).map_err(|error| {
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
            let row = rows_by_uuid
                .get(&item.file_uuid)
                .ok_or(ExpectedError::NotFound { what: "cast file" })?;
            let mut handle = MediaFileHandle::from(row.clone());
            handle.item = PlayingItem::InLibrary(*item);
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
            if let PlayingItem::InLibrary(identity) = item {
                prepared_by_item.insert(identity, (PlayingItem::InLibrary(identity), source));
            }
        }
    }
    let prepared = items
        .iter()
        .filter_map(|item| prepared_by_item.get(item).cloned())
        .collect::<Vec<_>>();
    let start_index = request
        .start_index
        .map(usize::try_from)
        .transpose()
        .map_err(|error| {
            ExpectedRequestError::invalid_input(format!("Invalid cast start index: {error}"))
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
                    position,
                    play: request.play,
                },
            )
        })
        .await?;
    Ok(())
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
        session: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        let broadcaster = if session.is_some() {
            &node.broadcaster
        } else {
            &node.gui_broadcaster
        };
        node.session()?.reemit_to(broadcaster).await?;
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
        Ok(Some(session_snapshot(&session).await?))
    }
}

async fn session_snapshot(session: &PlayerSession) -> Result<SessionSnapshotResponse> {
    let (status, revision) = session.playback_snapshot().await;
    session_snapshot_from_status(status, revision)
}

pub(crate) fn session_snapshot_from_status(
    status: PlayerStatus,
    revision: u64,
) -> Result<SessionSnapshotResponse> {
    let items = status
        .playlist
        .into_iter()
        .map(|item| match item {
            PlayingItem::InLibrary(item) => Ok(LibraryItemRequest::from(item)),
            _ => Err(anyhow::Error::new(ExpectedRequestError::Unavailable {
                what: "handing off files outside a library",
            })),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(SessionSnapshotResponse {
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
            return Ok(Some(HandoffResponse {}));
        }
        let load = handoff_load(snapshot_handoff_source(&node, attached.as_deref()).await?);
        match target {
            Some(target) => {
                hand_off_to_renderer(&node, attached.clone(), target, load, generation).await?
            }
            None => hand_off_to_this_device(&node, attached, &load, generation).await?,
        }
        Ok(Some(HandoffResponse {}))
    }
}

/// Step 1: what this device controls now, from the attached renderer or the local session.
async fn snapshot_handoff_source(
    node: &NodeContext,
    attached: Option<&RendererLink>,
) -> Result<SessionSnapshotResponse> {
    let Some(link) = attached else {
        let session = node.session()?;
        return session_snapshot(&session).await;
    };
    link.call(&SessionSnapshotRequest {}).await
}

/// Step 2: the load that recreates the snapshot on the target.
fn handoff_load(snapshot: SessionSnapshotResponse) -> CastLoadRequest {
    CastLoadRequest {
        items: snapshot.items,
        start_index: snapshot.index,
        mode: PlaylistOperateMode::Replace,
        expected_revision: None,
        playback_mode: Some(snapshot.playback_mode),
        position_seconds: Some(snapshot.position_seconds),
        play: Some(snapshot.playing),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandoffOutcome {
    SourcePaused,
    PausePending,
    ResumeWon,
}

/// Steps 2 and 3 for a renderer target: load it over a new link, then pause the source.
async fn hand_off_to_renderer(
    node: &Arc<NodeContext>,
    attached: Option<Arc<RendererLink>>,
    target: &str,
    load: CastLoadRequest,
    generation: u64,
) -> Result<()> {
    let (link, alias) = node
        .connect_renderer(target, RendererLinkUse::Control)
        .await?;
    if let Err(error) = link.call(&load).await {
        link.close();
        return Err(error);
    }
    let outcome = if let Some(source) = attached {
        match node
            .pause_handoff_source(source.fingerprint(), &source, generation)
            .await
        {
            Ok(true) => HandoffOutcome::SourcePaused,
            Ok(false) => HandoffOutcome::ResumeWon,
            Err(error) => {
                log::warn!("Handoff destination started; source pause pending: {error:#}");
                schedule_handoff_pause_retry(node, source, generation).await;
                HandoffOutcome::PausePending
            }
        }
    } else {
        node.session()?.player.lock().await.pause();
        HandoffOutcome::SourcePaused
    };
    node.install_renderer_link(link, alias)?;
    node.publish_capabilities()?;
    log::info!("Handoff outcome: {outcome:?}");
    Ok(())
}

/// Steps 2 to 4 for this device as the target: load locally, pause the renderer that was
/// playing, then detach from it.
async fn hand_off_to_this_device(
    node: &Arc<NodeContext>,
    attached: Option<Arc<RendererLink>>,
    load: &CastLoadRequest,
    generation: u64,
) -> Result<()> {
    apply_cast_load(node, load).await?;
    if let Some(source) = attached {
        let outcome = match node
            .pause_handoff_source(source.fingerprint(), &source, generation)
            .await
        {
            Ok(true) => HandoffOutcome::SourcePaused,
            Ok(false) => HandoffOutcome::ResumeWon,
            Err(error) => {
                log::warn!("Handoff destination started; source pause pending: {error:#}");
                schedule_handoff_pause_retry(node, source.clone(), generation).await;
                HandoffOutcome::PausePending
            }
        };
        node.detach_renderer().await?;
        log::info!("Handoff outcome: {outcome:?}");
    }
    Ok(())
}

async fn schedule_handoff_pause_retry(
    node: &Arc<NodeContext>,
    source: Arc<RendererLink>,
    generation: u64,
) {
    let weak_node = Arc::downgrade(node);
    let cancel = node.handoff_retry_cancellation_token();
    let fingerprint = source.fingerprint().to_owned();
    let task = tokio::spawn(async move {
        let started = Instant::now();
        let mut delay = Duration::from_secs(1);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(delay) => {}
            }
            let Some(node) = weak_node.upgrade() else {
                return;
            };
            if node.handoff_generation() != generation
                || started.elapsed() >= Duration::from_secs(120)
            {
                return;
            }
            match node
                .pause_handoff_source(&fingerprint, &source, generation)
                .await
            {
                Ok(true) | Ok(false) => return,
                Err(error) => log::warn!("Retrying handoff pause for {fingerprint}: {error:#}"),
            }
            delay = match delay.as_secs() {
                1 => Duration::from_secs(2),
                2 => Duration::from_secs(4),
                4 => Duration::from_secs(8),
                _ => Duration::from_secs(10),
            };
        }
    });
    node.register_handoff_retry(task).await;
}
