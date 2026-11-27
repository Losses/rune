use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use discovery::access_policy::PeerIdentity;
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
    library_registry::{LibraryRecordId, LibraryRegistry, ServiceKind},
    playing_item::{MediaFileHandle, dispatcher::PlayingItemActionDispatcher},
};
use ::playback::PlayerCommand;
use ::playback::player::{
    HandoffBlockId as PlayerHandoffBlockId, LibraryItem, PausedPlayback, Playable, PlaybackSource,
    PlaybackState as PlayerPlaybackState, PlayerCommandError, PlayerStatus, PlayingItem,
    PlaylistEdit, PlaylistLoad, PlaylistPlacement, RemoteAudioError,
};

use crate::backends::remote::{RendererLink, RendererLinkUse};
use crate::macros::RequestSlots;
use crate::node::{HandoffPause, HandoffPlaybackEvent, HandoffPlaybackReport, HandoffRun};
use crate::request_error::{ExpectedRequestError, player_command_error};
use crate::request_owner::{Implementation, refuse_libraries_kept_here};
use crate::rpc::NamedLibraries;
use crate::server::libraries::{cast_library_state, cast_library_states, parse_library_ids};
use crate::service::{ServiceBinding, unbound_item_slots};
use crate::{
    Session, Signal,
    library_pool::{LibraryHandle, LibraryPool},
    messages::*,
    node::NodeContext,
    session::{HandoffLoadParams, PlayerSession},
    utils::{
        GlobalParams, ParamsExtractor, RESOLVED_TRACK_LIMIT, files_to_playback_request,
        held_service_binding, require_library, resolved_start, service_binding_for_library,
    },
};
use anyhow::Error;
use futures::future;
use std::slice;
use tokio::time as tokio_time;

async fn apply_player_command(
    player: &Arc<Mutex<dyn Playable>>,
    command: PlayerCommand,
) -> Result<()> {
    let completion = { player.lock().await.command_with_completion(command) };
    completion
        .await
        .map_err(|error| Error::new(player_command_error(error)))?;
    Ok(())
}

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

/// The streaming service of the library that owns `item`, from the binding of the open service
/// library when the item is its own, else from its registry record alone, so nothing is built or
/// contacted. None for a local item. `open_library` is the local library the request came with,
/// which holds its own items.
pub(crate) fn item_service(
    node: &NodeContext,
    open_library: Option<&LibraryHandle>,
    item: &PlayingItemRef,
) -> Result<Option<ServiceKind>> {
    let PlayingItemRef::LibraryTrack { library_id, .. } = item else {
        return Ok(None);
    };
    let library_id = Uuid::parse_str(library_id).map_err(|error| {
        ExpectedRequestError::invalid_input(format!("Invalid library id: {error}"))
    })?;
    if open_library.is_some_and(|library| library.library_id == library_id) {
        return Ok(None);
    }
    if let Some(binding) = node.service_binding(library_id)? {
        return Ok(Some(binding.service()));
    }
    let registry = LibraryRegistry::load(Path::new(&*node.config_path))?;
    Ok(registry
        .record(LibraryRecordId(library_id))
        .ok()
        .and_then(|record| record.protocol.service()))
}

/// What the streaming service library that owns `item` answers about its items, if one does:
/// as the binding this device holds for it declares, or else as its service declares for it with
/// what this device keeps of the user's choices for it. `open_library` is the local library the
/// request came with, which answers for its own items.
pub(crate) fn item_service_slots(
    node: &NodeContext,
    open_library: Option<&LibraryHandle>,
    item: &PlayingItemRef,
) -> Result<Option<RequestSlots>> {
    let PlayingItemRef::LibraryTrack { library_id, .. } = item else {
        return Ok(None);
    };
    let library_id = Uuid::parse_str(library_id).map_err(|error| {
        ExpectedRequestError::invalid_input(format!("Invalid library id: {error}"))
    })?;
    if open_library.is_some_and(|library| library.library_id == library_id) {
        return Ok(None);
    }
    if let Some(binding) = held_service_binding(node, library_id)? {
        return Ok(Some(Implementation::Service(binding).item_slots()));
    }
    let registry = LibraryRegistry::load(Path::new(&*node.config_path))?;
    Ok(registry
        .record(LibraryRecordId(library_id))
        .ok()
        .and_then(|record| record.protocol.service())
        .map(|service| unbound_item_slots(Path::new(&*node.config_path), service, library_id)))
}

/// The binding of the streaming service library that owns `item`, if one does. The open
/// library and the binding of the open service library answer first; the registry is read only
/// when neither holds the item's library, and `open_library` is the local library the request
/// came with, which holds its own items.
pub(crate) async fn item_service_binding(
    node: &Arc<NodeContext>,
    open_library: Option<&LibraryHandle>,
    item: &PlayingItemRef,
) -> Result<Option<Arc<dyn ServiceBinding>>> {
    let PlayingItemRef::LibraryTrack { library_id, .. } = item else {
        return Ok(None);
    };
    let library_id = Uuid::parse_str(library_id).map_err(|error| {
        ExpectedRequestError::invalid_input(format!("Invalid library id: {error}"))
    })?;
    if open_library.is_some_and(|library| library.library_id == library_id) {
        return Ok(None);
    }
    if let Some(binding) = node.service_binding(library_id)? {
        return Ok(Some(binding));
    }
    let registry = LibraryRegistry::load(Path::new(&*node.config_path))?;
    let is_service = registry
        .record(LibraryRecordId(library_id))
        .is_ok_and(|record| record.protocol.service().is_some());
    if !is_service {
        return Ok(None);
    }
    service_binding_for_library(node, library_id)
        .await
        .map(Some)
        .map_err(|error| item_library_failure(error, library_id))
}

/// A failure to make the binding of an item's service library, as the request about the item
/// fails with: a library whose service needs a new sign-in, or one out of reach. The player
/// reads the same failure for how its tracks are unavailable; a request must not fail as
/// internal for it.
fn item_library_failure(error: Error, library_id: Uuid) -> Error {
    let typed = match error
        .chain()
        .find_map(|cause| cause.downcast_ref::<RemoteAudioError>())
    {
        Some(RemoteAudioError::CredentialsRejected) => {
            ExpectedRequestError::ServiceSignInRequired {
                record_id: library_id.to_string(),
            }
        }
        Some(_) => ExpectedRequestError::Unavailable {
            what: "streaming library",
        },
        None => return error,
    };
    Error::new(typed).context(format!("{error:#}"))
}

/// What the handlers that route by item take: the open local library, the node and the pool.
pub(crate) type ItemRoutingParams = (Option<LibraryHandle>, Arc<NodeContext>, Arc<LibraryPool>);

/// The library a local item is read from: the library the item names, which is the open one
/// when the item is its own. A file played by its path is read through the open library.
pub(crate) async fn local_item_library(
    open_library: Option<LibraryHandle>,
    pool: &Arc<LibraryPool>,
    item: &PlayingItemRef,
) -> Result<LibraryHandle> {
    let PlayingItemRef::LibraryTrack { library_id, .. } = item else {
        return require_library(open_library);
    };
    let library_id = Uuid::parse_str(library_id).map_err(|error| {
        ExpectedRequestError::invalid_input(format!("Invalid library id: {error}"))
    })?;
    match open_library {
        Some(library) if library.library_id == library_id => Ok(library),
        _ => pool.acquire_for_request(library_id).await,
    }
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
        .collect::<future::JoinAll<_>>()
        .await
        .into_iter()
        .collect()
}

async fn resolve_open_service_tracks(
    requests: &[PlayingItemRef],
    _binding: &dyn ServiceBinding,
) -> Result<Vec<PlayingItem>> {
    let mut resolved = Vec::with_capacity(requests.len());
    for request in requests {
        resolved.extend(playing_items_from_requests(slice::from_ref(request), None).await?);
    }
    Ok(resolved)
}

struct PlaylistEditOptions {
    mode: PlaylistOperateMode,
    start_index: Option<usize>,
    playback_mode: Option<PlaybackMode>,
    position: Option<Duration>,
    play: LoadPlay,
}

/// The prepared queue position of the occurrence `requested_start_index` names in
/// `items`. Preparation leaves out items that are no longer playable, so the position is
/// the number of prepared occurrences before the selected one. A selected occurrence that
/// did not prepare fails instead of starting another surviving track.
fn remap_local_start_index(
    items: &[PlayingItem],
    prepared_by_item: &HashMap<PlayingItem, (PlayingItem, PlaybackSource)>,
    requested_start_index: Option<usize>,
) -> Result<Option<usize>> {
    let Some(index) = requested_start_index else {
        return Ok(None);
    };
    if items.is_empty() {
        // An empty list still applies its edit, such as replacing the queue with nothing.
        return Ok(None);
    }
    if index >= items.len() {
        return Err(
            ExpectedRequestError::invalid_input("Invalid start index: out of range").into(),
        );
    }
    let selected = &items[index];
    if !prepared_by_item.contains_key(selected) {
        return Err(ExpectedRequestError::Unavailable {
            what: "selected track",
        }
        .into());
    }
    Ok(Some(
        items[..index]
            .iter()
            .filter(|item| prepared_by_item.contains_key(*item))
            .count(),
    ))
}

fn edit_playlist(
    player: &dyn Playable,
    prepared: &[(PlayingItem, PlaybackSource)],
    options: PlaylistEditOptions,
) -> (bool, bool, PlaylistEdit) {
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
            play: match play {
                LoadPlay::Play => true,
                LoadPlay::Pause => false,
                LoadPlay::Auto => position.is_none() || playback_mode.is_some(),
            },
        });
    let changed = (mode == PlaylistOperateMode::Replace && !player.get_playlist().is_empty())
        || !prepared.is_empty();
    // A commanded load always displaces the track that plays, and replacing the playlist
    // clears it outright; inserting behind or next to it does not. Whatever plays the
    // displaced track later is no longer the playback the current owner started.
    let displaces = load.is_some() || placement == PlaylistPlacement::Replace;
    (
        changed,
        displaces,
        PlaylistEdit {
            tracks: prepared.to_vec(),
            placement,
            playback_mode: playback_mode.map(Into::into),
            load,
            block: None,
            started_playback: None,
        },
    )
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

/// The row ids of the tracks, named by the caller, that belong to `library`.
pub(crate) async fn queue_file_ids(
    items: &[PlayingItemRef],
    library: &LibraryHandle,
) -> Result<Vec<i32>> {
    let library_id = library.library_id.to_string();
    let mut track_uuids = Vec::new();
    for item in items {
        if let PlayingItemRef::LibraryTrack {
            library_id: item_library,
            track,
        } = item
            && *item_library == library_id
        {
            track_uuids.push(Uuid::parse_str(track).map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid excluded track: {error}"))
            })?);
        }
    }
    let ids = file_ids_by_uuid(&library.main_db, &track_uuids).await?;
    Ok(track_uuids
        .iter()
        .filter_map(|uuid| ids.get(uuid).copied())
        .collect())
}

struct ResolvedMixQuery {
    playing_items: Vec<PlayingItemRef>,
    start_index: Option<usize>,
}

async fn resolve_mix_query(
    request: &ResolveMixQueryRequest,
    library: &LibraryHandle,
    pool: &Arc<LibraryPool>,
) -> Result<ResolvedMixQuery> {
    let fallback_items =
        playing_items_from_requests(&request.fallback_playing_items, Some(library)).await?;

    let tracks = if request.queries.is_empty() {
        PlayingItemActionDispatcher::new()
            .get_file_handle(&library.fsio, &library.main_db, &fallback_items)
            .await?
    } else {
        let queue = queue_file_ids(&request.excluded.items, library).await?;
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
            RESOLVED_TRACK_LIMIT,
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
    let items: Vec<PlayingItem> = prepared.iter().map(|(item, _)| item.clone()).collect();
    let (items, start_index) = if !request.instantly_play {
        (items, None)
    } else {
        let initial_item = match &request.initial_playback_item {
            Some(initial_item) => {
                Some(playing_item_from_request(initial_item.clone(), Some(library)).await?)
            }
            None => None,
        };
        let (items, start_index) = resolved_start(
            request.hint_position,
            items,
            initial_item.as_ref(),
            fallback_items,
        );
        (items, Some(start_index))
    };
    let playing_items = playing_item_requests(&items, pool, None).await;

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
        apply_player_command(
            &player,
            PlayerCommand::Load {
                index: volume as usize,
            },
        )
        .await?;
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
        apply_player_command(&player, PlayerCommand::Play).await?;
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
        apply_player_command(&player, PlayerCommand::Pause).await?;
        Ok(Some(()))
    }
}

impl ParamsExtractor for PauseReportingRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.player),)
    }
}

impl Signal for PauseReportingRequest {
    type Params = (Arc<Mutex<dyn Playable>>,);
    type Response = PauseReportingResponse;

    async fn handle(
        &self,
        (player,): Self::Params,
        _session: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        // The player answers after the commands already sent, such as a Play pressed just
        // before this pause, and counts a track that is reopening to play as playing.
        let paused = { player.lock().await.pause_reporting() };
        let paused = paused
            .await
            .map_err(|_| Error::new(player_command_error(PlayerCommandError::ThreadUnavailable)))?;
        Ok(Some(PauseReportingResponse {
            paused: paused.into(),
            call_id: None,
        }))
    }
}

/// Names who started a load: the controller device's identity for a remote load, `None` for
/// one this device itself made.
fn load_initiator(session: Option<&Session>) -> Option<PeerIdentity> {
    session.map(|session| PeerIdentity {
        fingerprint: session.fingerprint.clone(),
        public_key: session.public_key.clone(),
    })
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

        apply_player_command(&player, PlayerCommand::Next).await?;
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

        apply_player_command(&player, PlayerCommand::Previous).await?;
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
        apply_player_command(&player, PlayerCommand::SetPlaybackMode(mode.into())).await?;
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
        let mut completion = None;
        node.session()?
            .edit_queue(dart_signal.expected_revision, |player| {
                let playlist = player.get_playlist();
                if index >= playlist.len() {
                    invalid_index = true;
                    return false;
                }
                previous_item = player.get_status().item;
                selected_item = Some(playlist[index].clone());
                completion = Some(player.command_with_completion(PlayerCommand::Switch(index)));
                false
            })
            .await?;
        if invalid_index {
            return Err(
                ExpectedRequestError::invalid_input("Invalid switch index: out of range").into(),
            );
        }
        if selected_item.is_some() {
            completion
                .context("Switch command completion was not created")?
                .await
                .map_err(|error| Error::new(player_command_error(error)))?;
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
        apply_player_command(&player, PlayerCommand::Seek(dart_signal.position_seconds)).await?;
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
        apply_player_command(&player, PlayerCommand::SetVolume(volume)).await?;
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
        apply_player_command(&player, PlayerCommand::SetRealtimeFFTEnabled(enabled)).await?;
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
        apply_player_command(&player, PlayerCommand::SetAdaptiveSwitchingEnabled(enabled)).await?;
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
        apply_player_command(
            &player,
            PlayerCommand::SetMixWithOthers(dart_signal.enabled),
        )
        .await?;
        Ok(Some(()))
    }
}

impl ParamsExtractor for ResolveMixQueryRequest {
    type Params = (Option<LibraryHandle>, Arc<LibraryPool>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.library.clone(), all_params.library_pool.clone())
    }
}

impl Signal for ResolveMixQueryRequest {
    type Params = (Option<LibraryHandle>, Arc<LibraryPool>);
    type Response = ResolveMixQueryResponse;

    async fn handle(
        &self,
        (library, pool): Self::Params,
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
        let resolved = resolve_mix_query(request, &library, &pool).await?;
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
            session.as_ref(),
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
            .take_back_handoff_block(
                PlayerHandoffBlockId::try_from(&request.block)?,
                request.resume,
            )
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
            let result = apply_cast_load(&node, request, session.as_ref()).await?;
            return Ok(Some(LoadResolvedItemsResponse {
                call_id: None,
                result,
            }));
        } else {
            if let Some(binding) = node.open_service_played_here()? {
                let items = resolve_open_service_tracks(&request.items, binding.as_ref()).await?;
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
                    .edit_queue_with_load(
                        request.expected_revision,
                        load_initiator(session.as_ref()),
                        |player, started_report| {
                            let (changed, displaces, mut edit) = edit_playlist(
                                player,
                                &prepared,
                                PlaylistEditOptions {
                                    mode: request.mode,
                                    start_index,
                                    playback_mode: request.playback_mode,
                                    position: request.position_seconds.and_then(|seconds| {
                                        Duration::try_from_secs_f64(seconds).ok()
                                    }),
                                    play: request.play,
                                },
                            );
                            edit.started_playback = Some(started_report);
                            let completion =
                                Some(player.command_with_completion(PlayerCommand::EditPlaylist(
                                    edit, None,
                                )));
                            (changed, false, displaces, completion)
                        },
                    )
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
                                Error::new(ExpectedRequestError::Unavailable {
                                    what: "resolved library item",
                                })
                                .context(error)
                            })?;
                        if item_library.descriptor.protocol.service().is_some() {
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
                        Error::new(ExpectedRequestError::Unavailable {
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
            let requested_start_index = request
                .start_index
                .map(usize::try_from)
                .transpose()
                .map_err(|error| {
                    ExpectedRequestError::invalid_input(format!("Invalid start index: {error}"))
                })?;
            let start_index =
                remap_local_start_index(&items, &prepared_by_item, requested_start_index)?;
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
                .edit_queue_with_load(
                    request.expected_revision,
                    load_initiator(session.as_ref()),
                    |player, started_report| {
                        let (changed, displaces, mut edit) = edit_playlist(
                            player,
                            &prepared,
                            PlaylistEditOptions {
                                mode: request.mode,
                                start_index,
                                playback_mode: request.playback_mode,
                                position,
                                play: request.play,
                            },
                        );
                        edit.started_playback = Some(started_report);
                        let completion = Some(
                            player.command_with_completion(PlayerCommand::EditPlaylist(edit, None)),
                        );
                        (changed, false, displaces, completion)
                    },
                )
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
        refuse_libraries_kept_here(&node, true, |visit| request.load.visit_libraries(visit))?;
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
    session: Option<&Session>,
) -> Result<ResolvedItemsLoadResult> {
    let (result, _) = apply_resolved_items_load(node, request, None, session).await?;
    Ok(result)
}

async fn apply_handoff_load(
    node: &NodeContext,
    block: PlayerHandoffBlockId,
    request: &LoadResolvedItemsRequest,
    session: Option<&Session>,
) -> Result<PlayingItemRef> {
    if request.mode != PlaylistOperateMode::PlayNext {
        return Err(ExpectedRequestError::invalid_input("Handoff load must use PlayNext").into());
    }
    let (_, handoff_item) = apply_resolved_items_load(node, request, Some(block), session).await?;
    handoff_item.ok_or_else(|| {
        ExpectedRequestError::Unavailable {
            what: "handoff block item",
        }
        .into()
    })
}

/// Applies the load and, when it starts playback, names `session`'s device the owner of the
/// playback that runs from then on: the controller device for a remote start, no one in
/// particular (`None`) for a start this device itself made.
async fn apply_resolved_items_load(
    node: &NodeContext,
    request: &LoadResolvedItemsRequest,
    block: Option<PlayerHandoffBlockId>,
    session: Option<&Session>,
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
                    Error::new(ExpectedRequestError::invalid_input(format!(
                        "Invalid cast library id: {error}"
                    )))
                })?,
                file_uuid: Uuid::parse_str(track).map_err(|error| {
                    Error::new(ExpectedRequestError::invalid_input(format!(
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
                Error::new(ExpectedRequestError::Unavailable {
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
        let play = request.play != LoadPlay::Pause;
        node.session()?
            .handoff_load_queue(HandoffLoadParams {
                expected_revision: request.expected_revision,
                tracks: prepared,
                block,
                start_index,
                position,
                play,
                playback_mode: request.playback_mode.map(Into::into),
                initiator: load_initiator(session),
            })
            .await?;
    } else {
        node.session()?
            .edit_queue_with_load(
                request.expected_revision,
                load_initiator(session),
                |player, started_report| {
                    let (changed, displaces, mut edit) = edit_playlist(
                        player,
                        &prepared,
                        PlaylistEditOptions {
                            mode: request.mode,
                            start_index,
                            playback_mode: request.playback_mode,
                            position,
                            play: request.play,
                        },
                    );
                    edit.started_playback = Some(started_report);
                    let completion = Some(
                        player.command_with_completion(PlayerCommand::EditPlaylist(edit, None)),
                    );
                    (changed, false, displaces, completion)
                },
            )
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
    let service_library_ids = registry
        .libraries
        .iter()
        .filter_map(|library| library.protocol.service().map(|_| library.record_id.0))
        .collect();
    session_snapshot_from_status(status, revision, &service_library_ids)
}

pub(crate) fn session_snapshot_from_status(
    status: PlayerStatus,
    revision: u64,
    service_library_ids: &HashSet<Uuid>,
) -> Result<SessionSnapshotResponse> {
    let items = status
        .playlist
        .into_iter()
        .map(|entry| match entry.item {
            PlayingItem::LibraryTrack { library_id, track }
                if !service_library_ids.contains(&library_id) =>
            {
                Ok(PlayingItemRef::LibraryTrack {
                    library_id: library_id.to_string(),
                    track,
                })
            }
            PlayingItem::LibraryTrack { .. } => {
                Err(Error::new(ExpectedRequestError::Unavailable {
                    what: "handing off files outside a library",
                }))
            }
            _ => Err(Error::new(ExpectedRequestError::Unavailable {
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
        playing: status.state == PlayerPlaybackState::Playing,
        revision,
    })
}

impl ParamsExtractor for PrepareHandoffRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}

impl Signal for PrepareHandoffRequest {
    type Params = Arc<NodeContext>;
    type Response = PrepareHandoffResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        _request: &Self,
    ) -> Result<Option<Self::Response>> {
        Ok(Some(PrepareHandoffResponse {
            ticket: node.handoff_tickets.prepare().to_string(),
            call_id: None,
        }))
    }
}

impl ParamsExtractor for CancelHandoffRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}

impl Signal for CancelHandoffRequest {
    type Params = Arc<NodeContext>;
    type Response = CancelHandoffResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let ticket = Uuid::parse_str(&request.ticket)
            .map_err(|_| ExpectedRequestError::HandoffSuperseded)?;
        node.handoff_tickets.cancel(ticket).await?;
        Ok(Some(CancelHandoffResponse { call_id: None }))
    }
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
        let run = match request.ticket.as_deref() {
            Some(ticket) => {
                let ticket =
                    Uuid::parse_str(ticket).map_err(|_| ExpectedRequestError::HandoffSuperseded)?;
                Some(node.handoff_tickets.start(ticket).await?)
            }
            None => None,
        };
        let result = request.handoff(&node, run.as_ref()).await;
        if let Some(run) = run {
            run.finish();
        }
        result
    }
}

impl HandoffRequest {
    async fn handoff(
        &self,
        node: &Arc<NodeContext>,
        run: Option<&HandoffRun<'_>>,
    ) -> Result<Option<HandoffResponse>> {
        let generation = node.begin_handoff().await;
        run.map_or(Ok(()), HandoffRun::check_cancelled)?;
        let attached = node.renderer_link()?;
        let target = self.target.as_deref();
        let already_there = match (target, &attached) {
            (Some(target), Some(link)) => link.fingerprint() == target,
            (None, None) => true,
            _ => false,
        };
        if already_there {
            run.map_or(Ok(()), HandoffRun::commit)?;
            return Ok(Some(HandoffResponse { call_id: None }));
        }
        let paused = pause_handoff_source(node, attached.clone(), generation).await?;
        if target.is_none()
            && let Some(run) = run
        {
            // Returning to this device starts by taking back the previous block. Once that
            // begins, finish the existing return path to settle the queues on both sides.
            if let Err(error) = run.commit() {
                resume_handoff_source(node, paused).await?;
                return Err(error.into());
            }
        }
        if target.is_none()
            && let PausedSource::Renderer(source) = &paused.source
            && let Some(block) = node.take_handoff_block(source.fingerprint()).await
        {
            let take_back = source
                .call(&TakeBackHandoffBlockRequest {
                    block: block.into(),
                    resume: false,
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
                    if let Err(error) = apply_cast_load(node, &load, None).await {
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
                        resume_handoff_source(node, paused).await?;
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
                    if !NodeContext::handoff_block_confirmed_absent(&error) {
                        // The answer never came back, so the target may still hold the block
                        // this return path cannot see anymore.
                        node.note_pending_handoff_rollback(source.fingerprint(), block)
                            .await;
                    }
                    log::info!("Handoff block is unavailable; using the existing return path");
                }
                Err(error) => {
                    node.remember_handoff_block(source.fingerprint().to_owned(), block)
                        .await;
                    // A failure that is not a confirmed absence leaves the block's state on the
                    // target unknown; a later connection must ask again.
                    if !NodeContext::handoff_block_confirmed_absent(&error) {
                        node.note_pending_handoff_rollback(source.fingerprint(), block)
                            .await;
                    }
                    resume_handoff_source(node, paused).await?;
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
                    hand_off_to_renderer(node, target, block, load, expected_position, run).await?;
                }
                None => hand_off_to_this_device(node, attached.clone(), &load).await?,
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
                node.detach_renderer().await?;
                if let Some(source) = &attached {
                    // The failed target already has its own session back. Attaching over it
                    // directly would pause that restored session again.
                    node.attach_renderer(source.fingerprint()).await?;
                }
            }
            resume_handoff_source(node, paused).await?;
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
        // A renderer whose protocol predates the reporting pause cannot answer what its
        // pause stopped, so the snapshot before the pause is its only answer. For one that
        // can, the same snapshot is the recovery answer of a pause whose own answer is lost:
        // the pause may have landed while its reply did not.
        let before = source
            .call(&SessionSnapshotRequest { call_id: None })
            .await?;
        // A queue the destination cannot load fails the handoff before the source is
        // touched: a pause is more than an interruption, because it settles a pending
        // automatic resume.
        if before
            .items
            .iter()
            .any(|item| !matches!(item, PlayingItemRef::LibraryTrack { .. }))
        {
            return Err(ExpectedRequestError::Unavailable {
                what: "casting files outside a library",
            }
            .into());
        }
        // The pause answers what it stopped: a play that reached the player after the last
        // snapshot, such as a track reopening to play, counts as playing here.
        let paused = match node
            .pause_handoff_source(source.fingerprint(), &source, generation)
            .await
        {
            Ok(paused) => paused,
            Err(error) => {
                // A pause that fails leaves the source in an unknown state: its reply may
                // be all that was lost. When the source was playing before the handoff,
                // leaving it paused loses playback the user never asked to stop, so it is
                // resumed.
                if before.playing {
                    source.call(&PlayRequest { call_id: None }).await?;
                }
                return Err(error);
            }
        };
        let Some(paused) = paused else {
            return Err(ExpectedRequestError::HandoffSuperseded.into());
        };
        let was_playing = match paused {
            HandoffPause::Reported(report) => report == PauseReport::Playing,
            HandoffPause::Unreported => before.playing,
        };
        let snapshot = source.call(&SessionSnapshotRequest { call_id: None }).await;
        let snapshot = match snapshot {
            Ok(snapshot) => snapshot,
            Err(error) => {
                if was_playing {
                    source.call(&PlayRequest { call_id: None }).await?;
                }
                return Err(error);
            }
        };
        if snapshot.playing {
            if was_playing {
                source.call(&PlayRequest { call_id: None }).await?;
            }
            // Something resumed the source while it was being handed off.
            return Err(ExpectedRequestError::HandoffSuperseded.into());
        }
        Ok(PausedHandoffSource {
            snapshot,
            source: PausedSource::Renderer(source),
            was_playing,
        })
    } else {
        let session = node.session()?;
        // The player answers after the commands already sent, such as a Play pressed just before
        // the handoff, and counts a track that is loading to play as playing.
        let paused = { session.player.lock().await.pause_reporting() };
        let was_playing = paused
            .await
            .map_err(|_| Error::new(player_command_error(PlayerCommandError::ThreadUnavailable)))?
            == PausedPlayback::Playing;
        let snapshot = match session_snapshot(node, &session).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                if was_playing {
                    apply_player_command(&session.player, PlayerCommand::Play).await?;
                }
                return Err(error);
            }
        };
        if snapshot.playing {
            if was_playing {
                apply_player_command(&session.player, PlayerCommand::Play).await?;
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
            apply_player_command(&session.player, PlayerCommand::Play).await?;
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
        play: if snapshot.playing {
            LoadPlay::Play
        } else {
            LoadPlay::Pause
        },
    }
}

/// Load the paused snapshot onto a renderer target.
async fn hand_off_to_renderer(
    node: &Arc<NodeContext>,
    target: &str,
    block: PlayerHandoffBlockId,
    load: LoadResolvedItemsRequest,
    expected_position: f64,
    run: Option<&HandoffRun<'_>>,
) -> Result<()> {
    let check_cancelled = || run.map_or(Ok(()), HandoffRun::check_cancelled);
    check_cancelled()?;
    let (connected_link, alias) = node
        .connect_renderer(target, RendererLinkUse::Control)
        .await?;
    // A rollback whose outcome is still unknown may have left its block on the target: loading
    // the new one on top of it would stack both. The connection just reconciled what it could,
    // so anything still pending now refused to resolve; the handoff waits for a connection that
    // can clear it.
    if node.has_pending_handoff_rollbacks(target).await {
        connected_link.close();
        return Err(ExpectedRequestError::Unavailable {
            what: "the target renderer's unresolved handoff rollback",
        }
        .into());
    }
    if let Err(error) = check_cancelled() {
        connected_link.close();
        return Err(error.into());
    }
    let mut events = node.subscribe_handoff_playback_events();
    // The target starts the handed-off track as the load asks: playing, or paused at its place.
    let expected_state = if load.play != LoadPlay::Pause {
        PlaybackState::Playing
    } else {
        PlaybackState::Paused
    };
    let handoff = HandoffLoadRequest {
        block: block.into(),
        load,
        call_id: None,
    };
    let response = match settle_handoff_load(run, connected_link.call(&handoff), async |error| {
        roll_back_handoff_block(node, &connected_link, block, error, run).await
    })
    .await
    {
        Ok(response) => response,
        Err(error) => {
            connected_link.close();
            return Err(error);
        }
    };
    let target_item = response.handoff_item;
    let target_link = connected_link.id();
    let previous_player = node.current_player_snapshot()?;
    node.install_renderer_link(connected_link, alias)?;
    node.transition_current_player(previous_player, node.current_player()?)
        .await?;
    let link = node
        .renderer_link()?
        .context("renderer link disappeared after handoff load")?;
    node.remember_handoff_block(target.to_owned(), block).await;
    node.publish_capabilities()?;
    let confirm = async {
        if let Err(error) = link.call(&RefreshSessionRequest { call_id: None }).await {
            warn!("Handoff succeeded, but renderer state refresh failed: {error:#}");
        }
        wait_for_handoff_start(
            &mut events,
            target_link,
            &target_item,
            expected_position,
            expected_state,
        )
        .await
    };
    let confirmed = match run {
        Some(run) => run.confirm(confirm).await,
        None => confirm.await,
    };
    if let Err(error) = confirmed {
        return Err(roll_back_handoff_block(node, &link, block, error, run).await);
    }
    Ok(())
}

/// An already sent Load may still apply. Wait for the reply before removing its block.
async fn settle_handoff_load<T>(
    run: Option<&HandoffRun<'_>>,
    load: impl Future<Output = Result<T>>,
    roll_back: impl AsyncFnOnce(Error) -> Error,
) -> Result<T> {
    let response = load.await;
    match run.map_or(Ok(()), HandoffRun::check_cancelled) {
        Ok(()) => response,
        Err(error) => Err(roll_back(error.into()).await),
    }
}

async fn roll_back_handoff_block(
    node: &NodeContext,
    link: &RendererLink,
    block: PlayerHandoffBlockId,
    error: Error,
    run: Option<&HandoffRun<'_>>,
) -> Error {
    match link
        .call(&TakeBackHandoffBlockRequest {
            block: block.into(),
            resume: true,
            call_id: None,
        })
        .await
    {
        Ok(_) => {
            node.forget_handoff_block(link.fingerprint(), block).await;
            error
        }
        Err(rollback_error) => {
            if NodeContext::handoff_block_confirmed_absent(&rollback_error) {
                node.forget_handoff_block(link.fingerprint(), block).await;
            } else {
                if let Some(run) = run {
                    run.mark_rollback_unsettled();
                }
                node.note_pending_handoff_rollback(link.fingerprint(), block)
                    .await;
            }
            warn!("Could not roll back failed handoff block: {rollback_error:#}");
            error.context(format!("handoff rollback failed: {rollback_error:#}"))
        }
    }
}

/// Load the paused snapshot locally, then detach from its renderer source.
async fn hand_off_to_this_device(
    node: &Arc<NodeContext>,
    attached: Option<Arc<RendererLink>>,
    load: &LoadResolvedItemsRequest,
) -> Result<()> {
    let session = node.session()?;
    let original = session_snapshot(node, &session).await?;
    if let Err(error) = apply_cast_load(node, load, None).await {
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
            if let Err(restore_error) = apply_cast_load(node, &restore, None).await {
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

/// Waits for the target to report the handed-off track at the handed-off place in the expected
/// state, playing or paused, or to report it unavailable. Only the reports that came over
/// `target_link` count.
async fn wait_for_handoff_start(
    events: &mut broadcast::Receiver<HandoffPlaybackEvent>,
    target_link: Uuid,
    expected_item: &PlayingItemRef,
    expected_position: f64,
    expected_state: PlaybackState,
) -> Result<()> {
    tokio_time::timeout(HANDOFF_START_TIMEOUT, async {
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
            if event.link != target_link {
                continue;
            }
            match event.report {
                HandoffPlaybackReport::Playlist(items) => {
                    if items.iter().any(|item| {
                        same_handoff_item(&item.item, expected_item) && item.unavailable.is_some()
                    }) {
                        return Err(ExpectedRequestError::Unavailable {
                            what: "handed-off track",
                        }
                        .into());
                    }
                }
                HandoffPlaybackReport::Status(status) => {
                    if status
                        .item_request
                        .as_ref()
                        .is_some_and(|item| same_handoff_item(item, expected_item))
                        && status.state == expected_state
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
        Error::new(ExpectedRequestError::Unavailable {
            what: "handoff playback start",
        })
        .context(format!(
            "expected item {expected_item:?} {expected_state:?} at {expected_position}s"
        ))
    })?
}

fn same_handoff_item(left: &PlayingItemRef, right: &PlayingItemRef) -> bool {
    left == right
}

#[cfg(test)]
mod handoff_confirmation_tests {
    use super::*;
    use std::time::Instant;
    use tokio::time as tokio_time;

    #[tokio::test]
    async fn cancelled_load_waits_for_the_reply_and_rolls_back_before_settling() {
        use crate::node::HandoffTickets;
        use std::sync::atomic::{AtomicBool, Ordering};
        use tokio::sync::{Notify, oneshot};

        let owner = HandoffTickets::default();
        let ticket = owner.prepare();
        let entered = Notify::new();
        let (release, reply) = oneshot::channel();
        let target_applied = AtomicBool::new(false);
        let handoff = async {
            let run = owner.start(ticket).await.unwrap();
            let result = settle_handoff_load(
                Some(&run),
                async {
                    entered.notify_one();
                    reply.await?;
                    target_applied.store(true, Ordering::Relaxed);
                    Ok(())
                },
                async |error| {
                    assert!(target_applied.swap(false, Ordering::Relaxed));
                    error
                },
            )
            .await;
            run.finish();
            assert!(matches!(
                result.unwrap_err().downcast_ref(),
                Some(ExpectedRequestError::HandoffSuperseded)
            ));
        };
        let close = async {
            entered.notified().await;
            let cancel = owner.cancel(ticket);
            tokio::pin!(cancel);
            assert!(futures::poll!(&mut cancel).is_pending());
            assert!(!target_applied.load(Ordering::Relaxed));
            release.send(()).unwrap();
            cancel.await.unwrap();
            assert!(!target_applied.load(Ordering::Relaxed));
        };
        tokio::join!(handoff, close);
        assert!(owner.start(owner.prepare()).await.unwrap().commit().is_ok());
    }

    #[tokio::test]
    async fn slow_handoff_playback_confirmation_completes_when_status_arrives() -> Result<()> {
        let expected_item = PlayingItemRef::IndependentFile {
            path: "slow-start.ogg".into(),
        };
        let target_link = Uuid::new_v4();
        let (sender, mut events) = broadcast::channel(1);
        let delayed_sender = sender.clone();
        let delayed_item = expected_item.clone();
        tokio::spawn(async move {
            tokio_time::sleep(Duration::from_millis(100)).await;
            let _ = delayed_sender.send(HandoffPlaybackEvent {
                link: target_link,
                report: HandoffPlaybackReport::Status(Box::new(playing_status(delayed_item, 42.0))),
            });
        });

        let started = Instant::now();
        wait_for_handoff_start(
            &mut events,
            target_link,
            &expected_item,
            42.0,
            PlaybackState::Playing,
        )
        .await?;
        assert!(started.elapsed() >= Duration::from_millis(100));
        Ok(())
    }

    #[tokio::test]
    async fn only_the_target_link_confirms_or_fails_a_handoff() -> Result<()> {
        let expected_item = PlayingItemRef::IndependentFile {
            path: "handed-off.ogg".into(),
        };
        let target_link = Uuid::new_v4();
        let (sender, mut events) = broadcast::channel(4);
        // The track playing at the handed-off place over another link, such as the renderer
        // handed off from, is not the target starting it.
        sender
            .send(HandoffPlaybackEvent {
                link: Uuid::new_v4(),
                report: HandoffPlaybackReport::Status(Box::new(playing_status(
                    expected_item.clone(),
                    42.0,
                ))),
            })
            .map_err(|_| anyhow::anyhow!("handoff event channel closed"))?;
        sender
            .send(HandoffPlaybackEvent {
                link: target_link,
                report: HandoffPlaybackReport::Playlist(vec![PlaylistItem {
                    unavailable: Some(PlaylistUnavailableReason::Unreadable),
                    item: expected_item.clone(),
                    artist: String::new(),
                    album: String::new(),
                    title: String::new(),
                    duration: 0.0,
                }]),
            })
            .map_err(|_| anyhow::anyhow!("handoff event channel closed"))?;

        let error = wait_for_handoff_start(
            &mut events,
            target_link,
            &expected_item,
            42.0,
            PlaybackState::Playing,
        )
        .await
        .expect_err("a status from another link confirmed the handoff");
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Unavailable {
                what: "handed-off track"
            })
        ));
        Ok(())
    }

    fn playing_status(item: PlayingItemRef, progress_seconds: f32) -> PlaybackStatus {
        PlaybackStatus {
            session_id: "handoff".into(),
            state: PlaybackState::Playing,
            progress_seconds,
            progress_percentage: 0.0,
            artist: None,
            album: None,
            title: None,
            duration: 0.0,
            index: Some(0),
            item: None,
            item_request: Some(item),
            playback_mode: PlaybackMode::Sequential,
            ready: true,
            cover_art_path: None,
            lib_path: String::new(),
        }
    }
}
