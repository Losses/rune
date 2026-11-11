use anyhow::{Result, anyhow};
use database::actions::utils::generate_group_name;
use database::actions::{collection::CollectionQueryType, search::convert_to_collection_types};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};
use subsonic::{
    SearchLimits, SubsonicAlbum, SubsonicArtist, SubsonicError, SubsonicServerError, SubsonicSong,
};

use crate::TypedRequest;
use crate::macros::{RequestSlots, RequestTranslator};
use crate::messages::{
    AddItemToPlaylistRequest, AddItemToPlaylistResponse, AudioAvailability, AudioOrigin,
    CastPreflightRequest, Collection, CollectionGroup, CollectionGroupSummary,
    CollectionGroupSummaryResponse, CollectionType, CreatePlaylistRequest, CreatePlaylistResponse,
    FetchAllPlaylistsRequest, FetchAllPlaylistsResponse, FetchCollectionByIdsRequest,
    FetchCollectionByIdsResponse, FetchCollectionGroupSummaryRequest, FetchCollectionGroupsRequest,
    FetchCollectionGroupsResponse, FetchMediaFileByIdsRequest, FetchMediaFileByIdsResponse,
    GetPlaylistByIdRequest, GetPlaylistByIdResponse, HandoffLoadRequest, LoadRequest,
    LoadResolvedItemsRequest, LoadResolvedItemsResponse, MediaFile, MixQueryRequest,
    MixQueryResponse, MovePlaylistItemRequest, NextRequest, PauseRequest, PlayRequest,
    PlayingItemRef, Playlist, PlaylistOperateMode, PreviousRequest, RefreshSessionRequest,
    RemoveItemFromPlaylistRequest, RemoveItemFromPlaylistResponse, RemovePlaylistRequest,
    RemovePlaylistResponse, RemoveRequest, ReorderPlaylistItemPositionRequest,
    ReorderPlaylistItemPositionResponse, SearchForRequest, SearchForResponse, SeekRequest,
    SessionSnapshotRequest, SetAdaptiveSwitchingEnabledRequest, SetPlaybackModeRequest,
    SetRealtimeFFTEnabledRequest, SwitchRequest, TakeBackHandoffBlockRequest,
    UpdatePlaylistRequest, UpdatePlaylistResponse, VolumeRequest, VolumeResponse,
};
use crate::request_error::ExpectedRequestError;
use crate::rpc::{RequestTranslationFuture, Slot, UnsupportedReason};
use crate::subsonic_binding::{SubsonicBinding, SubsonicIdKind};
use database::actions::mixes::{QueryOperator, parse_query};
use uuid::Uuid;

fn resolve_playlist_id(binding: &SubsonicBinding, playlist_id: i32) -> Result<String> {
    let catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    Ok(catalog
        .ids
        .resolve(SubsonicIdKind::Playlist, playlist_id)?
        .to_owned())
}

fn playlist_id(binding: &SubsonicBinding, remote_id: &str) -> Result<i32> {
    binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?
        .ids
        .intern(SubsonicIdKind::Playlist, remote_id)
}

async fn fetch_remote_playlist(
    binding: &SubsonicBinding,
    id: i32,
) -> Result<(String, Vec<String>)> {
    let remote_id = resolve_playlist_id(binding, id)?;
    let playlist = binding
        .client
        .playlist(&remote_id)
        .await
        .map_err(subsonic_request_error)?;
    Ok((
        remote_id,
        playlist.songs.into_iter().map(|song| song.id).collect(),
    ))
}

pub(crate) async fn fetch_all_playlists_request(
    binding: &SubsonicBinding,
    _: &FetchAllPlaylistsRequest,
) -> Result<Option<FetchAllPlaylistsResponse>> {
    let remote = binding
        .client
        .playlists()
        .await
        .map_err(subsonic_request_error)?;
    let mut catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    let playlists = remote
        .into_iter()
        .map(|item| {
            Ok(Playlist {
                id: catalog.ids.intern(SubsonicIdKind::Playlist, &item.id)?,
                name: item.name,
                group: String::new(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(FetchAllPlaylistsResponse {
        playlists,
        call_id: None,
    }))
}

pub(crate) async fn get_playlist_by_id_request(
    binding: &SubsonicBinding,
    request: &GetPlaylistByIdRequest,
) -> Result<Option<GetPlaylistByIdResponse>> {
    let remote_id = resolve_playlist_id(binding, request.playlist_id)?;
    let remote = binding
        .client
        .playlist(&remote_id)
        .await
        .map_err(subsonic_request_error)?;
    Ok(Some(GetPlaylistByIdResponse {
        playlist: Playlist {
            id: playlist_id(binding, &remote.id)?,
            name: remote.name,
            group: String::new(),
        },
        call_id: None,
    }))
}

pub(crate) async fn create_playlist_request(
    binding: &SubsonicBinding,
    request: &CreatePlaylistRequest,
) -> Result<Option<CreatePlaylistResponse>> {
    let existing_ids = binding
        .client
        .playlists()
        .await
        .map_err(subsonic_request_error)?
        .into_iter()
        .map(|item| item.id)
        .collect::<HashSet<_>>();
    binding
        .client
        .create_playlist(&request.name, &[])
        .await
        .map_err(subsonic_request_error)?;
    let remote = binding
        .client
        .playlists()
        .await
        .map_err(subsonic_request_error)?;
    let created = remote
        .into_iter()
        .find(|item| item.name == request.name && !existing_ids.contains(&item.id))
        .ok_or_else(|| ExpectedRequestError::UnexpectedResponse {
            expected: "created Subsonic playlist".into(),
            actual: "no matching playlist returned".into(),
        })?;
    Ok(Some(CreatePlaylistResponse {
        playlist: Playlist {
            id: playlist_id(binding, &created.id)?,
            name: created.name,
            group: String::new(),
        },
        call_id: None,
    }))
}

pub(crate) async fn update_playlist_request(
    binding: &SubsonicBinding,
    request: &UpdatePlaylistRequest,
) -> Result<Option<UpdatePlaylistResponse>> {
    let remote_id = resolve_playlist_id(binding, request.playlist_id)?;
    binding
        .client
        .update_playlist(&remote_id, Some(&request.name), &[], &[])
        .await
        .map_err(subsonic_request_error)?;
    Ok(Some(UpdatePlaylistResponse {
        playlist: Playlist {
            id: request.playlist_id,
            name: request.name.clone(),
            group: String::new(),
        },
        call_id: None,
    }))
}

pub(crate) async fn remove_playlist_request(
    binding: &SubsonicBinding,
    request: &RemovePlaylistRequest,
) -> Result<Option<RemovePlaylistResponse>> {
    let remote_id = resolve_playlist_id(binding, request.playlist_id)?;
    binding
        .client
        .delete_playlist(&remote_id)
        .await
        .map_err(subsonic_request_error)?;
    Ok(Some(RemovePlaylistResponse {
        playlist_id: request.playlist_id,
        success: true,
        call_id: None,
    }))
}

pub(crate) async fn add_item_to_playlist_request(
    binding: &SubsonicBinding,
    request: &AddItemToPlaylistRequest,
) -> Result<Option<AddItemToPlaylistResponse>> {
    let playlist_remote = resolve_playlist_id(binding, request.playlist_id)?;
    let track_remote = {
        let catalog = binding
            .catalog
            .lock()
            .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
        catalog
            .ids
            .resolve(SubsonicIdKind::Track, request.media_file_id)?
            .to_owned()
    };
    let playlist = binding
        .client
        .playlist(&playlist_remote)
        .await
        .map_err(subsonic_request_error)?;
    let songs = playlist
        .songs
        .iter()
        .map(|song| song.id.clone())
        .collect::<Vec<_>>();
    let append_position = i32::try_from(songs.len())
        .map_err(|_| ExpectedRequestError::invalid_input("Subsonic playlist is too large"))?;
    if request
        .position
        .is_some_and(|position| position != append_position)
    {
        return Err(ExpectedRequestError::Unsupported {
            request: AddItemToPlaylistRequest::wire_name(),
            reason: UnsupportedReason::NotImplemented,
        }
        .into());
    }
    binding
        .client
        .update_playlist(&playlist_remote, None, &[&track_remote], &[])
        .await
        .map_err(subsonic_request_error)?;
    Ok(Some(AddItemToPlaylistResponse {
        success: true,
        occurrence_id: append_position.to_string(),
        call_id: None,
    }))
}

fn occurrence_position(value: &str, len: usize) -> Result<usize> {
    let position = value
        .parse::<usize>()
        .map_err(|_| ExpectedRequestError::invalid_input("Invalid Subsonic playlist occurrence"))?;
    if position >= len {
        return Err(
            ExpectedRequestError::invalid_input("Unknown Subsonic playlist occurrence").into(),
        );
    }
    Ok(position)
}

pub(crate) async fn remove_item_from_playlist_request(
    binding: &SubsonicBinding,
    request: &RemoveItemFromPlaylistRequest,
) -> Result<Option<RemoveItemFromPlaylistResponse>> {
    let (remote_id, songs) = fetch_remote_playlist(binding, request.playlist_id).await?;
    let position = occurrence_position(&request.occurrence_id, songs.len())?;
    let position = u32::try_from(position)
        .map_err(|_| ExpectedRequestError::invalid_input("Subsonic playlist is too large"))?;
    binding
        .client
        .update_playlist(&remote_id, None, &[], &[position])
        .await
        .map_err(subsonic_request_error)?;
    Ok(Some(RemoveItemFromPlaylistResponse {
        success: true,
        error: String::new(),
        call_id: None,
    }))
}

pub(crate) async fn reorder_playlist_item_position_request(
    binding: &SubsonicBinding,
    request: &ReorderPlaylistItemPositionRequest,
) -> Result<Option<ReorderPlaylistItemPositionResponse>> {
    let (remote_id, songs) = fetch_remote_playlist(binding, request.playlist_id).await?;
    let from = occurrence_position(&request.occurrence_id, songs.len())?;
    let target = usize::try_from(request.new_position)
        .map_err(|_| ExpectedRequestError::invalid_input("Invalid playlist position"))?;
    if target >= songs.len() {
        return Err(ExpectedRequestError::invalid_input("Invalid playlist position").into());
    }
    let mut reordered = songs;
    let song = reordered.remove(from);
    reordered.insert(target, song);
    let refs = reordered.iter().map(String::as_str).collect::<Vec<_>>();
    binding
        .client
        .replace_playlist_songs(&remote_id, &refs)
        .await
        .map_err(subsonic_request_error)?;
    Ok(Some(ReorderPlaylistItemPositionResponse {
        success: true,
        call_id: None,
    }))
}

fn subsonic_cover_map(
    binding: &SubsonicBinding,
    covers: Vec<(i32, String)>,
) -> std::collections::HashMap<i32, String> {
    let mut map = HashMap::new();
    for (object_id, cover_id) in covers {
        if let Some(path) = binding.covers.cached_path(&cover_id) {
            map.insert(object_id, path);
        } else {
            binding
                .covers
                .schedule(Arc::clone(&binding.client), cover_id);
        }
    }
    map
}

pub(crate) async fn search_for_request(
    binding: &SubsonicBinding,
    request: &SearchForRequest,
) -> Result<Option<SearchForResponse>> {
    let fields = convert_to_collection_types(request.fields.clone());
    let all = fields.is_empty();
    let count = u32::try_from(request.n.max(0)).unwrap_or(u32::MAX);
    let limits = SearchLimits {
        artists: if all || fields.contains(&CollectionQueryType::Artist) {
            count
        } else {
            0
        },
        albums: if all || fields.contains(&CollectionQueryType::Album) {
            count
        } else {
            0
        },
        songs: if all || fields.contains(&CollectionQueryType::Track) {
            count
        } else {
            0
        },
    };
    let results = binding
        .client
        .search(&request.query_str, limits)
        .await
        .map_err(subsonic_request_error)?;
    let mut catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    let artists = results
        .artists
        .iter()
        .take(limits.artists as usize)
        .map(|item| {
            let id = catalog.ids.intern(SubsonicIdKind::Artist, &item.id)?;
            catalog.artists.insert(id, item.clone());
            Ok(id)
        })
        .collect::<Result<Vec<_>>>()?;
    let albums = results
        .albums
        .iter()
        .take(limits.albums as usize)
        .map(|item| {
            let id = catalog.ids.intern(SubsonicIdKind::Album, &item.id)?;
            catalog.albums.insert(id, item.clone());
            Ok(id)
        })
        .collect::<Result<Vec<_>>>()?;
    let tracks = results
        .songs
        .iter()
        .take(limits.songs as usize)
        .map(|item| {
            let id = catalog.ids.intern(SubsonicIdKind::Track, &item.id)?;
            catalog.songs.insert(id, item.clone());
            Ok(id)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(SearchForResponse {
        call_id: request.call_id,
        artists,
        albums,
        playlists: Vec::new(),
        tracks,
        genres: Vec::new(),
    }))
}

pub(crate) async fn fetch_media_file_by_ids_request(
    binding: &SubsonicBinding,
    request: &FetchMediaFileByIdsRequest,
) -> Result<Option<FetchMediaFileByIdsResponse>> {
    let mut media_files = Vec::with_capacity(request.ids.len());
    let mut covers = Vec::with_capacity(request.ids.len());
    for id in &request.ids {
        let (remote_id, cached_song) = {
            let catalog = binding
                .catalog
                .lock()
                .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
            (
                catalog.ids.resolve(SubsonicIdKind::Track, *id)?.to_owned(),
                catalog.songs.get(id).cloned(),
            )
        };
        let song = match cached_song {
            Some(song) => song,
            None => {
                let song = binding
                    .client
                    .song(&remote_id)
                    .await
                    .map_err(subsonic_request_error)?;
                let mut catalog = binding
                    .catalog
                    .lock()
                    .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
                let id = catalog.ids.intern(SubsonicIdKind::Track, &song.id)?;
                catalog.songs.insert(id, song.clone());
                song
            }
        };
        let cover_art_id = {
            let mut catalog = binding
                .catalog
                .lock()
                .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
            song.cover_art
                .as_deref()
                .map(|cover| catalog.ids.intern(SubsonicIdKind::CoverArt, cover))
                .transpose()?
                .unwrap_or_default()
        };
        if let Some(cover) = song.cover_art.clone() {
            covers.push((*id, cover));
        }
        media_files.push(MediaFile {
            id: *id,
            library_id: binding.library_id.to_string(),
            track: song.id.clone(),
            path: String::new(),
            artist: song.artist.unwrap_or_default(),
            album: song.album.unwrap_or_default(),
            title: song.title,
            duration: f64::from(song.duration.unwrap_or_default()),
            cover_art_id,
            track_number: song
                .track
                .and_then(|number| i32::try_from(number).ok())
                .unwrap_or_default(),
            audio_availability: AudioAvailability::Unavailable,
            audio_origin: AudioOrigin::Remote,
        });
    }
    Ok(Some(FetchMediaFileByIdsResponse {
        call_id: None,
        media_files,
        cover_art_map: subsonic_cover_map(binding, covers),
    }))
}

pub(crate) async fn fetch_collection_by_ids_request(
    binding: &SubsonicBinding,
    request: &FetchCollectionByIdsRequest,
) -> Result<Option<FetchCollectionByIdsResponse>> {
    let kind = match request.collection_type {
        CollectionType::Artist => SubsonicIdKind::Artist,
        CollectionType::Album => SubsonicIdKind::Album,
        _ => {
            return Err(ExpectedRequestError::Unsupported {
                request: FetchCollectionByIdsRequest::wire_name(),
                reason: UnsupportedReason::NotImplemented,
            }
            .into());
        }
    };
    let mut result = Vec::with_capacity(request.ids.len());
    for id in &request.ids {
        let (remote_id, cached_name) = {
            let catalog = binding
                .catalog
                .lock()
                .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
            let remote_id = catalog.ids.resolve(kind, *id)?.to_owned();
            let cached_name = match kind {
                SubsonicIdKind::Artist => catalog.artists.get(id).map(|artist| artist.name.clone()),
                SubsonicIdKind::Album => catalog.albums.get(id).map(|album| album.name.clone()),
                SubsonicIdKind::Track | SubsonicIdKind::CoverArt | SubsonicIdKind::Playlist => None,
            };
            (remote_id, cached_name)
        };
        let name = match cached_name {
            Some(name) => name,
            None => match kind {
                SubsonicIdKind::Artist => {
                    let item = binding
                        .client
                        .artist(&remote_id)
                        .await
                        .map_err(subsonic_request_error)?;
                    let name = item.name.clone();
                    let mut catalog = binding
                        .catalog
                        .lock()
                        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
                    catalog.artists.insert(
                        *id,
                        SubsonicArtist {
                            id: item.id,
                            name: item.name,
                            album_count: item.album_count,
                            cover_art: item.cover_art,
                        },
                    );
                    name
                }
                SubsonicIdKind::Album => {
                    let item = binding
                        .client
                        .album(&remote_id)
                        .await
                        .map_err(subsonic_request_error)?;
                    let name = item.name.clone();
                    let mut catalog = binding
                        .catalog
                        .lock()
                        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
                    catalog.albums.insert(
                        *id,
                        SubsonicAlbum {
                            id: item.id,
                            name: item.name,
                            artist: item.artist,
                            artist_id: item.artist_id,
                            song_count: item.song_count,
                            duration: item.duration,
                            year: item.year,
                            genre: item.genre,
                            cover_art: item.cover_art,
                        },
                    );
                    name
                }
                SubsonicIdKind::Track | SubsonicIdKind::CoverArt => {
                    return Err(anyhow!("Unexpected Subsonic track collection"));
                }
                SubsonicIdKind::Playlist => {
                    return Err(anyhow!("Unexpected Subsonic playlist collection"));
                }
            },
        };
        let cover_id = {
            let mut catalog = binding
                .catalog
                .lock()
                .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
            let cover = match kind {
                SubsonicIdKind::Artist => catalog
                    .artists
                    .get(id)
                    .and_then(|item| item.cover_art.clone()),
                SubsonicIdKind::Album => catalog
                    .albums
                    .get(id)
                    .and_then(|item| item.cover_art.clone()),
                SubsonicIdKind::Track | SubsonicIdKind::CoverArt | SubsonicIdKind::Playlist => None,
            };
            if let Some(cover_id) = cover.as_deref() {
                catalog.ids.intern(SubsonicIdKind::CoverArt, cover_id)?;
            }
            cover
        };
        let cover_art_map = subsonic_cover_map(
            binding,
            cover_id.map(|cover| vec![(*id, cover)]).unwrap_or_default(),
        );
        result.push(Collection {
            id: *id,
            name,
            queries: Vec::new(),
            collection_type: request.collection_type,
            cover_art_map,
            readonly: true,
        });
    }
    Ok(Some(FetchCollectionByIdsResponse {
        call_id: None,
        collection_type: request.collection_type,
        result,
    }))
}

fn jukebox_disabled(request: &'static str) -> anyhow::Error {
    ExpectedRequestError::Unsupported {
        request,
        reason: UnsupportedReason::NotImplemented,
    }
    .into()
}

fn jukebox_error(error: SubsonicError, request: &'static str) -> anyhow::Error {
    match error {
        SubsonicError::Transport(_) => ExpectedRequestError::Unavailable {
            what: "Subsonic jukebox",
        }
        .into(),
        SubsonicError::HttpStatus(status) if status == reqwest::StatusCode::UNAUTHORIZED => {
            ExpectedRequestError::SubsonicNotAuthorized.into()
        }
        SubsonicError::HttpStatus(_) => ExpectedRequestError::Unavailable {
            what: "Subsonic jukebox",
        }
        .into(),
        SubsonicError::JukeboxUnsupported => jukebox_disabled(request),
        error => subsonic_request_error(error),
    }
}

pub(crate) fn jukebox_error_for_refresh(error: SubsonicError) -> anyhow::Error {
    jukebox_error(error, RefreshSessionRequest::wire_name())
}

async fn jukebox_status(binding: &SubsonicBinding) -> Result<subsonic::SubsonicJukeboxStatus> {
    binding
        .client
        .jukebox_status()
        .await
        .map_err(|error| jukebox_error(error, "Subsonic jukebox status"))
}

async fn jukebox_play_request(binding: &SubsonicBinding, _: &PlayRequest) -> Result<Option<()>> {
    binding
        .client
        .jukebox_start()
        .await
        .map_err(|error| jukebox_error(error, PlayRequest::wire_name()))?;
    Ok(Some(()))
}

async fn jukebox_pause_request(binding: &SubsonicBinding, _: &PauseRequest) -> Result<Option<()>> {
    binding
        .client
        .jukebox_stop()
        .await
        .map_err(|error| jukebox_error(error, PauseRequest::wire_name()))?;
    Ok(Some(()))
}

async fn jukebox_load_request(
    binding: &SubsonicBinding,
    request: &LoadRequest,
) -> Result<Option<()>> {
    let index = u32::try_from(request.index).map_err(|error| {
        ExpectedRequestError::invalid_input(format!("Invalid jukebox index: {error}"))
    })?;
    binding
        .client
        .jukebox_skip(index, 0)
        .await
        .map_err(|error| jukebox_error(error, LoadRequest::wire_name()))?;
    Ok(Some(()))
}

async fn jukebox_next_request(binding: &SubsonicBinding, _: &NextRequest) -> Result<Option<()>> {
    let queue = binding
        .client
        .jukebox_get()
        .await
        .map_err(|error| jukebox_error(error, NextRequest::wire_name()))?;
    let index = queue.status.current_index.unwrap_or(0).saturating_add(1);
    binding
        .client
        .jukebox_skip(index, 0)
        .await
        .map_err(|error| jukebox_error(error, NextRequest::wire_name()))?;
    Ok(Some(()))
}

async fn jukebox_previous_request(
    binding: &SubsonicBinding,
    _: &PreviousRequest,
) -> Result<Option<()>> {
    let queue = binding
        .client
        .jukebox_get()
        .await
        .map_err(|error| jukebox_error(error, PreviousRequest::wire_name()))?;
    let index = queue.status.current_index.unwrap_or(0).saturating_sub(1);
    binding
        .client
        .jukebox_skip(index, 0)
        .await
        .map_err(|error| jukebox_error(error, PreviousRequest::wire_name()))?;
    Ok(Some(()))
}

async fn jukebox_switch_request(
    binding: &SubsonicBinding,
    request: &SwitchRequest,
) -> Result<Option<()>> {
    binding
        .client
        .jukebox_skip(request.index, 0)
        .await
        .map_err(|error| jukebox_error(error, SwitchRequest::wire_name()))?;
    Ok(Some(()))
}

async fn jukebox_seek_request(
    binding: &SubsonicBinding,
    request: &SeekRequest,
) -> Result<Option<()>> {
    if !request.position_seconds.is_finite()
        || request.position_seconds < 0.0
        || request.position_seconds > f64::from(u32::MAX)
    {
        return Err(ExpectedRequestError::invalid_input("Invalid jukebox seek position").into());
    }
    let position = request.position_seconds as u32;
    let status = jukebox_status(binding).await?;
    binding
        .client
        .jukebox_skip(status.current_index.unwrap_or(0), position)
        .await
        .map_err(|error| jukebox_error(error, SeekRequest::wire_name()))?;
    Ok(Some(()))
}

async fn jukebox_volume_request(
    binding: &SubsonicBinding,
    request: &VolumeRequest,
) -> Result<Option<VolumeResponse>> {
    binding
        .client
        .jukebox_set_gain(request.volume)
        .await
        .map_err(|error| jukebox_error(error, VolumeRequest::wire_name()))?;
    Ok(Some(VolumeResponse {
        volume: request.volume,
        call_id: request.call_id,
    }))
}

async fn jukebox_playback_mode_request(
    binding: &SubsonicBinding,
    request: &SetPlaybackModeRequest,
) -> Result<Option<()>> {
    if request.mode != 3 {
        return Err(ExpectedRequestError::Unsupported {
            request: SetPlaybackModeRequest::wire_name(),
            reason: UnsupportedReason::NotImplemented,
        }
        .into());
    }
    binding
        .client
        .jukebox_shuffle()
        .await
        .map_err(|error| jukebox_error(error, SetPlaybackModeRequest::wire_name()))?;
    Ok(Some(()))
}

async fn jukebox_remove_request(
    binding: &SubsonicBinding,
    request: &RemoveRequest,
) -> Result<Option<()>> {
    binding
        .client
        .jukebox_remove(request.index)
        .await
        .map_err(|error| jukebox_error(error, RemoveRequest::wire_name()))?;
    Ok(Some(()))
}

fn subsonic_track_ids(
    binding: &SubsonicBinding,
    request: &LoadResolvedItemsRequest,
) -> Result<Vec<String>> {
    request
        .items
        .iter()
        .map(|item| {
            let PlayingItemRef::LibraryTrack { library_id, track } = item else {
                return Err(ExpectedRequestError::invalid_input(
                    "Subsonic jukebox accepts only Subsonic library items",
                )
                .into());
            };
            let library_id = Uuid::parse_str(library_id).map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid Subsonic library id: {error}"))
            })?;
            if library_id != binding.library_id {
                return Err(ExpectedRequestError::invalid_input(
                    "Subsonic jukebox item belongs to another library",
                )
                .into());
            }
            Ok(track.clone())
        })
        .collect()
}

async fn jukebox_load_resolved_items_request(
    binding: &SubsonicBinding,
    request: &LoadResolvedItemsRequest,
) -> Result<Option<LoadResolvedItemsResponse>> {
    let ids = subsonic_track_ids(binding, request)?;
    let references = ids.iter().map(String::as_str).collect::<Vec<_>>();
    let result = match request.mode {
        PlaylistOperateMode::Replace if references.is_empty() => {
            binding.client.jukebox_clear().await
        }
        PlaylistOperateMode::Replace => binding.client.jukebox_set(&references).await,
        PlaylistOperateMode::AppendToEnd | PlaylistOperateMode::PlayNext => {
            binding.client.jukebox_add(&references).await
        }
    };
    result.map_err(|error| jukebox_error(error, LoadResolvedItemsRequest::wire_name()))?;
    Ok(Some(LoadResolvedItemsResponse {
        call_id: request.call_id,
    }))
}

async fn jukebox_refresh_session_request(
    binding: &SubsonicBinding,
    _: &RefreshSessionRequest,
) -> Result<Option<()>> {
    binding.publish_jukebox_snapshot().await?;
    Ok(Some(()))
}

async fn ensure_artists(binding: &SubsonicBinding) -> Result<()> {
    let loaded = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?
        .artists_loaded;
    if loaded {
        return Ok(());
    }
    let artists = binding
        .client
        .artists()
        .await
        .map_err(subsonic_request_error)?;
    let mut catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    for artist in artists {
        let id = catalog.ids.intern(SubsonicIdKind::Artist, &artist.id)?;
        catalog.artists.insert(id, artist);
    }
    catalog.artists_loaded = true;
    Ok(())
}

async fn ensure_albums(binding: &SubsonicBinding) -> Result<()> {
    let loaded = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?
        .albums_loaded;
    if loaded {
        return Ok(());
    }
    let mut offset = 0_u32;
    loop {
        let albums = binding
            .client
            .album_list(500, offset)
            .await
            .map_err(subsonic_request_error)?;
        let count = u32::try_from(albums.len()).unwrap_or(u32::MAX);
        let mut catalog = binding
            .catalog
            .lock()
            .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
        for album in albums {
            let id = catalog.ids.intern(SubsonicIdKind::Album, &album.id)?;
            catalog.albums.insert(id, album);
        }
        if count < 500 {
            catalog.albums_loaded = true;
            return Ok(());
        }
        drop(catalog);
        offset = offset.checked_add(count).ok_or_else(|| {
            ExpectedRequestError::invalid_input("Subsonic album list is too large")
        })?;
    }
}

fn collection_groups(
    binding: &SubsonicBinding,
    collection_type: CollectionType,
    group_titles: &[String],
) -> Result<Vec<CollectionGroup>> {
    let catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    let entries: Vec<(i32, String, String, Option<String>)> = match collection_type {
        CollectionType::Artist => catalog
            .artists
            .iter()
            .map(|(id, artist)| {
                (
                    *id,
                    artist.name.clone(),
                    generate_group_name(&artist.name),
                    artist.cover_art.clone(),
                )
            })
            .collect(),
        CollectionType::Album => catalog
            .albums
            .iter()
            .map(|(id, album)| {
                (
                    *id,
                    album.name.clone(),
                    generate_group_name(&album.name),
                    album.cover_art.clone(),
                )
            })
            .collect(),
        _ => {
            return Err(ExpectedRequestError::Unsupported {
                request: FetchCollectionGroupsRequest::wire_name(),
                reason: UnsupportedReason::NotImplemented,
            }
            .into());
        }
    };
    Ok(group_titles
        .iter()
        .map(|title| {
            let mut collections: Vec<_> = entries
                .iter()
                .filter(|(_, _, group, _)| group == title)
                .map(|(id, name, _, _)| Collection {
                    id: *id,
                    name: name.clone(),
                    queries: Vec::new(),
                    collection_type,
                    cover_art_map: Default::default(),
                    readonly: true,
                })
                .collect();
            collections.sort_by(|left, right| left.name.cmp(&right.name));
            for collection in &mut collections {
                let cover = entries
                    .iter()
                    .find(|(id, _, _, _)| *id == collection.id)
                    .and_then(|(_, _, _, cover)| cover.clone());
                collection.cover_art_map = subsonic_cover_map(
                    binding,
                    cover
                        .map(|cover| vec![(collection.id, cover)])
                        .unwrap_or_default(),
                );
            }
            CollectionGroup {
                group_title: title.clone(),
                collections,
            }
        })
        .collect())
}

pub(crate) async fn fetch_collection_group_summary_request(
    binding: &SubsonicBinding,
    request: &FetchCollectionGroupSummaryRequest,
) -> Result<Option<CollectionGroupSummaryResponse>> {
    match request.collection_type {
        CollectionType::Artist => ensure_artists(binding).await?,
        CollectionType::Album => ensure_albums(binding).await?,
        _ => {
            return Err(ExpectedRequestError::Unsupported {
                request: FetchCollectionGroupSummaryRequest::wire_name(),
                reason: UnsupportedReason::NotImplemented,
            }
            .into());
        }
    }
    let catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    let names = match request.collection_type {
        CollectionType::Artist => catalog
            .artists
            .values()
            .map(|item| item.name.as_str())
            .collect(),
        CollectionType::Album => catalog
            .albums
            .values()
            .map(|item| item.name.as_str())
            .collect(),
        _ => Vec::new(),
    };
    let mut groups = BTreeMap::<String, i32>::new();
    for name in names {
        *groups.entry(generate_group_name(name)).or_default() += 1;
    }
    Ok(Some(CollectionGroupSummaryResponse {
        call_id: None,
        collection_type: request.collection_type,
        groups: groups
            .into_iter()
            .map(|(group_title, count)| CollectionGroupSummary { group_title, count })
            .collect(),
    }))
}

pub(crate) async fn fetch_collection_groups_request(
    binding: &SubsonicBinding,
    request: &FetchCollectionGroupsRequest,
) -> Result<Option<FetchCollectionGroupsResponse>> {
    match request.collection_type {
        CollectionType::Artist => ensure_artists(binding).await?,
        CollectionType::Album => ensure_albums(binding).await?,
        _ => {
            return Err(ExpectedRequestError::Unsupported {
                request: FetchCollectionGroupsRequest::wire_name(),
                reason: UnsupportedReason::NotImplemented,
            }
            .into());
        }
    }
    Ok(Some(FetchCollectionGroupsResponse {
        call_id: None,
        groups: collection_groups(binding, request.collection_type, &request.group_titles)?,
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DetailQuery {
    Artist(i32),
    Album(i32),
}

fn parse_detail_query(request: &MixQueryRequest) -> Result<DetailQuery> {
    let [query] = request.queries.as_slice() else {
        return Err(unsupported_detail_query().into());
    };
    let operator = parse_query(&(query.operator.clone(), query.parameter.clone()));
    match operator {
        QueryOperator::LibArtist(id) => Ok(DetailQuery::Artist(id)),
        QueryOperator::LibAlbum(id) => Ok(DetailQuery::Album(id)),
        _ => Err(unsupported_detail_query().into()),
    }
}

fn unsupported_detail_query() -> ExpectedRequestError {
    ExpectedRequestError::Unsupported {
        request: MixQueryRequest::wire_name(),
        reason: UnsupportedReason::NotImplemented,
    }
}

async fn detail_songs(binding: &SubsonicBinding, query: DetailQuery) -> Result<Vec<SubsonicSong>> {
    let albums = match query {
        DetailQuery::Album(id) => vec![id],
        DetailQuery::Artist(id) => {
            let remote_id = {
                let catalog = binding
                    .catalog
                    .lock()
                    .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
                catalog.ids.resolve(SubsonicIdKind::Artist, id)?.to_owned()
            };
            let artist = binding
                .client
                .artist(&remote_id)
                .await
                .map_err(subsonic_request_error)?;
            let mut catalog = binding
                .catalog
                .lock()
                .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
            let mut albums = Vec::with_capacity(artist.albums.len());
            for album in artist.albums {
                let id = catalog.ids.intern(SubsonicIdKind::Album, &album.id)?;
                catalog.albums.insert(id, album);
                albums.push(id);
            }
            albums
        }
    };
    let mut songs = Vec::new();
    for album_id in albums {
        let remote_id = {
            let catalog = binding
                .catalog
                .lock()
                .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
            catalog
                .ids
                .resolve(SubsonicIdKind::Album, album_id)?
                .to_owned()
        };
        let mut album = binding
            .client
            .album(&remote_id)
            .await
            .map_err(subsonic_request_error)?;
        album.songs.sort_by_key(|song| {
            (
                song.disc.unwrap_or_default(),
                song.track.unwrap_or_default(),
            )
        });
        let mut catalog = binding
            .catalog
            .lock()
            .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
        for song in album.songs {
            let id = catalog.ids.intern(SubsonicIdKind::Track, &song.id)?;
            catalog.songs.insert(id, song.clone());
            songs.push(song);
        }
    }
    Ok(songs)
}

pub(crate) async fn mix_query_request(
    binding: &SubsonicBinding,
    request: &MixQueryRequest,
) -> Result<Option<MixQueryResponse>> {
    let query = parse_detail_query(request)?;
    let songs = detail_songs(binding, query).await?;
    let mut catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    let translated = songs
        .into_iter()
        .skip(usize::try_from(request.cursor.max(0)).unwrap_or(usize::MAX))
        .take(usize::try_from(request.page_size.max(0)).unwrap_or(usize::MAX))
        .map(|song| {
            let id = catalog.ids.intern(SubsonicIdKind::Track, &song.id)?;
            catalog.songs.insert(id, song.clone());
            let cover_art_id = song
                .cover_art
                .as_deref()
                .map(|cover| catalog.ids.intern(SubsonicIdKind::CoverArt, cover))
                .transpose()?
                .unwrap_or_default();
            let media_file = MediaFile {
                id,
                library_id: binding.library_id.to_string(),
                track: song.id.clone(),
                path: String::new(),
                artist: song.artist.unwrap_or_default(),
                album: song.album.unwrap_or_default(),
                title: song.title,
                duration: f64::from(song.duration.unwrap_or_default()),
                cover_art_id,
                track_number: song
                    .track
                    .and_then(|value| i32::try_from(value).ok())
                    .unwrap_or_default(),
                audio_availability: AudioAvailability::Unavailable,
                audio_origin: AudioOrigin::Remote,
            };
            Ok((media_file, song.cover_art))
        })
        .collect::<Result<Vec<_>>>()?;
    drop(catalog);
    let covers = translated
        .iter()
        .filter_map(|(file, cover)| cover.clone().map(|cover| (file.id, cover)))
        .collect();
    let files = translated.into_iter().map(|(file, _)| file).collect();
    Ok(Some(MixQueryResponse {
        call_id: None,
        files,
        cover_art_map: subsonic_cover_map(binding, covers),
    }))
}

fn subsonic_request_error(error: SubsonicError) -> anyhow::Error {
    match error {
        SubsonicError::Transport(_) | SubsonicError::HttpStatus(_) => {
            ExpectedRequestError::Unavailable {
                what: "Subsonic server",
            }
            .into()
        }
        SubsonicError::Malformed(_) => ExpectedRequestError::UnexpectedResponse {
            expected: "valid Subsonic response".into(),
            actual: "malformed response".into(),
        }
        .into(),
        SubsonicError::Server {
            kind:
                SubsonicServerError::WrongCredentials
                | SubsonicServerError::NotAuthorized
                | SubsonicServerError::InvalidApiKey,
            ..
        } => ExpectedRequestError::SubsonicNotAuthorized.into(),
        SubsonicError::Server {
            kind:
                SubsonicServerError::Generic
                | SubsonicServerError::MissingParameter
                | SubsonicServerError::ClientTooOld
                | SubsonicServerError::ServerTooOld
                | SubsonicServerError::TokenAuthenticationNotSupported
                | SubsonicServerError::AuthenticationMechanismNotSupported
                | SubsonicServerError::ConflictingAuthentication
                | SubsonicServerError::NotFound
                | SubsonicServerError::Unknown(_),
            ..
        } => ExpectedRequestError::UnexpectedResponse {
            expected: "successful Subsonic request".into(),
            actual: "server rejected the request".into(),
        }
        .into(),
        SubsonicError::JukeboxUnsupported => ExpectedRequestError::Unsupported {
            request: "Subsonic jukebox request",
            reason: UnsupportedReason::NotImplemented,
        }
        .into(),
        SubsonicError::InvalidJukeboxGain => {
            ExpectedRequestError::invalid_input("invalid Subsonic jukebox gain").into()
        }
    }
}

macro_rules! subsonic_requests {
    (
        supported { $($request:ty => $handler:ident as $method:ident),* $(,)? }
        jukebox_supported { $($jukebox_request:ty => $jukebox_handler:ident as $jukebox_method:ident),* $(,)? }
        player { $($player_request:ty),* $(,)? }
    ) => {
        impl RequestTranslator for SubsonicBinding {
            $(
                fn $method<'a>(
                    &'a self,
                    request: &'a $request,
                ) -> RequestTranslationFuture<'a, <$request as crate::TypedRequest>::Response> {
                    Box::pin($handler(self, request))
                }
            )*
            $(
                fn $jukebox_method<'a>(
                    &'a self,
                    request: &'a $jukebox_request,
                ) -> RequestTranslationFuture<'a, <$jukebox_request as crate::TypedRequest>::Response> {
                    Box::pin($jukebox_handler(self, request))
                }
            )*
        }

        impl RequestSlots {
            pub(crate) fn subsonic(probe: &subsonic::SubsonicProbe) -> Self {
                let mut slots = Self::unimplemented();
                let player_reason = if probe.roles_known && probe.roles.jukebox_role {
                    UnsupportedReason::NotImplemented
                } else {
                    UnsupportedReason::AccountRoleMissing
                };
                $(slots.set::<$player_request>(Slot::Unsupported { reason: player_reason });)*
                if probe.roles_known && probe.roles.jukebox_role {
                    $(slots.set::<$jukebox_request>(Slot::Translate);)*
                }
                $(slots.set::<$request>(Slot::Translate);)*
                slots
            }
        }
    };
}

subsonic_requests! {
    supported {
        FetchAllPlaylistsRequest => fetch_all_playlists_request as translate_fetch_all_playlists_request,
        GetPlaylistByIdRequest => get_playlist_by_id_request as translate_get_playlist_by_id_request,
        CreatePlaylistRequest => create_playlist_request as translate_create_playlist_request,
        UpdatePlaylistRequest => update_playlist_request as translate_update_playlist_request,
        RemovePlaylistRequest => remove_playlist_request as translate_remove_playlist_request,
        AddItemToPlaylistRequest => add_item_to_playlist_request as translate_add_item_to_playlist_request,
        RemoveItemFromPlaylistRequest => remove_item_from_playlist_request as translate_remove_item_from_playlist_request,
        ReorderPlaylistItemPositionRequest => reorder_playlist_item_position_request as translate_reorder_playlist_item_position_request,
        FetchMediaFileByIdsRequest => fetch_media_file_by_ids_request as translate_fetch_media_file_by_ids_request,
        FetchCollectionGroupSummaryRequest => fetch_collection_group_summary_request as translate_fetch_collection_group_summary_request,
        FetchCollectionGroupsRequest => fetch_collection_groups_request as translate_fetch_collection_groups_request,
        FetchCollectionByIdsRequest => fetch_collection_by_ids_request as translate_fetch_collection_by_ids_request,
        MixQueryRequest => mix_query_request as translate_mix_query_request,
        SearchForRequest => search_for_request as translate_search_for_request,
    }
    jukebox_supported {
        VolumeRequest => jukebox_volume_request as translate_volume_request,
        SetPlaybackModeRequest => jukebox_playback_mode_request as translate_set_playback_mode_request,
        LoadRequest => jukebox_load_request as translate_load_request,
        PlayRequest => jukebox_play_request as translate_play_request,
        PauseRequest => jukebox_pause_request as translate_pause_request,
        NextRequest => jukebox_next_request as translate_next_request,
        PreviousRequest => jukebox_previous_request as translate_previous_request,
        SwitchRequest => jukebox_switch_request as translate_switch_request,
        SeekRequest => jukebox_seek_request as translate_seek_request,
        RemoveRequest => jukebox_remove_request as translate_remove_request,
        LoadResolvedItemsRequest => jukebox_load_resolved_items_request as translate_load_resolved_items_request,
        RefreshSessionRequest => jukebox_refresh_session_request as translate_refresh_session_request,
    }
    player {
        VolumeRequest,
        LoadRequest,
        PlayRequest,
        PauseRequest,
        NextRequest,
        PreviousRequest,
        SwitchRequest,
        SeekRequest,
        RemoveRequest,
        SetPlaybackModeRequest,
        MovePlaylistItemRequest,
        SetRealtimeFFTEnabledRequest,
        SetAdaptiveSwitchingEnabledRequest,
        LoadResolvedItemsRequest,
        HandoffLoadRequest,
        TakeBackHandoffBlockRequest,
        CastPreflightRequest,
        RefreshSessionRequest,
        SessionSnapshotRequest,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        net::SocketAddr,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use axum::{
        Router,
        body::Body,
        extract::{Path, Query, RawQuery, State},
        http::{Response, StatusCode},
        routing::get,
    };
    use database::library_registry::SubsonicOpenMode;
    use playback::player::PlayingItem;
    use subsonic::{
        SubsonicClient, SubsonicCredentials, SubsonicError, SubsonicProbe, SubsonicRoles,
    };
    use tempfile::TempDir;
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;
    use uuid::Uuid;

    use super::{
        add_item_to_playlist_request, create_playlist_request, fetch_all_playlists_request,
        fetch_collection_group_summary_request, fetch_collection_groups_request,
        fetch_media_file_by_ids_request, fetch_remote_playlist, get_playlist_by_id_request,
        jukebox_error, remove_item_from_playlist_request, remove_playlist_request,
        reorder_playlist_item_position_request, search_for_request, subsonic_request_error,
        update_playlist_request,
    };
    use crate::{
        TypedRequest,
        macros::{RequestSlots, RequestTranslator},
        messages::{
            AddItemToPlaylistRequest, CastPreflightRequest, CollectionType,
            CreateM3u8PlaylistRequest, CreatePlaylistRequest, FetchAllPlaylistsRequest,
            FetchCollectionGroupSummaryRequest, FetchCollectionGroupsRequest,
            FetchMediaFileByIdsRequest, FetchPlaylistOccurrencesRequest, GetPlaylistByIdRequest,
            HandoffLoadRequest, LoadRequest, LoadResolvedItemsRequest, MovePlaylistItemRequest,
            NextRequest, PauseRequest, PlayRequest, PlayingItemRef, PlaylistOperateMode,
            PreviousRequest, RefreshSessionRequest, RemoveItemFromPlaylistRequest,
            RemovePlaylistRequest, RemoveRequest, ReorderPlaylistItemPositionRequest,
            SearchForRequest, SeekRequest, SessionSnapshotRequest,
            SetAdaptiveSwitchingEnabledRequest, SetPlaybackModeRequest,
            SetRealtimeFFTEnabledRequest, SwitchRequest, TakeBackHandoffBlockRequest,
            UpdatePlaylistRequest, VolumeRequest,
        },
        request_error::{ExpectedRequestError, classify},
        request_owner::Implementation,
        rpc::{Slot, TargetRequest, UnsupportedReason},
        subsonic_binding::SubsonicBinding,
        utils::{Broadcaster, RinfRustSignal},
    };

    use crate::messages::RequestErrorKind;

    #[derive(Clone)]
    struct MockState {
        cover_calls: Arc<AtomicUsize>,
        requests: Arc<Mutex<Vec<String>>>,
        jukebox: Arc<Mutex<MockJukebox>>,
        jukebox_calls: Arc<Mutex<Vec<HashMap<String, String>>>>,
        scrobble_calls: Arc<Mutex<Vec<HashMap<String, String>>>>,
        reject_scrobble: Arc<AtomicUsize>,
        playlists: Arc<Mutex<Vec<MockPlaylist>>>,
        playlist_calls: MockPlaylistCalls,
    }

    type MockPlaylistCalls = Arc<Mutex<Vec<(String, Vec<(String, String)>)>>>;

    #[derive(Clone)]
    struct MockPlaylist {
        id: String,
        name: String,
        songs: Vec<String>,
    }

    struct MockJukebox {
        entries: Vec<serde_json::Value>,
        current_index: Option<u32>,
        playing: bool,
        gain: f32,
        position: u32,
    }

    impl Default for MockJukebox {
        fn default() -> Self {
            Self {
                entries: vec![mock_song("track-a"), mock_song("track-b")],
                current_index: Some(0),
                playing: false,
                gain: 1.0,
                position: 0,
            }
        }
    }

    struct MockServer {
        url: String,
        state: MockState,
        broadcasts: Arc<AtomicUsize>,
        _task: JoinHandle<()>,
    }

    struct CountingBroadcaster(Arc<AtomicUsize>);

    impl Broadcaster for CountingBroadcaster {
        fn broadcast(&self, _: &dyn RinfRustSignal) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    async fn mock_subsonic(root: &TempDir, jukebox_role: bool) -> (SubsonicBinding, MockServer) {
        mock_subsonic_mode(root, jukebox_role, SubsonicOpenMode::RemoteControl).await
    }

    async fn mock_subsonic_mode(
        root: &TempDir,
        jukebox_role: bool,
        mode: SubsonicOpenMode,
    ) -> (SubsonicBinding, MockServer) {
        let state = MockState {
            cover_calls: Arc::new(AtomicUsize::new(0)),
            requests: Arc::new(Mutex::new(Vec::new())),
            jukebox: Arc::new(Mutex::new(MockJukebox::default())),
            jukebox_calls: Arc::new(Mutex::new(Vec::new())),
            playlists: Arc::new(Mutex::new(vec![MockPlaylist {
                id: "playlist-1".into(),
                name: "Road".into(),
                songs: vec!["song-1".into(), "song-2".into(), "song-1".into()],
            }])),
            playlist_calls: Arc::new(Mutex::new(Vec::new())),
            scrobble_calls: Arc::new(Mutex::new(Vec::new())),
            reject_scrobble: Arc::new(AtomicUsize::new(0)),
        };
        let app = Router::new()
            .route("/rest/{*endpoint}", get(mock_endpoint))
            .with_state(state.clone());
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let url = format!("http://{address}");
        let client = SubsonicClient::new(
            url::Url::parse(&url).unwrap(),
            SubsonicCredentials::Password {
                user: "alice".into(),
                password: "cover-secret-password".into(),
            },
            reqwest::Client::new(),
        );
        let broadcasts = Arc::new(AtomicUsize::new(0));
        let binding = SubsonicBinding::new(
            client,
            SubsonicProbe {
                api_version: "1.16.1".into(),
                open_subsonic: false,
                server_type: None,
                server_version: None,
                extensions: Vec::new(),
                roles: SubsonicRoles {
                    jukebox_role,
                    ..SubsonicRoles::default()
                },
                roles_known: true,
            },
            root.path().to_path_buf(),
            Uuid::new_v4(),
            mode == SubsonicOpenMode::PlayOnThisDevice,
            Arc::new(CountingBroadcaster(Arc::clone(&broadcasts))),
        );
        (
            binding,
            MockServer {
                url,
                state,
                broadcasts,
                _task: task,
            },
        )
    }

    async fn mock_endpoint(
        State(state): State<MockState>,
        Path(endpoint): Path<String>,
        Query(query): Query<HashMap<String, String>>,
        RawQuery(raw_query): RawQuery,
    ) -> Response<Body> {
        let endpoint = endpoint.trim_end_matches(".view").to_owned();
        let params = raw_query
            .as_deref()
            .map(|query| {
                url::form_urlencoded::parse(query.as_bytes())
                    .map(|(key, value)| (key.into_owned(), value.into_owned()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let cover_id = query.get("id").cloned().unwrap_or_default();
        state.requests.lock().unwrap().push(endpoint.clone());
        if endpoint == "jukeboxControl" {
            state.jukebox_calls.lock().unwrap().push(query.clone());
            let mut jukebox = state.jukebox.lock().unwrap();
            let inner = match query.get("action").map(String::as_str) {
                Some("get") | Some("status") => {
                    let status = serde_json::json!({
                        "currentIndex": jukebox.current_index,
                        "playing": jukebox.playing,
                        "gain": jukebox.gain,
                        "position": jukebox.position,
                    });
                    if query.get("action").is_some_and(|action| action == "get") {
                        serde_json::json!({"jukeboxPlaylist": {
                            "entry": jukebox.entries.clone(),
                            "currentIndex": jukebox.current_index,
                            "playing": jukebox.playing,
                            "gain": jukebox.gain,
                            "position": jukebox.position,
                        }})
                    } else {
                        serde_json::json!({"jukeboxStatus": status})
                    }
                }
                Some("set") => {
                    jukebox.entries = vec![mock_song(
                        query.get("id").map(String::as_str).unwrap_or_default(),
                    )];
                    jukebox.current_index = Some(0);
                    serde_json::json!({})
                }
                Some("add") => {
                    jukebox.entries.push(mock_song(
                        query.get("id").map(String::as_str).unwrap_or_default(),
                    ));
                    serde_json::json!({})
                }
                Some("remove") => {
                    if let Some(index) = query
                        .get("index")
                        .and_then(|value| value.parse::<usize>().ok())
                        && index < jukebox.entries.len()
                    {
                        jukebox.entries.remove(index);
                    }
                    serde_json::json!({})
                }
                Some("clear") => {
                    jukebox.entries.clear();
                    jukebox.current_index = None;
                    serde_json::json!({})
                }
                Some("skip") => {
                    jukebox.current_index = query.get("index").and_then(|value| value.parse().ok());
                    jukebox.position = query
                        .get("offset")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or_default();
                    serde_json::json!({})
                }
                Some("start") => {
                    jukebox.playing = true;
                    serde_json::json!({})
                }
                Some("stop") => {
                    jukebox.playing = false;
                    serde_json::json!({})
                }
                Some("setGain") => {
                    jukebox.gain = query
                        .get("gain")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or_default();
                    serde_json::json!({})
                }
                Some("shuffle") => {
                    jukebox.entries.reverse();
                    serde_json::json!({})
                }
                _ => serde_json::json!({}),
            };
            return mock_response(inner);
        }
        if endpoint == "scrobble" {
            state.scrobble_calls.lock().unwrap().push(query);
            if state.reject_scrobble.load(Ordering::SeqCst) != 0 {
                return Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":50,"message":"account is no longer authorized"}}}"#,
                    ))
                    .unwrap();
            }
        }
        if endpoint == "getCoverArt" {
            state.cover_calls.fetch_add(1, Ordering::SeqCst);
            if cover_id == "error-cover" {
                return Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::from("cover-secret-password"))
                    .unwrap();
            }
            return Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "image/jpeg")
                .body(Body::from(format!("image-{cover_id}")))
                .unwrap();
        }
        if matches!(
            endpoint.as_str(),
            "getPlaylists" | "getPlaylist" | "createPlaylist" | "updatePlaylist" | "deletePlaylist"
        ) {
            state
                .playlist_calls
                .lock()
                .unwrap()
                .push((endpoint.clone(), params.clone()));
            let mut playlists = state.playlists.lock().unwrap();
            let get = |key: &str| {
                params
                    .iter()
                    .rev()
                    .find(|(name, _)| name == key)
                    .map(|(_, value)| value.as_str())
            };
            let inner = match endpoint.as_str() {
                "getPlaylists" => serde_json::json!({"playlists":{"playlist":playlists.iter().map(|playlist| serde_json::json!({"id":playlist.id,"name":playlist.name,"songCount":playlist.songs.len()})).collect::<Vec<_>>()}}),
                "getPlaylist" => playlists.iter().find(|playlist| Some(playlist.id.as_str()) == get("id")).map(|playlist| serde_json::json!({"playlist":{"id":playlist.id,"name":playlist.name,"entry":playlist.songs.iter().map(|id|mock_song(id)).collect::<Vec<_>>()}})).unwrap_or_else(|| serde_json::json!({})),
                "createPlaylist" => {
                    let songs = params.iter().filter(|(key, _)| key == "songId").map(|(_, value)| value.clone()).collect::<Vec<_>>();
                    if let Some(id) = get("playlistId") {
                        if let Some(playlist) = playlists.iter_mut().find(|playlist| playlist.id == id) { playlist.songs = songs; }
                    } else {
                        let id = format!("playlist-{}", playlists.len() + 1);
                        playlists.push(MockPlaylist { id, name: get("name").unwrap_or_default().into(), songs });
                    }
                    serde_json::json!({})
                }
                "updatePlaylist" => {
                    if let Some(playlist) = playlists.iter_mut().find(|playlist| Some(playlist.id.as_str()) == get("playlistId")) {
                        if let Some(name) = get("name") { playlist.name = name.into(); }
                        for (_, song) in params.iter().filter(|(key, _)| key == "songIdToAdd") { playlist.songs.push(song.clone()); }
                        let mut indexes = params.iter().filter(|(key, _)| key == "songIndexToRemove").filter_map(|(_, value)| value.parse::<usize>().ok()).collect::<Vec<_>>();
                        indexes.sort_unstable_by(|left, right| right.cmp(left));
                        for index in indexes { if index < playlist.songs.len() { playlist.songs.remove(index); } }
                    }
                    serde_json::json!({})
                }
                "deletePlaylist" => { playlists.retain(|playlist| Some(playlist.id.as_str()) != get("id")); serde_json::json!({}) }
                _ => serde_json::json!({}),
            };
            return mock_response(inner);
        }
        let inner = match endpoint.as_str() {
            "search3" => serde_json::json!({"searchResult3": {
                "song": [
                    {"id":"song-1","title":"One","coverArt":"cover-1"},
                    {"id":"song-2","title":"Two","coverArt":"cover-2"}
                ]
            }}),
            "getArtists" => serde_json::json!({"artists":{"index":{
                "name":"A","artist":[
                    {"id":"artist-1","name":"Artist One","coverArt":"cover-1"},
                    {"id":"artist-2","name":"Artist Two","coverArt":"cover-2"}
                ]
            }}}),
            _ => serde_json::json!({}),
        };
        mock_response(inner)
    }

    fn mock_song(id: &str) -> serde_json::Value {
        serde_json::json!({"id":id,"title":id,"artist":"Artist","album":"Album","duration":180})
    }

    fn mock_response(inner: serde_json::Value) -> Response<Body> {
        let mut envelope = serde_json::json!({"subsonic-response":{
            "status":"ok","version":"1.16.1"
        }});
        envelope["subsonic-response"]
            .as_object_mut()
            .unwrap()
            .extend(inner.as_object().unwrap().clone());
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(Body::from(envelope.to_string()))
            .unwrap()
    }

    async fn wait_for_covers(binding: &SubsonicBinding, cover_ids: &[&str]) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if cover_ids
                    .iter()
                    .all(|cover| binding.covers.path_for(cover).is_file())
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn searched_and_browsed_covers_are_cached_locally_and_reused() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        let search = search_for_request(
            &binding,
            &SearchForRequest {
                call_id: Some(1),
                query_str: "song".into(),
                fields: vec!["track".into()],
                n: 10,
            },
        )
        .await
        .unwrap()
        .unwrap();
        let fetch = |ids| FetchMediaFileByIdsRequest {
            call_id: None,
            ids,
            bake_cover_arts: true,
        };
        let first = fetch_media_file_by_ids_request(&binding, &fetch(search.tracks.clone()))
            .await
            .unwrap()
            .unwrap();
        assert!(first.cover_art_map.is_empty());
        assert!(first.media_files.iter().all(|file| file.cover_art_id > 0));
        wait_for_covers(&binding, &["cover-1", "cover-2"]).await;
        let second = fetch_media_file_by_ids_request(&binding, &fetch(search.tracks))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.cover_art_map.len(), 2);
        assert!(second.cover_art_map.values().all(|path| {
            path.starts_with(root.path().to_str().unwrap()) && !path.starts_with("http")
        }));
        assert_secret_free(&serde_json::to_string(&second).unwrap());
        let summary = FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
        };
        fetch_collection_group_summary_request(&binding, &summary)
            .await
            .unwrap();
        let groups = fetch_collection_groups_request(
            &binding,
            &FetchCollectionGroupsRequest {
                call_id: None,
                collection_type: CollectionType::Artist,
                bake_cover_arts: true,
                group_titles: vec!["A".into()],
            },
        )
        .await
        .unwrap()
        .unwrap();
        let group = groups.groups.first().unwrap();
        assert_eq!(group.collections.len(), 2);
        assert!(
            group
                .collections
                .iter()
                .all(|collection| collection.cover_art_map.len() == 1)
        );
        assert_eq!(server.state.cover_calls.load(Ordering::SeqCst), 2);
        let error = binding
            .client
            .cover_art("error-cover", None)
            .await
            .unwrap_err();
        let error = format!("{error:?}");
        for secret in ["t=", "s=", "p=", "cover-secret-password"] {
            assert!(!error.contains(secret));
        }
        assert!(!server.url.contains("cover-secret-password"));
    }

    #[tokio::test]
    async fn playlist_translations_mutate_remote_state_and_reject_unknown_track_before_http() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        let search = search_for_request(
            &binding,
            &SearchForRequest {
                call_id: None,
                query_str: "song".into(),
                fields: vec!["track".into()],
                n: 10,
            },
        )
        .await
        .unwrap()
        .unwrap();
        let playlists =
            fetch_all_playlists_request(&binding, &FetchAllPlaylistsRequest { call_id: None })
                .await
                .unwrap()
                .unwrap();
        let playlist_id = playlists.playlists[0].id;
        assert_eq!(playlists.playlists[0].name, "Road");
        assert_eq!(
            get_playlist_by_id_request(
                &binding,
                &GetPlaylistByIdRequest {
                    playlist_id,
                    call_id: None
                }
            )
            .await
            .unwrap()
            .unwrap()
            .playlist
            .name,
            "Road"
        );

        update_playlist_request(
            &binding,
            &UpdatePlaylistRequest {
                playlist_id,
                name: "Renamed".into(),
                group: String::new(),
                call_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            fetch_remote_playlist(&binding, playlist_id)
                .await
                .unwrap()
                .1,
            ["song-1", "song-2", "song-1"]
        );

        let call_count = server.state.playlist_calls.lock().unwrap().len();
        let error = match add_item_to_playlist_request(
            &binding,
            &AddItemToPlaylistRequest {
                playlist_id,
                media_file_id: i32::MAX,
                position: None,
                call_id: None,
            },
        )
        .await
        {
            Err(error) => error,
            Ok(_) => panic!("unknown track ID must be rejected"),
        };
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::InvalidInput { .. })
        ));
        assert_eq!(
            server.state.playlist_calls.lock().unwrap().len(),
            call_count
        );

        let added = add_item_to_playlist_request(
            &binding,
            &AddItemToPlaylistRequest {
                playlist_id,
                media_file_id: search.tracks[0],
                position: None,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(added.occurrence_id, "3");
        assert_eq!(
            fetch_remote_playlist(&binding, playlist_id)
                .await
                .unwrap()
                .1
                .len(),
            4
        );

        remove_item_from_playlist_request(
            &binding,
            &RemoveItemFromPlaylistRequest {
                playlist_id,
                occurrence_id: added.occurrence_id,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            fetch_remote_playlist(&binding, playlist_id)
                .await
                .unwrap()
                .1,
            ["song-1", "song-2", "song-1"]
        );

        reorder_playlist_item_position_request(
            &binding,
            &ReorderPlaylistItemPositionRequest {
                playlist_id,
                occurrence_id: "0".into(),
                new_position: 2,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        let reordered = fetch_remote_playlist(&binding, playlist_id)
            .await
            .unwrap()
            .1;
        assert_eq!(reordered, ["song-2", "song-1", "song-1"]);
        let replace_call = server
            .state
            .playlist_calls
            .lock()
            .unwrap()
            .iter()
            .find(|(name, params)| {
                name == "createPlaylist" && params.iter().any(|(key, _)| key == "playlistId")
            })
            .cloned()
            .unwrap();
        let replaced_ids = replace_call
            .1
            .iter()
            .filter(|(key, _)| key == "songId")
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(replaced_ids, ["song-2", "song-1", "song-1"]);

        remove_playlist_request(
            &binding,
            &RemovePlaylistRequest {
                playlist_id,
                call_id: None,
            },
        )
        .await
        .unwrap();
        assert!(binding.client.playlists().await.unwrap().is_empty());
        let created = create_playlist_request(
            &binding,
            &CreatePlaylistRequest {
                name: "Fresh".into(),
                group: String::new(),
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(created.playlist.name, "Fresh");
        assert_eq!(
            fetch_all_playlists_request(&binding, &FetchAllPlaylistsRequest { call_id: None })
                .await
                .unwrap()
                .unwrap()
                .playlists
                .len(),
            1
        );
    }

    #[test]
    fn subsonic_playlist_capabilities_support_only_expressible_playlist_requests() {
        let slots = RequestSlots::subsonic(&SubsonicProbe {
            api_version: "1.16.1".into(),
            open_subsonic: false,
            server_type: None,
            server_version: None,
            extensions: Vec::new(),
            roles: SubsonicRoles::default(),
            roles_known: true,
        });
        for slot in [
            FetchAllPlaylistsRequest::slot(&slots),
            GetPlaylistByIdRequest::slot(&slots),
            CreatePlaylistRequest::slot(&slots),
            UpdatePlaylistRequest::slot(&slots),
            RemovePlaylistRequest::slot(&slots),
            AddItemToPlaylistRequest::slot(&slots),
            RemoveItemFromPlaylistRequest::slot(&slots),
            ReorderPlaylistItemPositionRequest::slot(&slots),
        ] {
            assert_eq!(slot, Slot::Translate);
        }
        for slot in [
            FetchPlaylistOccurrencesRequest::slot(&slots),
            CreateM3u8PlaylistRequest::slot(&slots),
        ] {
            assert_eq!(
                slot,
                Slot::Unsupported {
                    reason: UnsupportedReason::NotImplemented
                }
            );
        }
    }

    fn assert_secret_free(value: &str) {
        for secret in ["t=", "s=", "p=", "cover-secret-password"] {
            assert!(!value.contains(secret));
        }
    }

    #[test]
    fn jukebox_capabilities_expose_only_controls_supported_by_subsonic() {
        let probe = SubsonicProbe {
            api_version: "1.16.1".into(),
            open_subsonic: false,
            server_type: None,
            server_version: None,
            extensions: Vec::new(),
            roles: SubsonicRoles {
                jukebox_role: true,
                ..SubsonicRoles::default()
            },
            roles_known: true,
        };
        let slots = RequestSlots::subsonic(&probe);
        assert_eq!(PlayRequest::slot(&slots), Slot::Translate);
        assert_eq!(PauseRequest::slot(&slots), Slot::Translate);
        assert_eq!(NextRequest::slot(&slots), Slot::Translate);
        assert_eq!(PreviousRequest::slot(&slots), Slot::Translate);
        assert_eq!(SwitchRequest::slot(&slots), Slot::Translate);
        assert_eq!(SeekRequest::slot(&slots), Slot::Translate);
        assert_eq!(VolumeRequest::slot(&slots), Slot::Translate);
        assert_eq!(LoadRequest::slot(&slots), Slot::Translate);
        assert_eq!(LoadResolvedItemsRequest::slot(&slots), Slot::Translate);
        assert_eq!(
            HandoffLoadRequest::slot(&slots),
            Slot::Unsupported {
                reason: UnsupportedReason::NotImplemented
            }
        );
        assert_eq!(
            TakeBackHandoffBlockRequest::slot(&slots),
            Slot::Unsupported {
                reason: UnsupportedReason::NotImplemented
            }
        );
        assert_eq!(RefreshSessionRequest::slot(&slots), Slot::Translate);
        assert_eq!(RemoveRequest::slot(&slots), Slot::Translate);
        // Existing capability defaults remain in place for unsupported jukebox operations.
        assert_eq!(
            MovePlaylistItemRequest::slot(&slots),
            Slot::Unsupported {
                reason: UnsupportedReason::NotImplemented
            }
        );
        assert_eq!(SetPlaybackModeRequest::slot(&slots), Slot::Translate);
        assert_eq!(
            SetRealtimeFFTEnabledRequest::slot(&slots),
            Slot::Unsupported {
                reason: UnsupportedReason::NotImplemented
            }
        );
        assert_eq!(
            SetAdaptiveSwitchingEnabledRequest::slot(&slots),
            Slot::Unsupported {
                reason: UnsupportedReason::NotImplemented
            }
        );
        assert_eq!(
            CastPreflightRequest::slot(&slots),
            Slot::Unsupported {
                reason: UnsupportedReason::NotImplemented
            }
        );
        assert_eq!(
            SessionSnapshotRequest::slot(&slots),
            Slot::Unsupported {
                reason: UnsupportedReason::NotImplemented
            }
        );
    }

    #[tokio::test]
    async fn play_on_device_reports_now_playing_then_the_completed_track() {
        let root = TempDir::new().unwrap();
        let (binding, server) =
            mock_subsonic_mode(&root, false, SubsonicOpenMode::PlayOnThisDevice).await;
        binding
            .report_track_started_at("song-42", 1_700_000_123_456)
            .await;
        binding.report_track_played("song-42").await;

        let calls = server.state.scrobble_calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["id"], "song-42");
        assert_eq!(calls[0]["submission"], "false");
        assert!(!calls[0].contains_key("time"));
        assert_eq!(calls[1]["id"], "song-42");
        assert_eq!(calls[1]["time"], "1700000123456");
        assert_eq!(calls[1]["submission"], "true");
    }

    #[tokio::test]
    async fn jukebox_binding_does_not_report_local_scrobbles() {
        let root = TempDir::new().unwrap();
        let (binding, server) =
            mock_subsonic_mode(&root, true, SubsonicOpenMode::RemoteControl).await;
        binding
            .report_track_started_at("song-42", 1_700_000_123_456)
            .await;
        binding.report_track_played("song-42").await;

        assert!(server.state.scrobble_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn rune_library_playback_facts_do_not_report_scrobbles() {
        let root = TempDir::new().unwrap();
        let (_, server) =
            mock_subsonic_mode(&root, false, SubsonicOpenMode::PlayOnThisDevice).await;
        let implementation = Implementation::Local;
        let item = PlayingItem::LibraryTrack {
            library_id: Uuid::new_v4(),
            track: "rune-track".into(),
        };

        implementation.track_started(&item, 1_700_000_123_456).await;
        implementation.track_played_through(&item).await;

        assert!(server.state.scrobble_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn skipped_track_keeps_only_the_now_playing_notification() {
        let root = TempDir::new().unwrap();
        let (binding, server) =
            mock_subsonic_mode(&root, false, SubsonicOpenMode::PlayOnThisDevice).await;
        binding
            .report_track_started_at("song-42", 1_700_000_123_456)
            .await;

        let calls = server.state.scrobble_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["id"], "song-42");
        assert_eq!(calls[0]["submission"], "false");
    }

    #[tokio::test]
    async fn unreachable_scrobble_server_does_not_propagate_into_playback() {
        let root = TempDir::new().unwrap();
        let (binding, server) =
            mock_subsonic_mode(&root, false, SubsonicOpenMode::PlayOnThisDevice).await;
        server._task.abort();

        tokio::time::timeout(
            Duration::from_secs(1),
            binding.report_track_started_at("song-42", 1_700_000_123_456),
        )
        .await
        .expect("scrobble failure blocked the playback caller");
    }

    #[tokio::test]
    async fn rejected_scrobble_credentials_emit_the_existing_relogin_failure() {
        let root = TempDir::new().unwrap();
        let (binding, server) =
            mock_subsonic_mode(&root, false, SubsonicOpenMode::PlayOnThisDevice).await;
        server.state.reject_scrobble.store(1, Ordering::SeqCst);
        binding
            .report_track_started_at("song-42", 1_700_000_123_456)
            .await;

        assert_eq!(server.broadcasts.load(Ordering::SeqCst), 1);
    }

    fn subsonic_load(
        library_id: Uuid,
        track_id: &str,
        mode: PlaylistOperateMode,
    ) -> LoadResolvedItemsRequest {
        LoadResolvedItemsRequest {
            items: vec![PlayingItemRef::LibraryTrack {
                library_id: library_id.to_string(),
                track: track_id.to_owned(),
            }],
            start_index: Some(0),
            mode,
            expected_revision: None,
            playback_mode: None,
            position_seconds: None,
            play: None,
            call_id: Some(17),
        }
    }

    #[tokio::test]
    async fn translated_player_calls_issue_jukebox_actions_and_change_status() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, true).await;
        let library_id = binding.library_id;
        RequestTranslator::translate_play_request(&binding, &PlayRequest { call_id: None })
            .await
            .unwrap()
            .unwrap();
        assert!(binding.client.jukebox_status().await.unwrap().playing);
        RequestTranslator::translate_pause_request(&binding, &PauseRequest { call_id: None })
            .await
            .unwrap()
            .unwrap();
        assert!(!binding.client.jukebox_status().await.unwrap().playing);

        RequestTranslator::translate_set_playback_mode_request(
            &binding,
            &SetPlaybackModeRequest {
                mode: 3,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        let unsupported_mode = RequestTranslator::translate_set_playback_mode_request(
            &binding,
            &SetPlaybackModeRequest {
                mode: 1,
                call_id: None,
            },
        )
        .await;
        assert!(matches!(
            unsupported_mode
                .err()
                .and_then(|error| error.downcast::<ExpectedRequestError>().ok()),
            Some(ExpectedRequestError::Unsupported { .. })
        ));

        RequestTranslator::translate_switch_request(
            &binding,
            &SwitchRequest {
                index: 1,
                expected_revision: None,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        RequestTranslator::translate_seek_request(
            &binding,
            &SeekRequest {
                position_seconds: 43.0,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        let status = binding.client.jukebox_status().await.unwrap();
        assert_eq!(status.current_index, Some(1));
        assert_eq!(status.position_secs, Some(43));

        let volume = RequestTranslator::translate_volume_request(
            &binding,
            &VolumeRequest {
                volume: 0.42,
                call_id: Some(8),
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(volume.call_id, Some(8));
        assert_eq!(binding.client.jukebox_status().await.unwrap().gain, 0.42);

        RequestTranslator::translate_load_resolved_items_request(
            &binding,
            &subsonic_load(library_id, "track-c", PlaylistOperateMode::Replace),
        )
        .await
        .unwrap()
        .unwrap();
        let queue = binding.client.jukebox_get().await.unwrap();
        assert_eq!(queue.playlist.songs.len(), 1);
        assert_eq!(queue.playlist.songs[0].id, "track-c");
        RequestTranslator::translate_load_resolved_items_request(
            &binding,
            &subsonic_load(library_id, "track-d", PlaylistOperateMode::AppendToEnd),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            binding
                .client
                .jukebox_get()
                .await
                .unwrap()
                .playlist
                .songs
                .len(),
            2
        );
        RequestTranslator::translate_load_resolved_items_request(
            &binding,
            &subsonic_load(library_id, "track-e", PlaylistOperateMode::PlayNext),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            binding
                .client
                .jukebox_get()
                .await
                .unwrap()
                .playlist
                .songs
                .len(),
            3
        );

        RequestTranslator::translate_load_request(
            &binding,
            &LoadRequest {
                index: 0,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        RequestTranslator::translate_next_request(&binding, &NextRequest { call_id: None })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            binding.client.jukebox_status().await.unwrap().current_index,
            Some(1)
        );
        RequestTranslator::translate_previous_request(&binding, &PreviousRequest { call_id: None })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            binding.client.jukebox_status().await.unwrap().current_index,
            Some(0)
        );
        RequestTranslator::translate_remove_request(
            &binding,
            &RemoveRequest {
                index: 1,
                expected_revision: None,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            binding
                .client
                .jukebox_get()
                .await
                .unwrap()
                .playlist
                .songs
                .len(),
            2
        );

        {
            let calls = server.state.jukebox_calls.lock().unwrap();
            assert!(
                calls
                    .iter()
                    .any(|call| call.get("action").is_some_and(|action| action == "set"))
            );
            assert!(
                calls
                    .iter()
                    .any(|call| call.get("action").is_some_and(|action| action == "add"))
            );
            assert!(
                calls
                    .iter()
                    .any(|call| call.get("action").is_some_and(|action| action == "setGain"))
            );
            assert!(
                calls
                    .iter()
                    .any(|call| call.get("action").is_some_and(|action| action == "remove"))
            );
            assert!(calls.iter().any(|call| {
                call.get("action").is_some_and(|action| action == "skip")
                    && call.get("index").is_some_and(|index| index == "1")
                    && call.get("offset").is_some_and(|offset| offset == "43")
            }));
            assert!(
                calls
                    .iter()
                    .any(|call| call.get("action").is_some_and(|action| action == "shuffle"))
            );
            assert!(
                calls
                    .iter()
                    .any(|call| call.get("action").is_some_and(|action| action == "start"))
            );
            assert!(
                calls
                    .iter()
                    .any(|call| call.get("action").is_some_and(|action| action == "stop"))
            );
        }
        let events_before_refresh = server.broadcasts.load(Ordering::SeqCst);
        RequestTranslator::translate_refresh_session_request(
            &binding,
            &RefreshSessionRequest { call_id: None },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            server.broadcasts.load(Ordering::SeqCst),
            events_before_refresh + 2
        );
    }

    #[tokio::test]
    async fn non_subsonic_load_items_are_rejected_before_http() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, true).await;
        let mut request = subsonic_load(binding.library_id, "unused", PlaylistOperateMode::Replace);
        request.items[0] = PlayingItemRef::from(playback::player::LibraryItem {
            library_id: Uuid::new_v4(),
            file_uuid: Uuid::new_v4(),
        });
        let error = match RequestTranslator::translate_load_resolved_items_request(
            &binding, &request,
        )
        .await
        {
            Ok(_) => panic!("non-Subsonic items must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::InvalidInput { .. })
        ));
        assert!(server.state.jukebox_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn no_role_keeps_player_calls_unsupported_and_makes_no_jukebox_request() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        let slots = RequestSlots::subsonic(&binding.probe);
        for slot in [
            PlayRequest::slot(&slots),
            PauseRequest::slot(&slots),
            LoadRequest::slot(&slots),
            NextRequest::slot(&slots),
            PreviousRequest::slot(&slots),
            SwitchRequest::slot(&slots),
            SeekRequest::slot(&slots),
            VolumeRequest::slot(&slots),
            RemoveRequest::slot(&slots),
            LoadResolvedItemsRequest::slot(&slots),
            SetPlaybackModeRequest::slot(&slots),
            MovePlaylistItemRequest::slot(&slots),
            SetRealtimeFFTEnabledRequest::slot(&slots),
            SetAdaptiveSwitchingEnabledRequest::slot(&slots),
            CastPreflightRequest::slot(&slots),
            RefreshSessionRequest::slot(&slots),
            SessionSnapshotRequest::slot(&slots),
        ] {
            assert_eq!(
                slot,
                Slot::Unsupported {
                    reason: UnsupportedReason::AccountRoleMissing
                }
            );
        }
        assert!(server.state.jukebox_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn jukebox_poll_broadcasts_changes_once_and_stops_when_closed() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, true).await;
        let binding = Arc::new(binding);
        binding.start_jukebox_polling();
        tokio::time::timeout(Duration::from_secs(4), async {
            while server.broadcasts.load(Ordering::SeqCst) < 2 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(3100)).await;
        assert_eq!(server.broadcasts.load(Ordering::SeqCst), 2);
        server.state.jukebox.lock().unwrap().playing = true;
        tokio::time::timeout(Duration::from_secs(4), async {
            while server.broadcasts.load(Ordering::SeqCst) < 4 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        binding.stop_jukebox_polling();
        let stopped_count = server.broadcasts.load(Ordering::SeqCst);
        server
            .state
            .jukebox
            .lock()
            .unwrap()
            .entries
            .push(mock_song("after-close"));
        tokio::time::sleep(Duration::from_millis(3200)).await;
        assert_eq!(server.broadcasts.load(Ordering::SeqCst), stopped_count);
    }

    #[tokio::test]
    async fn browsed_collection_returns_cached_local_path_after_background_fetch() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        fetch_collection_group_summary_request(
            &binding,
            &FetchCollectionGroupSummaryRequest {
                call_id: None,
                collection_type: CollectionType::Artist,
            },
        )
        .await
        .unwrap();
        let request = FetchCollectionGroupsRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
            bake_cover_arts: true,
            group_titles: vec!["A".into()],
        };
        let immediate = fetch_collection_groups_request(&binding, &request)
            .await
            .unwrap()
            .unwrap();
        assert!(
            immediate.groups[0]
                .collections
                .iter()
                .all(|collection| collection.cover_art_map.is_empty())
        );
        wait_for_covers(&binding, &["cover-1", "cover-2"]).await;
        let refreshed = fetch_collection_groups_request(&binding, &request)
            .await
            .unwrap()
            .unwrap();
        assert!(refreshed.groups[0].collections.iter().all(|collection| {
            collection.cover_art_map.len() == 1
                && collection
                    .cover_art_map
                    .values()
                    .all(|path| path.starts_with(root.path().to_str().unwrap()))
        }));
        assert_eq!(server.state.cover_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn rejected_request_credentials_are_typed_as_not_authorized() {
        use subsonic::SubsonicServerError;
        let failures = [
            SubsonicServerError::WrongCredentials,
            SubsonicServerError::NotAuthorized,
            SubsonicServerError::InvalidApiKey,
        ];
        for kind in failures {
            let error = subsonic_request_error(SubsonicError::Server {
                kind,
                message: "rejected".into(),
            });
            assert!(matches!(
                error.downcast_ref::<ExpectedRequestError>(),
                Some(ExpectedRequestError::SubsonicNotAuthorized)
            ));
            assert_eq!(classify(&error), RequestErrorKind::PermissionDenied);
        }
    }

    #[test]
    fn jukebox_failures_keep_authorization_unavailable_and_disabled_types() {
        use subsonic::SubsonicServerError;

        let unauthorized = jukebox_error(
            SubsonicError::Server {
                kind: SubsonicServerError::WrongCredentials,
                message: "rejected".into(),
            },
            PlayRequest::wire_name(),
        );
        assert!(matches!(
            unauthorized.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::SubsonicNotAuthorized)
        ));
        let unavailable = jukebox_error(
            SubsonicError::Transport("offline".into()),
            PlayRequest::wire_name(),
        );
        assert!(matches!(
            unavailable.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Unavailable { .. })
        ));
        let disabled = jukebox_error(SubsonicError::JukeboxUnsupported, PlayRequest::wire_name());
        assert!(matches!(
            disabled.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Unsupported {
                reason: UnsupportedReason::NotImplemented,
                ..
            })
        ));
    }
}
