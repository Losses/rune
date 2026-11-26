use anyhow::{Result, anyhow};
use database::actions::collection::CollectionQueryType;
use database::actions::utils::generate_group_name;
use database::error::ExpectedError;
use lyric::lrc::parse_lrc;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
    time::Instant,
};
use subsonic::{
    SearchLimits, SubsonicAlbum, SubsonicArtist, SubsonicClient, SubsonicError, SubsonicPlaylist,
    SubsonicServerError, SubsonicSong,
};

use crate::TypedRequest;
use crate::macros::{RequestSlots, RequestTranslator};
use crate::messages::{
    AddItemToPlaylistRequest, AddItemToPlaylistResponse, Album, Artist, AudioAvailability,
    AudioOrigin, CastPreflightRequest, Collection, CollectionGroup, CollectionGroupSummary,
    CollectionGroupSummaryResponse, CollectionType, ComplexQueryRequest, ComplexQueryResponse,
    CreatePlaylistRequest, CreatePlaylistResponse, FetchAllPlaylistsRequest,
    FetchAllPlaylistsResponse, FetchCollectionByIdsRequest, FetchCollectionByIdsResponse,
    FetchCollectionGroupSummaryRequest, FetchCollectionGroupsRequest,
    FetchCollectionGroupsResponse, FetchMediaFileByIdsRequest, FetchMediaFileByIdsResponse,
    FetchMediaFilesRequest, FetchMediaFilesResponse, FetchParsedMediaFileRequest,
    FetchParsedMediaFileResponse, FetchPlaylistOccurrencesRequest,
    FetchPlaylistOccurrencesResponse, GetLikedRequest, GetLikedResponse, GetLyricByTrackIdRequest,
    GetLyricByTrackIdResponse, GetMediaFilesCountRequest, GetMediaFilesCountResponse,
    GetPlaylistByIdRequest, GetPlaylistByIdResponse, GetPrimaryColorByTrackIdRequest,
    HandoffLoadRequest, LibraryContentChanged, LoadPlay, LoadRequest, LoadResolvedItemsRequest,
    LoadResolvedItemsResponse, LyricContentLine, LyricContentLineSection, MediaFile,
    MediaFileSummary, MixQueryRequest, MixQueryResponse, MovePlaylistItemRequest, NextRequest,
    PauseRequest, PlayRequest, PlaybackMode, PlayingItemRef, Playlist, PlaylistOperateMode,
    PlaylistRevisionRequest, PlaylistRevisionResponse, PreviousRequest, RefreshSessionRequest,
    RemoveItemFromPlaylistRequest, RemoveItemFromPlaylistResponse, RemovePlaylistRequest,
    RemovePlaylistResponse, RemoveRequest, ReorderPlaylistItemPositionRequest,
    ReorderPlaylistItemPositionResponse, ResolveMixQueryRequest, ResolveMixQueryResponse,
    SearchCollectionSummaryRequest, SearchCollectionSummaryResponse, SearchForRequest,
    SearchForResponse, SearchMediaFileSummaryRequest, SearchMediaFileSummaryResponse, SeekRequest,
    SessionSnapshotRequest, SessionSnapshotResponse, SetAdaptiveSwitchingEnabledRequest,
    SetLikedRequest, SetLikedResponse, SetPlaybackModeRequest, SetRealtimeFFTEnabledRequest,
    SwitchRequest, TakeBackHandoffBlockRequest, UpdatePlaylistRequest, UpdatePlaylistResponse,
    VolumeRequest, VolumeResponse,
};
use crate::request_error::ExpectedRequestError;
use crate::rpc::{RequestTranslationFuture, Slot, UnsupportedReason};
use crate::service::{
    ServiceFuture,
    home::{self, HomeItem, HomeSource, HomeTrack},
    ids::RemoteIdMap,
    mixes::{self, MixItem, MixSource, MixTarget, Plan},
    positional_occurrences::{
        listed_after, occurrence_id, occurrence_page, occurrence_position, occurrences_response,
        parse_occurrence_id, playlist_fingerprint,
    },
};
use crate::subsonic_binding::{SubsonicBinding, SubsonicCatalog, SubsonicIdKind};
use crate::utils::RESOLVED_TRACK_LIMIT;
use reqwest::StatusCode as ReqwestStatusCode;
use uuid::Uuid;

fn subsonic_track_id(binding: &SubsonicBinding, item: Option<&PlayingItemRef>) -> Result<String> {
    match item {
        Some(PlayingItemRef::LibraryTrack { library_id, track })
            if library_id == &binding.library_id.to_string() =>
        {
            Ok(track.clone())
        }
        Some(PlayingItemRef::LibraryTrack { .. }) => {
            Err(ExpectedRequestError::invalid_input("Item belongs to another library").into())
        }
        _ => Err(ExpectedRequestError::invalid_input("Expected a Subsonic library track").into()),
    }
}

pub(crate) async fn set_liked_request(
    binding: &SubsonicBinding,
    request: &SetLikedRequest,
) -> Result<Option<SetLikedResponse>> {
    let track = subsonic_track_id(binding, request.item.as_ref())?;
    if request.liked {
        binding
            .client
            .star(&[&track])
            .await
            .map_err(|error| subsonic_request_error(binding, error))?;
    } else {
        binding
            .client
            .unstar(&[&track])
            .await
            .map_err(|error| subsonic_request_error(binding, error))?;
    }
    Ok(request.item.clone().map(|item| SetLikedResponse {
        item,
        liked: request.liked,
        call_id: request.call_id,
    }))
}

pub(crate) async fn get_liked_request(
    binding: &SubsonicBinding,
    request: &GetLikedRequest,
) -> Result<Option<GetLikedResponse>> {
    let track = subsonic_track_id(binding, request.item.as_ref())?;
    let song = binding
        .client
        .song(&track)
        .await
        .map_err(|error| subsonic_request_error(binding, error))?;
    Ok(request.item.clone().map(|item| GetLikedResponse {
        item,
        liked: song.starred.is_some(),
        call_id: request.call_id,
    }))
}

pub(crate) async fn get_lyric_request(
    binding: &SubsonicBinding,
    request: &GetLyricByTrackIdRequest,
) -> Result<Option<GetLyricByTrackIdResponse>> {
    let track = subsonic_track_id(binding, request.item.as_ref())?;
    let song = binding
        .client
        .song(&track)
        .await
        .map_err(|error| subsonic_request_error(binding, error))?;
    let needs_extension = binding
        .probe()
        .extensions
        .iter()
        .any(|(name, _)| name == "songLyrics");
    let lyrics = if needs_extension {
        binding
            .client
            .lyrics_by_song_id(&track)
            .await
            .map_err(|error| subsonic_request_error(binding, error))?
    } else {
        binding
            .client
            .lyrics(song.artist.as_deref().unwrap_or_default(), &song.title)
            .await
            .map_err(|error| subsonic_request_error(binding, error))?
    };
    let mut lines = Vec::new();
    if let Some(lyric) = lyrics
        .synced_lyrics
        .into_iter()
        .find(|lyrics| lyrics.synced)
    {
        for (index, line) in lyric.lines.iter().enumerate() {
            let start = line.start.min(i32::MAX as u32) as i32;
            let end = lyric
                .lines
                .get(index + 1)
                .map(|next| next.start.min(i32::MAX as u32) as i32)
                .unwrap_or(start);
            lines.push(LyricContentLine {
                start_time: start,
                end_time: end,
                sections: vec![LyricContentLineSection {
                    start_time: start,
                    end_time: end,
                    content: line.value.clone(),
                }],
            });
        }
    } else if let Some(plain) = lyrics.plain_lyrics.filter(|value| !value.trim().is_empty()) {
        let parsed_lines = parse_lrc(&plain).ok().map(|parsed| parsed.lyrics);
        if let Some(parsed_lines) = parsed_lines.filter(|lines| !lines.is_empty()) {
            lines = parsed_lines
                .into_iter()
                .map(|line| {
                    let start_time = line.start_time.into();
                    let end_time = line.end_time.into();
                    let content = line.text;
                    LyricContentLine {
                        start_time,
                        end_time,
                        sections: vec![LyricContentLineSection {
                            start_time,
                            end_time,
                            content,
                        }],
                    }
                })
                .collect();
        } else {
            lines = plain
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| LyricContentLine {
                    start_time: 0,
                    end_time: 0,
                    sections: vec![LyricContentLineSection {
                        start_time: 0,
                        end_time: 0,
                        content: line.to_owned(),
                    }],
                })
                .collect();
        }
    }
    Ok(request.item.clone().map(|item| GetLyricByTrackIdResponse {
        item,
        lines,
        call_id: request.call_id,
    }))
}

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
        .map_err(|error| subsonic_request_error(binding, error))?;
    Ok((
        remote_id,
        playlist.songs.into_iter().map(|song| song.id).collect(),
    ))
}

/// A playlist's songs in the server's order, repeats included.
async fn playlist_songs(binding: &SubsonicBinding, id: i32) -> Result<Vec<SubsonicSong>> {
    let remote_id = resolve_playlist_id(binding, id)?;
    Ok(binding
        .client
        .playlist(&remote_id)
        .await
        .map_err(|error| subsonic_request_error(binding, error))?
        .songs)
}

/// The server's playlists, under the IDs this library gives them.
async fn interned_playlists(binding: &SubsonicBinding) -> Result<Vec<(i32, SubsonicPlaylist)>> {
    let remote = binding
        .client
        .playlists()
        .await
        .map_err(|error| subsonic_request_error(binding, error))?;
    let mut catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    remote
        .into_iter()
        .map(|item| {
            Ok((
                catalog.ids.intern(SubsonicIdKind::Playlist, &item.id)?,
                item,
            ))
        })
        .collect()
}

pub(crate) async fn fetch_all_playlists_request(
    binding: &SubsonicBinding,
    _: &FetchAllPlaylistsRequest,
) -> Result<Option<FetchAllPlaylistsResponse>> {
    let playlists = interned_playlists(binding)
        .await?
        .into_iter()
        .map(|(id, item)| Playlist {
            id,
            name: item.name,
            group: String::new(),
        })
        .collect();
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
        .map_err(|error| subsonic_request_error(binding, error))?;
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
        .map_err(|error| subsonic_request_error(binding, error))?
        .into_iter()
        .map(|item| item.id)
        .collect::<HashSet<_>>();
    binding
        .client
        .create_playlist(&request.name, &[])
        .await
        .map_err(|error| subsonic_request_error(binding, error))?;
    // The playlist exists from now on, whether or not it can be read back.
    playlist_entries_changed(binding);
    let remote = binding
        .client
        .playlists()
        .await
        .map_err(|error| subsonic_request_error(binding, error))?;
    let created = remote
        .into_iter()
        .find(|item| item.name == request.name && !existing_ids.contains(&item.id))
        .ok_or_else(|| ExpectedRequestError::UnexpectedResponse {
            expected: "created Subsonic playlist".into(),
            actual: "no matching playlist returned".into(),
        })?;
    let id = playlist_id(binding, &created.id)?;
    Ok(Some(CreatePlaylistResponse {
        playlist: Playlist {
            id,
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
        .map_err(|error| subsonic_request_error(binding, error))?;
    playlist_entries_changed(binding);
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
        .map_err(|error| subsonic_request_error(binding, error))?;
    playlist_entries_changed(binding);
    Ok(Some(RemovePlaylistResponse {
        playlist_id: request.playlist_id,
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
        .map_err(|error| subsonic_request_error(binding, error))?;
    let mut songs = playlist
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
        .map_err(|error| subsonic_request_error(binding, error))?;
    playlist_entries_changed(binding);
    songs.push(track_remote);
    Ok(Some(AddItemToPlaylistResponse {
        occurrence_id: occurrence_id(
            append_position,
            &playlist_fingerprint(songs.iter().map(String::as_str)),
        ),
        call_id: None,
    }))
}

/// Tells open listings that the playlists or a playlist's entries changed, as a local library
/// does: the playlists page shows the change, and a listing of the entries lists them again
/// rather than page or edit with occurrences of the old order. Subsonic names entries only by
/// their order, so an occurrence is a position together with the fingerprint of the order it
/// was listed in (see `positional_occurrences`).
fn playlist_entries_changed(binding: &SubsonicBinding) {
    binding
        .covers
        .broadcaster
        .broadcast(&LibraryContentChanged {
            path: String::new(),
        });
}

pub(crate) async fn remove_item_from_playlist_request(
    binding: &SubsonicBinding,
    request: &RemoveItemFromPlaylistRequest,
) -> Result<Option<RemoveItemFromPlaylistResponse>> {
    let occurrence = parse_occurrence_id(&request.occurrence_id)?;
    let (remote_id, songs) = fetch_remote_playlist(binding, request.playlist_id).await?;
    let position = occurrence_position(occurrence, &songs)?;
    let position = u32::try_from(position)
        .map_err(|_| ExpectedRequestError::invalid_input("Subsonic playlist is too large"))?;
    binding
        .client
        .update_playlist(&remote_id, None, &[], &[position])
        .await
        .map_err(|error| subsonic_request_error(binding, error))?;
    playlist_entries_changed(binding);
    Ok(Some(RemoveItemFromPlaylistResponse { call_id: None }))
}

pub(crate) async fn reorder_playlist_item_position_request(
    binding: &SubsonicBinding,
    request: &ReorderPlaylistItemPositionRequest,
) -> Result<Option<ReorderPlaylistItemPositionResponse>> {
    let occurrence = parse_occurrence_id(&request.occurrence_id)?;
    let (remote_id, songs) = fetch_remote_playlist(binding, request.playlist_id).await?;
    let from = occurrence_position(occurrence, &songs)?;
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
        .map_err(|error| subsonic_request_error(binding, error))?;
    playlist_entries_changed(binding);
    Ok(Some(ReorderPlaylistItemPositionResponse { call_id: None }))
}

/// Maps each object to its cover's file in the local cover cache, and returns the covers that
/// are not cached.
async fn cached_cover_map(
    binding: &SubsonicBinding,
    covers: Vec<(i32, String)>,
) -> Result<(HashMap<i32, String>, Vec<String>)> {
    let mut map = HashMap::new();
    let mut missing = Vec::new();
    for (object_id, cover_id) in covers {
        match binding.covers.cached_path(&cover_id).await? {
            Some(path) => {
                map.insert(object_id, path);
            }
            None => missing.push(cover_id),
        }
    }
    Ok((map, missing))
}

/// Lists a playlist's entries in order, repeats included, named as the playlist edits above
/// name them. A page continues only the order the previous page was listed in.
pub(crate) async fn fetch_playlist_occurrences_request(
    binding: &SubsonicBinding,
    request: &FetchPlaylistOccurrencesRequest,
) -> Result<Option<FetchPlaylistOccurrencesResponse>> {
    let after = listed_after(request)?;
    let songs = playlist_songs(binding, request.playlist_id).await?;
    let page = occurrence_page(
        after,
        songs.iter().map(|song| song.id.as_str()),
        request.page_size,
    )?;
    let (files, covers) = translate_songs(binding, &songs[page.range.clone()])?;
    let cover_art_map = if request.bake_cover_arts {
        subsonic_cover_map(binding, covers).await?
    } else {
        HashMap::new()
    };
    Ok(Some(occurrences_response(
        request,
        page,
        files,
        cover_art_map,
    )?))
}

async fn subsonic_cover_map(
    binding: &SubsonicBinding,
    covers: Vec<(i32, String)>,
) -> Result<HashMap<i32, String>> {
    let (map, missing) = cached_cover_map(binding, covers).await?;
    for cover_id in missing {
        binding
            .covers
            .schedule(Arc::clone(&binding.client), cover_id);
    }
    Ok(map)
}

pub(crate) async fn search_for_request(
    binding: &SubsonicBinding,
    request: &SearchForRequest,
) -> Result<Option<SearchForResponse>> {
    let fields: Vec<CollectionQueryType> = request.fields.iter().copied().map(Into::into).collect();
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
        .map_err(|error| subsonic_request_error(binding, error))?;
    let playlists = if count > 0 && (all || fields.contains(&CollectionQueryType::Playlist)) {
        // The search box ends its text with `*`, the prefix wildcard of Rune's search
        // syntax, which is not part of any playlist name.
        let query = request.query_str.trim_end_matches('*').to_lowercase();
        interned_playlists(binding)
            .await?
            .into_iter()
            .filter(|(_, item)| item.name.to_lowercase().contains(&query))
            .map(|(id, _)| id)
            .take(count as usize)
            .collect()
    } else {
        Vec::new()
    };
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
        playlists,
        tracks,
        genres: Vec::new(),
        directories: Vec::new(),
        mixes: Vec::new(),
    }))
}

/// The song a track id names: as listed before, or else as the server gives it now, which is
/// kept.
async fn song_by_id(binding: &SubsonicBinding, id: i32) -> Result<SubsonicSong> {
    let (remote_id, cached_song) = {
        let catalog = binding
            .catalog
            .lock()
            .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
        (
            catalog.ids.resolve(SubsonicIdKind::Track, id)?.to_owned(),
            catalog.songs.get(&id).cloned(),
        )
    };
    if let Some(song) = cached_song {
        return Ok(song);
    }
    let song = binding
        .client
        .song(&remote_id)
        .await
        .map_err(|error| subsonic_request_error(binding, error))?;
    let mut catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    let id = catalog.ids.intern(SubsonicIdKind::Track, &song.id)?;
    catalog.songs.insert(id, song.clone());
    Ok(song)
}

pub(crate) async fn fetch_media_file_by_ids_request(
    binding: &SubsonicBinding,
    request: &FetchMediaFileByIdsRequest,
) -> Result<Option<FetchMediaFileByIdsResponse>> {
    let mut media_files = Vec::with_capacity(request.ids.len());
    let mut covers = Vec::with_capacity(request.ids.len());
    for id in &request.ids {
        let song = song_by_id(binding, *id).await?;
        let cover_art_id = {
            let mut catalog = binding
                .catalog
                .lock()
                .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
            song.cover_art
                .as_deref()
                .map(|cover| catalog.ids.intern(SubsonicIdKind::CoverArt, cover))
                .transpose()?
        };
        if let Some(cover) = song.cover_art.clone() {
            covers.push((*id, cover));
        }
        media_files.push(subsonic_media_file(binding, *id, cover_art_id, &song));
    }
    Ok(Some(FetchMediaFileByIdsResponse {
        call_id: None,
        media_files,
        cover_art_map: subsonic_cover_map(binding, covers).await?,
    }))
}

fn empty_search_error(
    binding: &SubsonicBinding,
    error: SubsonicError,
    request: &'static str,
) -> anyhow::Error {
    match error {
        SubsonicError::Server {
            kind: SubsonicServerError::MissingParameter,
            ..
        } => ExpectedRequestError::Unsupported {
            request,
            reason: UnsupportedReason::NotImplemented,
        }
        .into(),
        error => subsonic_request_error(binding, error),
    }
}

fn media_file_from_song(
    binding: &SubsonicBinding,
    id: i32,
    song: &SubsonicSong,
) -> Result<MediaFile> {
    let cover_art_id = {
        let mut catalog = binding
            .catalog
            .lock()
            .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
        song.cover_art
            .as_deref()
            .map(|cover| catalog.ids.intern(SubsonicIdKind::CoverArt, cover))
            .transpose()?
    };
    Ok(subsonic_media_file(binding, id, cover_art_id, song))
}

/// Subsonic tracks stream from the server: playable here, not stored here.
const SUBSONIC_AUDIO_AVAILABILITY: AudioAvailability = AudioAvailability::RemoteOnly;

/// The `MediaFile` for a Subsonic song whose ids are already interned.
fn subsonic_media_file(
    binding: &SubsonicBinding,
    id: i32,
    cover_art_id: Option<i32>,
    song: &SubsonicSong,
) -> MediaFile {
    MediaFile {
        id,
        library_id: binding.library_id.to_string(),
        track: song.id.clone(),
        path: String::new(),
        artist: song.artist.clone().unwrap_or_default(),
        album: song.album.clone().unwrap_or_default(),
        title: song.title.clone(),
        duration: f64::from(song.duration.unwrap_or_default()),
        cover_art_id,
        track_number: song.track.and_then(|number| i32::try_from(number).ok()),
        audio_availability: SUBSONIC_AUDIO_AVAILABILITY,
        audio_origin: AudioOrigin::Remote,
    }
}

pub(crate) async fn fetch_media_files_request(
    binding: &SubsonicBinding,
    request: &FetchMediaFilesRequest,
) -> Result<Option<FetchMediaFilesResponse>> {
    let offset = u32::try_from(request.cursor)
        .map_err(|error| ExpectedRequestError::invalid_input(format!("Invalid cursor: {error}")))?;
    let count = u32::try_from(request.page_size).map_err(|error| {
        ExpectedRequestError::invalid_input(format!("Invalid page size: {error}"))
    })?;
    let songs = binding
        .client
        .search_with_song_offset(
            "",
            SearchLimits {
                songs: count,
                ..Default::default()
            },
            offset,
        )
        .await
        .map_err(|error| empty_search_error(binding, error, FetchMediaFilesRequest::wire_name()))?
        .songs;
    let mut media_files = Vec::with_capacity(songs.len());
    let mut covers = Vec::new();
    for song in songs {
        let id = {
            let mut catalog = binding
                .catalog
                .lock()
                .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
            let id = catalog.ids.intern(SubsonicIdKind::Track, &song.id)?;
            catalog.songs.insert(id, song.clone());
            id
        };
        if let Some(cover) = song.cover_art.clone() {
            covers.push((id, cover));
        }
        media_files.push(media_file_from_song(binding, id, &song)?);
    }
    let cover_art_map = if request.bake_cover_arts {
        subsonic_cover_map(binding, covers).await?
    } else {
        HashMap::new()
    };
    Ok(Some(FetchMediaFilesResponse {
        media_files,
        cover_art_map,
        call_id: request.call_id,
    }))
}

pub(crate) async fn get_media_files_count_request(
    binding: &SubsonicBinding,
    request: &GetMediaFilesCountRequest,
) -> Result<Option<GetMediaFilesCountResponse>> {
    let scan_status = binding
        .client
        .scan_status()
        .await
        .map_err(|error| subsonic_request_error(binding, error))?;
    let count = scan_status
        .count
        .ok_or_else(|| ExpectedRequestError::UnexpectedResponse {
            expected: "Subsonic scan status song count".into(),
            actual: "scan status omitted count".into(),
        })?;
    Ok(Some(GetMediaFilesCountResponse {
        count: i32::try_from(count).map_err(|error| {
            ExpectedRequestError::invalid_input(format!("Invalid song count: {error}"))
        })?,
        call_id: request.call_id,
    }))
}

pub(crate) async fn fetch_parsed_media_file_request(
    binding: &SubsonicBinding,
    request: &FetchParsedMediaFileRequest,
) -> Result<Option<FetchParsedMediaFileResponse>> {
    let remote_id = {
        let catalog = binding
            .catalog
            .lock()
            .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
        catalog
            .ids
            .resolve(SubsonicIdKind::Track, request.id)?
            .to_owned()
    };
    let song = binding
        .client
        .song(&remote_id)
        .await
        .map_err(|error| subsonic_request_error(binding, error))?;
    let mut catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    let id = catalog.ids.intern(SubsonicIdKind::Track, &song.id)?;
    catalog.songs.insert(id, song.clone());
    let artists = song
        .artist
        .as_ref()
        .map(|name| -> Result<Artist> {
            let id = song
                .artist_id
                .as_deref()
                .map(|remote| catalog.ids.intern(SubsonicIdKind::Artist, remote))
                .transpose()?;
            Ok(Artist {
                id: id.unwrap_or_default(),
                name: name.clone(),
            })
        })
        .transpose()?
        .into_iter()
        .collect();
    let album_id = song
        .album_id
        .as_deref()
        .map(|remote| catalog.ids.intern(SubsonicIdKind::Album, remote))
        .transpose()?
        .unwrap_or_default();
    let album = Album {
        id: album_id,
        name: song.album.clone().unwrap_or_default(),
    };
    drop(catalog);
    Ok(Some(FetchParsedMediaFileResponse {
        file: media_file_from_song(binding, id, &song)?,
        artists,
        album,
        call_id: request.call_id,
    }))
}

pub(crate) async fn search_media_file_summary_request(
    binding: &SubsonicBinding,
    request: &SearchMediaFileSummaryRequest,
) -> Result<Option<SearchMediaFileSummaryResponse>> {
    let count = u32::try_from(request.n)
        .map_err(|error| ExpectedRequestError::invalid_input(format!("Invalid N: {error}")))?;
    let songs = binding
        .client
        .search(
            "",
            SearchLimits {
                songs: count,
                ..Default::default()
            },
        )
        .await
        .map_err(|error| {
            empty_search_error(binding, error, SearchMediaFileSummaryRequest::wire_name())
        })?
        .songs;
    let mut catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    let result = songs
        .into_iter()
        .map(|song| {
            let id = catalog.ids.intern(SubsonicIdKind::Track, &song.id)?;
            let cover_art_id = song
                .cover_art
                .as_deref()
                .map(|cover| catalog.ids.intern(SubsonicIdKind::CoverArt, cover))
                .transpose()?;
            catalog.songs.insert(id, song.clone());
            Ok(MediaFileSummary {
                id,
                name: song.title,
                cover_art_id,
                audio_availability: SUBSONIC_AUDIO_AVAILABILITY,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(SearchMediaFileSummaryResponse {
        result,
        call_id: request.call_id,
    }))
}

pub(crate) async fn search_collection_summary_request(
    binding: &SubsonicBinding,
    request: &SearchCollectionSummaryRequest,
) -> Result<Option<SearchCollectionSummaryResponse>> {
    let collection_type = request.collection_type.ok_or_else(|| {
        ExpectedRequestError::invalid_input("Subsonic collection summary requires album or artist")
    })?;
    let (kind, limits) = match collection_type {
        CollectionType::Album => (
            SubsonicIdKind::Album,
            SearchLimits {
                albums: u32::try_from(request.n).map_err(|error| {
                    ExpectedRequestError::invalid_input(format!("Invalid N: {error}"))
                })?,
                ..Default::default()
            },
        ),
        CollectionType::Artist => (
            SubsonicIdKind::Artist,
            SearchLimits {
                artists: u32::try_from(request.n).map_err(|error| {
                    ExpectedRequestError::invalid_input(format!("Invalid N: {error}"))
                })?,
                ..Default::default()
            },
        ),
        _ => {
            return Err(ExpectedRequestError::Unsupported {
                request: SearchCollectionSummaryRequest::wire_name(),
                reason: UnsupportedReason::NotImplemented,
            }
            .into());
        }
    };
    let results = binding.client.search("", limits).await.map_err(|error| {
        empty_search_error(binding, error, SearchCollectionSummaryRequest::wire_name())
    })?;
    let entries = {
        let mut catalog = binding
            .catalog
            .lock()
            .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
        match kind {
            SubsonicIdKind::Album => results
                .albums
                .into_iter()
                .map(|item| {
                    let id = catalog.ids.intern(kind, &item.id)?;
                    let cover = item.cover_art.clone();
                    catalog.albums.insert(id, item.clone());
                    Ok((id, item.name, cover))
                })
                .collect::<Result<Vec<_>>>()?,
            SubsonicIdKind::Artist => results
                .artists
                .into_iter()
                .map(|item| {
                    let id = catalog.ids.intern(kind, &item.id)?;
                    let cover = item.cover_art.clone();
                    catalog.artists.insert(id, item.clone());
                    Ok((id, item.name, cover))
                })
                .collect::<Result<Vec<_>>>()?,
            _ => Vec::new(),
        }
    };
    let mut collections = Vec::with_capacity(entries.len());
    for (id, name, cover) in &entries {
        let cover_art_map = subsonic_cover_map(
            binding,
            cover
                .clone()
                .map(|cover| vec![(*id, cover)])
                .unwrap_or_default(),
        )
        .await?;
        collections.push(Collection {
            id: *id,
            name: name.clone(),
            queries: Vec::new(),
            collection_type,
            cover_art_map,
            readonly: true,
        });
    }
    Ok(Some(SearchCollectionSummaryResponse {
        collection_type,
        result: collections,
        call_id: request.call_id,
    }))
}

pub(crate) async fn fetch_collection_by_ids_request(
    binding: &SubsonicBinding,
    request: &FetchCollectionByIdsRequest,
) -> Result<Option<FetchCollectionByIdsResponse>> {
    if request.collection_type == CollectionType::Playlist {
        {
            let catalog = binding
                .catalog
                .lock()
                .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
            for id in &request.ids {
                catalog.ids.resolve(SubsonicIdKind::Playlist, *id)?;
            }
        }
        let playlists = interned_playlists(binding).await?;
        let mut result = Vec::with_capacity(request.ids.len());
        for id in &request.ids {
            let Some((_, item)) = playlists.iter().find(|(playlist_id, _)| playlist_id == id)
            else {
                continue;
            };
            let cover_art_map = subsonic_cover_map(
                binding,
                item.cover_art
                    .clone()
                    .map(|cover| vec![(*id, cover)])
                    .unwrap_or_default(),
            )
            .await?;
            result.push(Collection {
                id: *id,
                name: item.name.clone(),
                queries: Vec::new(),
                collection_type: CollectionType::Playlist,
                cover_art_map,
                readonly: false,
            });
        }
        return Ok(Some(FetchCollectionByIdsResponse {
            call_id: None,
            collection_type: request.collection_type,
            result,
        }));
    }
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
                        .map_err(|error| subsonic_request_error(binding, error))?;
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
                        .map_err(|error| subsonic_request_error(binding, error))?;
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
        )
        .await?;
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

fn jukebox_error(
    binding: &SubsonicBinding,
    error: SubsonicError,
    request: &'static str,
) -> anyhow::Error {
    match error {
        SubsonicError::Transport(_) => ExpectedRequestError::Unavailable {
            what: "Subsonic jukebox",
        }
        .into(),
        SubsonicError::HttpStatus(status) if status == ReqwestStatusCode::UNAUTHORIZED => {
            sign_in_required(binding)
        }
        SubsonicError::HttpStatus(_) => ExpectedRequestError::Unavailable {
            what: "Subsonic jukebox",
        }
        .into(),
        SubsonicError::JukeboxUnsupported => jukebox_disabled(request),
        error => subsonic_request_error(binding, error),
    }
}

pub(crate) fn jukebox_error_for_refresh(
    binding: &SubsonicBinding,
    error: SubsonicError,
) -> anyhow::Error {
    jukebox_error(binding, error, RefreshSessionRequest::wire_name())
}

async fn jukebox_status(binding: &SubsonicBinding) -> Result<subsonic::SubsonicJukeboxStatus> {
    binding
        .client
        .jukebox_status()
        .await
        .map_err(|error| jukebox_error(binding, error, "Subsonic jukebox status"))
}

async fn jukebox_play_request(binding: &SubsonicBinding, _: &PlayRequest) -> Result<Option<()>> {
    binding
        .client
        .jukebox_start()
        .await
        .map_err(|error| jukebox_error(binding, error, PlayRequest::wire_name()))?;
    Ok(Some(()))
}

async fn jukebox_pause_request(binding: &SubsonicBinding, _: &PauseRequest) -> Result<Option<()>> {
    binding
        .client
        .jukebox_stop()
        .await
        .map_err(|error| jukebox_error(binding, error, PauseRequest::wire_name()))?;
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
        .map_err(|error| jukebox_error(binding, error, LoadRequest::wire_name()))?;
    Ok(Some(()))
}

async fn jukebox_next_request(binding: &SubsonicBinding, _: &NextRequest) -> Result<Option<()>> {
    let queue = binding
        .client
        .jukebox_get()
        .await
        .map_err(|error| jukebox_error(binding, error, NextRequest::wire_name()))?;
    // The jukebox reports Sequential, so Next follows Rune's sequential rule: the
    // next song if there is one, otherwise playback stops. With no current song
    // there is nothing to move on from.
    let Some(current) = queue.status.current_index else {
        return Ok(Some(()));
    };
    let next = current.saturating_add(1);
    if (next as usize) < queue.playlist.songs.len() {
        binding
            .client
            .jukebox_skip(next, 0)
            .await
            .map_err(|error| jukebox_error(binding, error, NextRequest::wire_name()))?;
    } else {
        binding
            .client
            .jukebox_stop()
            .await
            .map_err(|error| jukebox_error(binding, error, NextRequest::wire_name()))?;
    }
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
        .map_err(|error| jukebox_error(binding, error, PreviousRequest::wire_name()))?;
    let index = queue.status.current_index.unwrap_or(0).saturating_sub(1);
    binding
        .client
        .jukebox_skip(index, 0)
        .await
        .map_err(|error| jukebox_error(binding, error, PreviousRequest::wire_name()))?;
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
        .map_err(|error| jukebox_error(binding, error, SwitchRequest::wire_name()))?;
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
        .map_err(|error| jukebox_error(binding, error, SeekRequest::wire_name()))?;
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
        .map_err(|error| jukebox_error(binding, error, VolumeRequest::wire_name()))?;
    Ok(Some(VolumeResponse {
        volume: request.volume,
        call_id: request.call_id,
    }))
}

async fn jukebox_playback_mode_request(
    binding: &SubsonicBinding,
    request: &SetPlaybackModeRequest,
) -> Result<Option<()>> {
    if request.mode != PlaybackMode::Shuffle {
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
        .map_err(|error| jukebox_error(binding, error, SetPlaybackModeRequest::wire_name()))?;
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
        .map_err(|error| jukebox_error(binding, error, RemoveRequest::wire_name()))?;
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

/// Whether a load that jumps to a start index then plays. The same rule as Rune's
/// own queue edit: an explicit choice wins, a position alone means "do not
/// assume", and a plain load plays.
fn load_plays(request: &LoadResolvedItemsRequest) -> bool {
    match request.play {
        LoadPlay::Play => true,
        LoadPlay::Pause => false,
        LoadPlay::Auto => request.position_seconds.is_none() || request.playback_mode.is_some(),
    }
}

/// The song and offset a load jumps to, checked against the loaded items.
fn load_start(request: &LoadResolvedItemsRequest) -> Result<Option<(u32, u32)>> {
    let Some(start) = request.start_index.filter(|_| !request.items.is_empty()) else {
        return Ok(None);
    };
    if start as usize >= request.items.len() {
        return Err(ExpectedRequestError::invalid_input("Invalid jukebox start index").into());
    }
    let offset = match request.position_seconds {
        None => 0,
        Some(position)
            if position.is_finite() && (0.0..=f64::from(u32::MAX)).contains(&position) =>
        {
            position as u32
        }
        Some(_) => {
            return Err(
                ExpectedRequestError::invalid_input("Invalid jukebox start position").into(),
            );
        }
    };
    Ok(Some((start, offset)))
}

/// Puts `ids` right after the current song and returns the index of the first one.
/// The jukebox has no insert, so the queue is rebuilt with `set` and the current
/// song is jumped back to at its position, and started again if it was playing.
async fn jukebox_insert_after_current(binding: &SubsonicBinding, ids: &[String]) -> Result<u32> {
    let request = LoadResolvedItemsRequest::wire_name();
    let queue = binding
        .client
        .jukebox_get()
        .await
        .map_err(|error| jukebox_error(binding, error, request))?;
    let existing = queue.playlist.songs.iter().map(|song| song.id.as_str());
    let added = ids.iter().map(String::as_str);
    let Some(current) = queue
        .status
        .current_index
        .filter(|current| (*current as usize) < queue.playlist.songs.len())
    else {
        let end = queue.playlist.songs.len() as u32;
        binding
            .client
            .jukebox_add(&added.collect::<Vec<_>>())
            .await
            .map_err(|error| jukebox_error(binding, error, request))?;
        return Ok(end);
    };
    let split = current as usize + 1;
    let rebuilt = existing
        .clone()
        .take(split)
        .chain(added)
        .chain(existing.skip(split))
        .collect::<Vec<_>>();
    binding
        .client
        .jukebox_set(&rebuilt)
        .await
        .map_err(|error| jukebox_error(binding, error, request))?;
    binding
        .client
        .jukebox_skip(current, queue.status.position_secs.unwrap_or_default())
        .await
        .map_err(|error| jukebox_error(binding, error, request))?;
    if queue.status.playing {
        binding
            .client
            .jukebox_start()
            .await
            .map_err(|error| jukebox_error(binding, error, request))?;
    } else {
        binding
            .client
            .jukebox_stop()
            .await
            .map_err(|error| jukebox_error(binding, error, request))?;
    }
    Ok(current + 1)
}

async fn jukebox_load_resolved_items_request(
    binding: &SubsonicBinding,
    request: &LoadResolvedItemsRequest,
) -> Result<Option<LoadResolvedItemsResponse>> {
    let error = |error| jukebox_error(binding, error, LoadResolvedItemsRequest::wire_name());
    let ids = subsonic_track_ids(binding, request)?;
    let references = ids.iter().map(String::as_str).collect::<Vec<_>>();
    let start = load_start(request)?;
    let insertion = match request.mode {
        PlaylistOperateMode::Replace if references.is_empty() => {
            binding.client.jukebox_clear().await.map_err(error)?;
            0
        }
        PlaylistOperateMode::Replace => {
            binding
                .client
                .jukebox_set(&references)
                .await
                .map_err(error)?;
            0
        }
        PlaylistOperateMode::AppendToEnd => {
            let end = if start.is_some() {
                binding
                    .client
                    .jukebox_get()
                    .await
                    .map_err(error)?
                    .playlist
                    .songs
                    .len() as u32
            } else {
                0
            };
            binding
                .client
                .jukebox_add(&references)
                .await
                .map_err(error)?;
            end
        }
        PlaylistOperateMode::PlayNext if references.is_empty() => 0,
        PlaylistOperateMode::PlayNext => jukebox_insert_after_current(binding, &ids).await?,
    };
    if let Some((index, offset)) = start {
        binding
            .client
            .jukebox_skip(insertion + index, offset)
            .await
            .map_err(error)?;
        if load_plays(request) {
            binding.client.jukebox_start().await.map_err(error)?;
        } else {
            binding.client.jukebox_stop().await.map_err(error)?;
        }
    }
    Ok(Some(LoadResolvedItemsResponse {
        call_id: request.call_id,
        result: Default::default(),
    }))
}

async fn jukebox_refresh_session_request(
    binding: &SubsonicBinding,
    _: &RefreshSessionRequest,
) -> Result<Option<()>> {
    binding.publish_jukebox_snapshot().await?;
    Ok(Some(()))
}

/// The jukebox keeps no queue revision; the one this device records with each snapshot
/// is what a following load names.
async fn jukebox_playlist_revision_request(
    binding: &SubsonicBinding,
    request: &PlaylistRevisionRequest,
) -> Result<Option<PlaylistRevisionResponse>> {
    let (_, revision) = binding.jukebox_snapshot().await?;
    Ok(Some(PlaylistRevisionResponse {
        revision,
        call_id: request.call_id,
    }))
}

async fn jukebox_session_snapshot_request(
    binding: &SubsonicBinding,
    request: &SessionSnapshotRequest,
) -> Result<Option<SessionSnapshotResponse>> {
    let (jukebox, revision) = binding.jukebox_snapshot().await?;
    let items = jukebox
        .playlist
        .songs
        .into_iter()
        .map(|song| PlayingItemRef::LibraryTrack {
            library_id: binding.library_id.to_string(),
            track: song.id,
        })
        .collect();
    Ok(Some(SessionSnapshotResponse {
        items,
        index: jukebox.status.current_index,
        position_seconds: f64::from(jukebox.status.position_secs.unwrap_or_default()),
        playback_mode: PlaybackMode::Sequential,
        playing: jukebox.status.playing,
        revision,
        call_id: request.call_id,
    }))
}

/// The items of a kept list, under the ids this library gives them, each as the catalog holds it:
/// a read since the list's, such as one of the item's own page, keeps what it found. The catalog
/// keeps the items it did not hold.
fn numbered<T: Clone>(
    ids: &mut RemoteIdMap<SubsonicIdKind>,
    held: &mut HashMap<i32, T>,
    kind: SubsonicIdKind,
    items: &[T],
    remote_id: fn(&T) -> &str,
) -> Result<Vec<(i32, T)>> {
    items
        .iter()
        .map(|item| {
            let id = ids.intern(kind, remote_id(item))?;
            Ok((id, held.entry(id).or_insert_with(|| item.clone()).clone()))
        })
        .collect()
}

/// The albums of a kept list, numbered as `numbered` does.
fn numbered_albums(
    binding: &SubsonicBinding,
    albums: &[SubsonicAlbum],
) -> Result<Vec<(i32, SubsonicAlbum)>> {
    let mut catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    let catalog = &mut *catalog;
    numbered(
        &mut catalog.ids,
        &mut catalog.albums,
        SubsonicIdKind::Album,
        albums,
        |album| album.id.as_str(),
    )
}

/// The artists, in the order the server lists them, from the kept list.
async fn listed_artists(binding: &SubsonicBinding) -> Result<Vec<(i32, SubsonicArtist)>> {
    let artists = binding
        .lists
        .artists
        .get(Instant::now())
        .await
        .map_err(|error| subsonic_request_error(binding, error))?;
    let mut catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    let catalog = &mut *catalog;
    numbered(
        &mut catalog.ids,
        &mut catalog.artists,
        SubsonicIdKind::Artist,
        &artists,
        |artist| artist.id.as_str(),
    )
}

/// Every album, by name, from the kept list.
async fn listed_albums(binding: &SubsonicBinding) -> Result<Vec<(i32, SubsonicAlbum)>> {
    let albums = binding
        .lists
        .albums
        .get(Instant::now())
        .await
        .map_err(|error| subsonic_request_error(binding, error))?;
    numbered_albums(binding, &albums)
}

/// Every album by name, each once, read page after page, for the kept list. A short page ends
/// the list, and so does a page that adds no album, as a server that ignores the offset repeats
/// one.
pub(crate) async fn read_albums_by_name(
    client: &SubsonicClient,
) -> Result<Vec<SubsonicAlbum>, SubsonicError> {
    let mut albums = Vec::new();
    let mut listed = HashSet::new();
    let mut offset = 0_u32;
    loop {
        let page = client.album_list(500, offset).await?;
        let count = u32::try_from(page.len()).unwrap_or(u32::MAX);
        let before = albums.len();
        albums.extend(
            page.into_iter()
                .filter(|album| listed.insert(album.id.clone())),
        );
        if count < 500 || albums.len() == before {
            return Ok(albums);
        }
        // A list longer than an offset can name ends there.
        let Some(next) = offset.checked_add(count) else {
            return Ok(albums);
        };
        offset = next;
    }
}

async fn collection_groups(
    binding: &SubsonicBinding,
    collection_type: CollectionType,
    group_titles: &[String],
) -> Result<Vec<CollectionGroup>> {
    let entries: Vec<(i32, String, String, Option<String>)> = match collection_type {
        CollectionType::Artist => listed_artists(binding)
            .await?
            .into_iter()
            .map(|(id, artist)| {
                let group = generate_group_name(&artist.name);
                (id, artist.name, group, artist.cover_art)
            })
            .collect(),
        CollectionType::Album => listed_albums(binding)
            .await?
            .into_iter()
            .map(|(id, album)| {
                let group = generate_group_name(&album.name);
                (id, album.name, group, album.cover_art)
            })
            .collect(),
        CollectionType::Playlist => interned_playlists(binding)
            .await?
            .into_iter()
            .map(|(id, playlist)| {
                let group = generate_group_name(&playlist.name);
                (id, playlist.name, group, playlist.cover_art)
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
    let mut groups = Vec::with_capacity(group_titles.len());
    for title in group_titles {
        let mut collections: Vec<_> = entries
            .iter()
            .filter(|(_, _, group, _)| group == title)
            .map(|(id, name, _, _)| Collection {
                id: *id,
                name: name.clone(),
                queries: Vec::new(),
                collection_type,
                cover_art_map: Default::default(),
                // A Subsonic playlist can be renamed and deleted; artists and albums cannot change.
                readonly: collection_type != CollectionType::Playlist,
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
            )
            .await?;
        }
        groups.push(CollectionGroup {
            group_title: title.clone(),
            collections,
        });
    }
    Ok(groups)
}

pub(crate) async fn fetch_collection_group_summary_request(
    binding: &SubsonicBinding,
    request: &FetchCollectionGroupSummaryRequest,
) -> Result<Option<CollectionGroupSummaryResponse>> {
    let names: Vec<String> = match request.collection_type {
        CollectionType::Artist => listed_artists(binding)
            .await?
            .into_iter()
            .map(|(_, artist)| artist.name)
            .collect(),
        CollectionType::Album => listed_albums(binding)
            .await?
            .into_iter()
            .map(|(_, album)| album.name)
            .collect(),
        CollectionType::Playlist => interned_playlists(binding)
            .await?
            .into_iter()
            .map(|(_, playlist)| playlist.name)
            .collect(),
        _ => {
            return Err(ExpectedRequestError::Unsupported {
                request: FetchCollectionGroupSummaryRequest::wire_name(),
                reason: UnsupportedReason::NotImplemented,
            }
            .into());
        }
    };
    let mut groups = BTreeMap::<String, i32>::new();
    for name in &names {
        *groups.entry(generate_group_name(name)).or_default() += 1;
    }
    Ok(Some(CollectionGroupSummaryResponse {
        call_id: None,
        collection_type: request.collection_type,
        groups: groups
            .into_iter()
            .map(|(group_title, count)| CollectionGroupSummary { group_title, count })
            .collect(),
        reserved_groups: Vec::new(),
    }))
}

pub(crate) async fn fetch_collection_groups_request(
    binding: &SubsonicBinding,
    request: &FetchCollectionGroupsRequest,
) -> Result<Option<FetchCollectionGroupsResponse>> {
    Ok(Some(FetchCollectionGroupsResponse {
        call_id: None,
        groups: collection_groups(binding, request.collection_type, &request.group_titles).await?,
    }))
}

/// The Subsonic API's cap on the number of songs one `getRandomSongs` request returns.
const SUBSONIC_RANDOM_SONGS_MAX: u32 = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DetailQuery {
    Artist(i32),
    Album(i32),
    /// The cover wall's query: random tracks that have a cover.
    RandomCovers(u32),
}

fn intern_songs(binding: &SubsonicBinding, songs: &[SubsonicSong]) -> Result<()> {
    let mut catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    for song in songs {
        let id = catalog.ids.intern(SubsonicIdKind::Track, &song.id)?;
        catalog.songs.insert(id, song.clone());
    }
    Ok(())
}

async fn detail_songs(binding: &SubsonicBinding, query: DetailQuery) -> Result<Vec<SubsonicSong>> {
    let albums = match query {
        DetailQuery::RandomCovers(count) => {
            let mut songs = binding
                .client
                .random_songs(count)
                .await
                .map_err(|error| subsonic_request_error(binding, error))?;
            songs.retain(|song| song.cover_art.is_some());
            intern_songs(binding, &songs)?;
            return Ok(songs);
        }
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
                .map_err(|error| subsonic_request_error(binding, error))?;
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
            .map_err(|error| subsonic_request_error(binding, error))?;
        album.songs.sort_by_key(|song| {
            (
                song.disc.unwrap_or_default(),
                song.track.unwrap_or_default(),
            )
        });
        intern_songs(binding, &album.songs)?;
        songs.extend(album.songs);
    }
    Ok(songs)
}

/// Waits until every cover of `songs` is in the local cover cache, and returns the IDs of the
/// covers that are now cached. A cover that cannot be fetched is left out rather than failing
/// the others.
async fn cache_covers(binding: &SubsonicBinding, songs: &[SubsonicSong]) -> HashSet<String> {
    let cover_ids = songs
        .iter()
        .filter_map(|song| song.cover_art.clone())
        .collect();
    binding
        .covers
        .cache_covers(&binding.client, cover_ids)
        .await
}

/// A page of the whole song list, in the order the server's empty search gives them.
async fn read_song_page(
    client: &SubsonicClient,
    offset: u32,
    count: u32,
) -> Result<Vec<SubsonicSong>, SubsonicError> {
    Ok(client
        .search_with_song_offset(
            "",
            SearchLimits {
                songs: count,
                ..Default::default()
            },
            offset,
        )
        .await?
        .songs)
}

// A resolve takes its tracks from the song list read for the kept one.
const _: () = assert!(mixes::MAX_COVER_POOL >= RESOLVED_TRACK_LIMIT);

/// The first songs of the whole song list, at most [`mixes::MAX_COVER_POOL`], each once, for the
/// kept list. A cover filter has to look at every song, and the pages of its list are asked for
/// one after another.
pub(crate) async fn read_song_list(
    client: &SubsonicClient,
) -> Result<Vec<SubsonicSong>, SubsonicError> {
    Ok(mixes::first_of_each_key(
        read_songs_from(client, 0, mixes::MAX_COVER_POOL).await?,
        |song| song.id.clone(),
    ))
}

/// A song of a kept song list as the catalog holds it: a read since the list's, such as one of
/// the song itself, keeps what it found.
fn held_song<'a>(catalog: &'a SubsonicCatalog, song: &'a SubsonicSong) -> &'a SubsonicSong {
    catalog
        .ids
        .lookup(SubsonicIdKind::Track, &song.id)
        .and_then(|id| catalog.songs.get(&id))
        .unwrap_or(song)
}

pub(crate) async fn mix_query_request(
    binding: &SubsonicBinding,
    request: &MixQueryRequest,
) -> Result<Option<MixQueryResponse>> {
    let planned = mixes::plan(&request.queries, MixQueryRequest::wire_name())?;
    let cover_only = planned.cover_only;
    let cover_wall = matches!(planned.plan, Plan::Random(_));
    // The cover filter of every song pages through the kept song list.
    let from_kept_list = cover_only && matches!(planned.plan, Plan::All);
    let cursor = usize::try_from(request.cursor.max(0)).unwrap_or(usize::MAX);
    let page_size = usize::try_from(request.page_size.max(0)).unwrap_or(usize::MAX);
    // Songs without a cover are dropped before the page is cut, so a page of a cover filter is
    // full of covers.
    let paged = |songs: Vec<SubsonicSong>| -> Vec<SubsonicSong> {
        songs
            .into_iter()
            .filter(|song| !cover_only || song.cover_art.is_some())
            .skip(cursor)
            .take(page_size)
            .collect()
    };
    let mut songs = match planned.plan {
        Plan::Random(count) if cover_only => {
            // `getRandomSongs` takes no seed, so the pages of a seeded random list could not
            // agree.
            if request.random_seed.is_some() {
                return Err(mixes::unsupported_query(MixQueryRequest::wire_name()));
            }
            let count = u32::try_from(count)
                .unwrap_or(u32::MAX)
                .clamp(1, SUBSONIC_RANDOM_SONGS_MAX);
            paged(detail_songs(binding, DetailQuery::RandomCovers(count)).await?)
        }
        // Only the cover wall draws at random, and it keeps to the tracks that have a cover.
        Plan::Random(_) => return Err(mixes::unsupported_query(MixQueryRequest::wire_name())),
        // Subsonic builds no artist page.
        Plan::ArtistPage { .. } => {
            return Err(mixes::unsupported_query(MixQueryRequest::wire_name()));
        }
        Plan::Collection(MixTarget::Album(id)) => {
            paged(detail_songs(binding, DetailQuery::Album(id)).await?)
        }
        Plan::Collection(MixTarget::Artist(id)) => {
            paged(detail_songs(binding, DetailQuery::Artist(id)).await?)
        }
        Plan::Collection(MixTarget::Playlist(id)) => paged(playlist_songs(binding, id).await?),
        Plan::All if !cover_only => {
            songs_from(binding, cursor, page_size, MixQueryRequest::wire_name()).await?
        }
        // The server cannot be asked for songs with a cover, so the kept song list, a bounded
        // pool of the songs, is filtered here.
        Plan::All => {
            let songs = binding
                .lists
                .songs
                .get(Instant::now())
                .await
                .map_err(|error| {
                    empty_search_error(binding, error, MixQueryRequest::wire_name())
                })?;
            let catalog = binding
                .catalog
                .lock()
                .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
            // The pages count through the songs that had a cover when the list was read, so each
            // keeps its songs whatever a later read finds. A song whose cover a later read found
            // gone is left out of its page.
            songs
                .iter()
                .filter(|song| song.cover_art.is_some())
                .skip(cursor)
                .take(page_size)
                .map(|song| held_song(&catalog, song))
                .filter(|song| song.cover_art.is_some())
                .cloned()
                .collect()
        }
        Plan::Items(items) => paged(items_songs(binding, &items).await?),
    };
    // The cover wall shows only covers and reads them once, so its covers must be on disk
    // before it gets the tracks. Track lists show a cover whenever its background fetch lands.
    if cover_wall {
        let cached = cache_covers(binding, &songs).await;
        songs.retain(|song| {
            song.cover_art
                .as_ref()
                .is_some_and(|cover| cached.contains(cover))
        });
    }
    let (mut files, covers) = if from_kept_list {
        translate_kept_songs(binding, &songs)?
    } else {
        translate_songs(binding, &songs)?
    };
    let cover_art_map = if cover_wall {
        // A batch larger than the cache can evict a cover cached above before the map is built.
        // The wall reads its covers once, so a track whose cover is gone is left out, and the
        // cover is not fetched again.
        let (map, _) = cached_cover_map(binding, covers).await?;
        files.retain(|file| map.contains_key(&file.id));
        map
    } else {
        subsonic_cover_map(binding, covers).await?
    };
    Ok(Some(MixQueryResponse {
        call_id: None,
        files,
        cover_art_map,
    }))
}

/// Songs asked for per search page while resolving the tracks page.
const RESOLVE_PAGE_SIZE: u32 = 500;

/// The server's first `limit` songs in the order the tracks page lists them, read for
/// `request`.
async fn tracks_page_songs(
    binding: &SubsonicBinding,
    limit: usize,
    request: &'static str,
) -> Result<Vec<SubsonicSong>> {
    songs_from(binding, 0, limit, request).await
}

/// The server's songs in the order the tracks page lists them from `offset` on, at most `limit`
/// of them, read for `request`.
async fn songs_from(
    binding: &SubsonicBinding,
    offset: usize,
    limit: usize,
    request: &'static str,
) -> Result<Vec<SubsonicSong>> {
    read_songs_from(&binding.client, offset, limit)
        .await
        .map_err(|error| empty_search_error(binding, error, request))
}

/// The server's songs in the order the tracks page lists them from `offset` on, at most `limit`
/// of them. Only an empty page ends the list: a server may return fewer songs than a page asks
/// for. No page asks for more songs than are still wanted, and a list longer than an offset can
/// name ends there.
async fn read_songs_from(
    client: &SubsonicClient,
    offset: usize,
    limit: usize,
) -> Result<Vec<SubsonicSong>, SubsonicError> {
    let mut songs = Vec::new();
    while songs.len() < limit {
        let Ok(page_offset) = u32::try_from(offset.saturating_add(songs.len())) else {
            break;
        };
        let page = read_song_page(
            client,
            page_offset,
            u32::try_from(limit - songs.len())
                .map_or(RESOLVE_PAGE_SIZE, |left| left.min(RESOLVE_PAGE_SIZE)),
        )
        .await?;
        if page.is_empty() {
            break;
        }
        songs.extend(page);
    }
    songs.truncate(limit);
    Ok(songs)
}

/// The songs of single tracks and collections together, as `mixes::union_order` lists them,
/// each collection's as its own page lists them and each song once. An id the binding never
/// gave names no song, as for a local library.
async fn items_songs(binding: &SubsonicBinding, items: &[MixItem]) -> Result<Vec<SubsonicSong>> {
    let mut songs = Vec::new();
    for item in mixes::union_order(items) {
        match item {
            MixItem::Track(id) => {
                let known = binding
                    .catalog
                    .lock()
                    .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?
                    .ids
                    .resolve(SubsonicIdKind::Track, id)
                    .is_ok();
                if known {
                    songs.push(song_by_id(binding, id).await?);
                }
            }
            MixItem::Collection(MixTarget::Album(id)) => {
                songs.extend(detail_songs(binding, DetailQuery::Album(id)).await?)
            }
            MixItem::Collection(MixTarget::Artist(id)) => {
                songs.extend(detail_songs(binding, DetailQuery::Artist(id)).await?)
            }
            MixItem::Collection(MixTarget::Playlist(id)) => {
                songs.extend(playlist_songs(binding, id).await?)
            }
        }
    }
    Ok(mixes::first_of_each_key(songs, |song| song.id.clone()))
}

/// Songs in wire form, and the `(media file id, cover id)` pairs of their covers.
type WireSongs = (Vec<MediaFile>, Vec<(i32, String)>);

/// Numbers songs and their covers and builds their wire form.
fn translate_songs(binding: &SubsonicBinding, songs: &[SubsonicSong]) -> Result<WireSongs> {
    let mut catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    let mut files = Vec::with_capacity(songs.len());
    let mut covers = Vec::new();
    for song in songs {
        let id = catalog.ids.intern(SubsonicIdKind::Track, &song.id)?;
        catalog.songs.insert(id, song.clone());
        let cover_art_id = song
            .cover_art
            .as_deref()
            .map(|cover| catalog.ids.intern(SubsonicIdKind::CoverArt, cover))
            .transpose()?;
        files.push(subsonic_media_file(binding, id, cover_art_id, song));
        covers.extend(song.cover_art.clone().map(|cover| (id, cover)));
    }
    Ok((files, covers))
}

/// Numbers songs of a kept list and builds their wire form from what the catalog holds of them,
/// which a read since the list's may have brought up to date. The catalog takes only the songs it
/// did not hold, so a kept list never writes an older copy over a newer one.
fn translate_kept_songs(binding: &SubsonicBinding, songs: &[SubsonicSong]) -> Result<WireSongs> {
    let mut catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    let mut files = Vec::with_capacity(songs.len());
    let mut covers = Vec::new();
    for song in songs {
        let id = catalog.ids.intern(SubsonicIdKind::Track, &song.id)?;
        let song = catalog
            .songs
            .entry(id)
            .or_insert_with(|| song.clone())
            .clone();
        let cover_art_id = song
            .cover_art
            .as_deref()
            .map(|cover| catalog.ids.intern(SubsonicIdKind::CoverArt, cover))
            .transpose()?;
        files.push(subsonic_media_file(binding, id, cover_art_id, &song));
        covers.extend(song.cover_art.clone().map(|cover| (id, cover)));
    }
    Ok((files, covers))
}

impl MixSource for SubsonicBinding {
    fn library_id(&self) -> String {
        self.library_id.to_string()
    }

    fn collection_tracks(&self, target: MixTarget) -> ServiceFuture<'_, Vec<String>> {
        Box::pin(async move {
            let songs = match target {
                MixTarget::Album(id) => detail_songs(self, DetailQuery::Album(id)).await?,
                MixTarget::Artist(id) => detail_songs(self, DetailQuery::Artist(id)).await?,
                // Read afresh, so the entries play as the playlist holds them now, repeats
                // included.
                MixTarget::Playlist(id) => playlist_songs(self, id).await?,
            };
            Ok(songs.into_iter().map(|song| song.id).collect())
        })
    }

    fn all_tracks(&self, limit: usize) -> ServiceFuture<'_, Vec<String>> {
        Box::pin(async move {
            // What a resolve plays or queues must be what the server has now, so it never takes
            // the kept list: it waits for the list as the server lists it now, joining a read in
            // flight, and the cover filter and the home keep what it read. The kept list holds
            // more songs than a resolve takes.
            let songs = self
                .lists
                .songs
                .current(Instant::now())
                .await
                .map_err(|error| {
                    empty_search_error(self, error, ResolveMixQueryRequest::wire_name())
                })?;
            Ok(songs
                .iter()
                .take(limit)
                .map(|song| song.id.clone())
                .collect())
        })
    }

    fn track_key(&self, id: i32) -> Result<Option<String>> {
        Ok(self
            .catalog
            .lock()
            .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?
            .ids
            .resolve(SubsonicIdKind::Track, id)
            .ok()
            .map(str::to_owned))
    }
}

/// Resolves the track list a track was clicked in. As in a local library, this library's
/// tracks in the caller's queue are left out, and when the clicked track is not in the
/// result the fallback tracks play instead. A fallback or clicked track of another library is
/// refused before the server is asked anything.
pub(crate) async fn resolve_mix_query_request(
    binding: &SubsonicBinding,
    request: &ResolveMixQueryRequest,
) -> Result<Option<ResolveMixQueryResponse>> {
    for item in request.fallback_playing_items.iter().chain(
        request
            .initial_playback_item
            .as_ref()
            .filter(|_| request.instantly_play),
    ) {
        subsonic_track_id(binding, Some(item))?;
    }
    Ok(Some(mixes::resolve_mix_query(binding, request).await?))
}

/// The tracks of songs, numbered and kept so their metadata is at hand later.
fn song_tracks(binding: &SubsonicBinding, songs: Vec<SubsonicSong>) -> Result<Vec<HomeTrack>> {
    let mut catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    songs
        .into_iter()
        .map(|song| {
            let id = catalog.ids.intern(SubsonicIdKind::Track, &song.id)?;
            let track = HomeTrack {
                remote: song.id.clone(),
                name: song.title.clone(),
                cover: song.cover_art.clone(),
            };
            catalog.songs.insert(id, song);
            Ok(track)
        })
        .collect()
}

/// The albums, oldest added first, from the kept list, which the server lists newest first.
async fn albums_oldest_first(binding: &SubsonicBinding) -> Result<Vec<HomeItem>> {
    let albums = binding
        .lists
        .newest_albums
        .get(Instant::now())
        .await
        .map_err(|error| subsonic_request_error(binding, error))?;
    let mut items = album_items(numbered_albums(binding, &albums)?);
    items.reverse();
    Ok(items)
}

/// The `limit` newest albums, newest first: from the kept list when there is one, and else from
/// as many of the server's pages as they take.
async fn newest_albums(binding: &SubsonicBinding, limit: usize) -> Result<Vec<HomeItem>> {
    if let Some(albums) = binding.lists.newest_albums.kept(Instant::now()).await {
        let newest = &albums[..limit.min(albums.len())];
        return Ok(album_items(numbered_albums(binding, newest)?));
    }
    let albums = read_newest_albums(&binding.client, limit)
        .await
        .map_err(|error| subsonic_request_error(binding, error))?;
    let mut catalog = binding
        .catalog
        .lock()
        .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
    albums
        .into_iter()
        .map(|album| {
            let id = catalog.ids.intern(SubsonicIdKind::Album, &album.id)?;
            let item = HomeItem {
                id,
                name: album.name.clone(),
                cover: album.cover_art.clone(),
            };
            catalog.albums.insert(id, album);
            Ok(item)
        })
        .collect()
}

/// The albums, newest added first, at most `limit` of them, read page after page. Only a page
/// that adds no album ends the list, as a server may return fewer albums than a page asks for,
/// or a page that repeats albums already listed, as one that ignores the offset does. A list
/// longer than an offset can name ends there.
pub(crate) async fn read_newest_albums(
    client: &SubsonicClient,
    limit: usize,
) -> Result<Vec<SubsonicAlbum>, SubsonicError> {
    let mut albums = Vec::new();
    let mut listed = HashSet::new();
    let mut offset = 0_u32;
    while albums.len() < limit {
        let size = u32::try_from(limit - albums.len()).map_or(500, |left| left.min(500));
        let page = client.album_list_by("newest", size, offset).await?;
        let count = u32::try_from(page.len()).unwrap_or(u32::MAX);
        let before = albums.len();
        albums.extend(
            page.into_iter()
                .filter(|album| listed.insert(album.id.clone())),
        );
        if albums.len() == before {
            break;
        }
        let Some(next) = offset.checked_add(count) else {
            break;
        };
        offset = next;
    }
    albums.truncate(limit);
    Ok(albums)
}

/// Numbered albums as the home lists them.
fn album_items(albums: Vec<(i32, SubsonicAlbum)>) -> Vec<HomeItem> {
    albums
        .into_iter()
        .map(|(id, album)| HomeItem {
            id,
            name: album.name,
            cover: album.cover_art,
        })
        .collect()
}

impl HomeSource for SubsonicBinding {
    fn collections(&self, collection_type: CollectionType) -> ServiceFuture<'_, Vec<HomeItem>> {
        Box::pin(async move {
            match collection_type {
                CollectionType::Album => albums_oldest_first(self).await,
                // Subsonic dates no artist, so they stay in the order the server lists them.
                CollectionType::Artist => Ok(listed_artists(self)
                    .await?
                    .into_iter()
                    .map(|(id, artist)| HomeItem {
                        id,
                        name: artist.name,
                        cover: artist.cover_art,
                    })
                    .collect()),
                CollectionType::Playlist => Ok(interned_playlists(self)
                    .await?
                    .into_iter()
                    .map(|(id, playlist)| HomeItem {
                        id,
                        name: playlist.name,
                        cover: playlist.cover_art,
                    })
                    .collect()),
                _ => Err(mixes::unsupported_query(ComplexQueryRequest::wire_name())),
            }
        })
    }

    fn editable(&self, collection_type: CollectionType) -> bool {
        // As on the playlists page: a playlist can be renamed and deleted, and artists and
        // albums cannot change.
        collection_type == CollectionType::Playlist
    }

    fn newest(
        &self,
        collection_type: CollectionType,
        limit: usize,
    ) -> ServiceFuture<'_, Option<Vec<HomeItem>>> {
        Box::pin(async move {
            match collection_type {
                CollectionType::Album => newest_albums(self, limit).await.map(Some),
                // Subsonic dates no artist, and lists its playlists all at once.
                _ => Ok(None),
            }
        })
    }

    fn tracks(&self, limit: usize) -> ServiceFuture<'_, Vec<HomeTrack>> {
        Box::pin(async move {
            // The first songs of the kept song list, else as many as the home shows.
            let Some(songs) = self.lists.songs.kept(Instant::now()).await else {
                let songs =
                    tracks_page_songs(self, limit, ComplexQueryRequest::wire_name()).await?;
                return song_tracks(self, songs);
            };
            // As the catalog holds them, under one lock, and only the songs it did not hold go
            // into it, so a kept list never writes an older copy over a newer one.
            let mut catalog = self
                .catalog
                .lock()
                .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
            songs
                .iter()
                .take(limit)
                .map(|song| {
                    let id = catalog.ids.intern(SubsonicIdKind::Track, &song.id)?;
                    let held = catalog.songs.entry(id).or_insert_with(|| song.clone());
                    Ok(HomeTrack {
                        remote: held.id.clone(),
                        name: held.title.clone(),
                        cover: held.cover_art.clone(),
                    })
                })
                .collect()
        })
    }

    fn newest_tracks(&self, limit: usize) -> ServiceFuture<'_, Vec<HomeTrack>> {
        Box::pin(async move {
            // The server lists its newest albums, not its newest songs, so the newest tracks are
            // those of the newest albums, each album's in its own order. An album holds a track
            // at least, so `limit` albums are enough.
            let mut songs = Vec::new();
            let mut listed = HashSet::new();
            for album in newest_albums(self, limit).await? {
                if songs.len() >= limit {
                    break;
                }
                let album_songs = detail_songs(self, DetailQuery::Album(album.id)).await?;
                songs.extend(
                    album_songs
                        .into_iter()
                        .filter(|song| listed.insert(song.id.clone())),
                );
            }
            songs.truncate(limit);
            song_tracks(self, songs)
        })
    }

    fn liked(&self) -> ServiceFuture<'_, Vec<HomeTrack>> {
        Box::pin(async move {
            let songs = self
                .client
                .starred_songs()
                .await
                .map_err(|error| subsonic_request_error(self, error))?;
            song_tracks(self, songs)
        })
    }

    fn number(&self, tracks: Vec<HomeTrack>) -> Result<Vec<HomeItem>> {
        let mut catalog = self
            .catalog
            .lock()
            .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
        tracks
            .into_iter()
            .map(|track| {
                Ok(HomeItem {
                    id: catalog.ids.intern(SubsonicIdKind::Track, &track.remote)?,
                    name: track.name,
                    cover: track.cover,
                })
            })
            .collect()
    }

    fn cover_paths(&self, covers: Vec<(i32, String)>) -> ServiceFuture<'_, HashMap<i32, String>> {
        Box::pin(subsonic_cover_map(self, covers))
    }

    fn random_seed(&self) -> u64 {
        home::rotating_seed(self.home_seed)
    }
}

pub(crate) async fn complex_query_request(
    binding: &SubsonicBinding,
    request: &ComplexQueryRequest,
) -> Result<Option<ComplexQueryResponse>> {
    Ok(Some(home::complex_query(binding, request).await?))
}

/// The server no longer accepts the sign-in of the library `binding` serves.
fn sign_in_required(binding: &SubsonicBinding) -> anyhow::Error {
    ExpectedRequestError::ServiceSignInRequired {
        record_id: binding.library_id.to_string(),
    }
    .into()
}

fn subsonic_request_error(binding: &SubsonicBinding, error: SubsonicError) -> anyhow::Error {
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
        } => sign_in_required(binding),
        SubsonicError::Server {
            kind: SubsonicServerError::NotFound,
            ..
        } => ExpectedError::NotFound {
            what: "Subsonic item",
        }
        .into(),
        SubsonicError::Server {
            kind:
                SubsonicServerError::Generic
                | SubsonicServerError::MissingParameter
                | SubsonicServerError::ClientTooOld
                | SubsonicServerError::ServerTooOld
                | SubsonicServerError::TokenAuthenticationNotSupported
                | SubsonicServerError::AuthenticationMechanismNotSupported
                | SubsonicServerError::ConflictingAuthentication
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
        SubsonicError::RuneVisualizerFeedUnsupported => ExpectedRequestError::Unsupported {
            request: "Subsonic visualizer feed",
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
        routed_native { $($native_request:ty),* $(,)? }
        jukebox_supported { $($jukebox_request:ty => $jukebox_handler:ident as $jukebox_method:ident),* $(,)? }
        player { $($player_request:ty),* $(,)? }
        items { $($item_request:ty => $item_slot:ident),* $(,)? }
    ) => {
        $(const _: () = assert!(
            <$item_request as crate::rpc::TargetRequest>::ROUTED_BY_ITEM,
            "item slots are only for requests routed by item",
        );)*

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
                $(slots.set::<$native_request>(Slot::Native);)*
                slots
            }

            /// What a Subsonic library answers about its own items, of the requests routed by
            /// item, whichever library is open. The others are unsupported for its items.
            pub(crate) fn subsonic_items() -> Self {
                let mut slots = Self::unimplemented();
                $(slots.set::<$item_request>(Slot::$item_slot);)*
                slots
            }
        }
    };
}

subsonic_requests! {
    supported {
        SetLikedRequest => set_liked_request as translate_set_liked_request,
        GetLikedRequest => get_liked_request as translate_get_liked_request,
        GetLyricByTrackIdRequest => get_lyric_request as translate_get_lyric_by_track_id_request,
        FetchAllPlaylistsRequest => fetch_all_playlists_request as translate_fetch_all_playlists_request,
        GetPlaylistByIdRequest => get_playlist_by_id_request as translate_get_playlist_by_id_request,
        CreatePlaylistRequest => create_playlist_request as translate_create_playlist_request,
        UpdatePlaylistRequest => update_playlist_request as translate_update_playlist_request,
        RemovePlaylistRequest => remove_playlist_request as translate_remove_playlist_request,
        AddItemToPlaylistRequest => add_item_to_playlist_request as translate_add_item_to_playlist_request,
        RemoveItemFromPlaylistRequest => remove_item_from_playlist_request as translate_remove_item_from_playlist_request,
        ReorderPlaylistItemPositionRequest => reorder_playlist_item_position_request as translate_reorder_playlist_item_position_request,
        FetchPlaylistOccurrencesRequest => fetch_playlist_occurrences_request as translate_fetch_playlist_occurrences_request,
        FetchMediaFileByIdsRequest => fetch_media_file_by_ids_request as translate_fetch_media_file_by_ids_request,
        FetchMediaFilesRequest => fetch_media_files_request as translate_fetch_media_files_request,
        GetMediaFilesCountRequest => get_media_files_count_request as translate_get_media_files_count_request,
        FetchParsedMediaFileRequest => fetch_parsed_media_file_request as translate_fetch_parsed_media_file_request,
        SearchMediaFileSummaryRequest => search_media_file_summary_request as translate_search_media_file_summary_request,
        SearchCollectionSummaryRequest => search_collection_summary_request as translate_search_collection_summary_request,
        FetchCollectionGroupSummaryRequest => fetch_collection_group_summary_request as translate_fetch_collection_group_summary_request,
        FetchCollectionGroupsRequest => fetch_collection_groups_request as translate_fetch_collection_groups_request,
        FetchCollectionByIdsRequest => fetch_collection_by_ids_request as translate_fetch_collection_by_ids_request,
        MixQueryRequest => mix_query_request as translate_mix_query_request,
        ResolveMixQueryRequest => resolve_mix_query_request as translate_resolve_mix_query_request,
        SearchForRequest => search_for_request as translate_search_for_request,
        ComplexQueryRequest => complex_query_request as translate_complex_query_request,
    }
    routed_native {
        SetLikedRequest,
        GetLikedRequest,
        GetLyricByTrackIdRequest,
        GetPrimaryColorByTrackIdRequest,
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
        SessionSnapshotRequest => jukebox_session_snapshot_request as translate_session_snapshot_request,
        PlaylistRevisionRequest => jukebox_playlist_revision_request as translate_playlist_revision_request,
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
        PlaylistRevisionRequest,
    }
    items {
        SetLikedRequest => Translate,
        GetLikedRequest => Translate,
        GetLyricByTrackIdRequest => Translate,
        // The hub takes the color of a streaming item from its cover in the library's cover cache.
        GetPrimaryColorByTrackIdRequest => Native,
    }
}

#[cfg(test)]
mod tests {
    use playback::player;
    use reqwest::Client;
    use serde_json::Map;
    use std::fs;
    use std::iter;
    use std::{
        collections::{HashMap, HashSet},
        net::SocketAddr,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tokio::sync as tokio_sync;
    use tokio::time as tokio_time;
    use url::Url;
    use url::form_urlencoded;
    use visualizer::Visualizer;

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
        add_item_to_playlist_request, complex_query_request, create_playlist_request,
        fetch_all_playlists_request, fetch_collection_by_ids_request,
        fetch_collection_group_summary_request, fetch_collection_groups_request,
        fetch_media_file_by_ids_request, fetch_media_files_request,
        fetch_parsed_media_file_request, fetch_playlist_occurrences_request, fetch_remote_playlist,
        get_liked_request, get_lyric_request, get_media_files_count_request,
        get_playlist_by_id_request, jukebox_error, mix_query_request,
        remove_item_from_playlist_request, remove_playlist_request,
        reorder_playlist_item_position_request, resolve_mix_query_request, resolve_playlist_id,
        search_collection_summary_request, search_for_request, search_media_file_summary_request,
        set_liked_request, subsonic_request_error, update_playlist_request,
    };
    use crate::{
        TypedRequest,
        macros::{RequestSlots, RequestTranslator},
        messages::{
            AddItemToPlaylistRequest, AudioAvailability, CallerQueue, CastPreflightRequest,
            CollectionType, CreateM3u8PlaylistRequest, CreatePlaylistRequest, ExcludedItems,
            FetchAllPlaylistsRequest, FetchCollectionByIdsRequest,
            FetchCollectionGroupSummaryRequest, FetchCollectionGroupsRequest,
            FetchDirectoryTreeRequest, FetchMediaFileByIdsRequest, FetchMediaFilesRequest,
            FetchParsedMediaFileRequest, FetchPlaylistOccurrencesRequest, GetLikedRequest,
            GetLyricByTrackIdRequest, GetMediaFilesCountRequest, GetPlaylistByIdRequest,
            HandoffLoadRequest, LoadPlay, LoadRequest, LoadResolvedItemsRequest, MixQuery,
            MixQueryRequest, MovePlaylistItemRequest, NextRequest, PauseRequest, PlayRequest,
            PlaybackMode, PlayingItemRef, PlaylistOccurrenceCursor, PlaylistOperateMode,
            PlaylistRevisionRequest, PreviousRequest, RefreshSessionRequest,
            RemoveItemFromPlaylistRequest, RemovePlaylistRequest, RemoveRequest,
            ReorderPlaylistItemPositionRequest, ResolveMixQueryRequest,
            SearchCollectionSummaryRequest, SearchForRequest, SearchMediaFileSummaryRequest,
            SeekRequest, SessionSnapshotRequest, SetAdaptiveSwitchingEnabledRequest,
            SetLikedRequest, SetPlaybackModeRequest, SetRealtimeFFTEnabledRequest, SwitchRequest,
            TakeBackHandoffBlockRequest, UpdatePlaylistRequest, VolumeRequest,
        },
        request_error::{ExpectedRequestError, classify},
        request_owner::Implementation,
        rpc::{Slot, TargetRequest, UnsupportedReason},
        subsonic_binding::{SubsonicBinding, SubsonicIdKind},
        utils::{Broadcaster, RinfRustSignal},
    };

    use crate::messages::{
        ComplexQuery, ComplexQueryRequest, LibraryHomeListMode, LibraryHomeQuery,
        ResolveMixQueryResponse,
    };
    use crate::messages::{RequestErrorKind, SearchField};

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
        starred: Arc<Mutex<HashSet<String>>>,
        empty_search_rejected: Arc<AtomicUsize>,
        song_page_cap: Arc<AtomicUsize>,
        random_song_sizes: Arc<Mutex<Vec<String>>>,
        no_random_songs: Arc<AtomicUsize>,
        // `error-cover` is answered only while this is not write-locked.
        error_cover_gate: Arc<tokio_sync::RwLock<()>>,
        /// The song list gives covers to the odd songs when this is not zero.
        listed_covers: Arc<AtomicUsize>,
        /// How many albums the server added after the two it always lists.
        newer_albums: Arc<AtomicUsize>,
        /// How many of its four songs the song list holds.
        listed_songs: Arc<AtomicUsize>,
        /// Gives the same full page of albums by name for every offset when this is not zero, as a
        /// server that ignores the offset does.
        repeated_albums: Arc<AtomicUsize>,
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
            starred: Arc::new(Mutex::new(HashSet::new())),
            empty_search_rejected: Arc::new(AtomicUsize::new(0)),
            song_page_cap: Arc::new(AtomicUsize::new(usize::MAX)),
            random_song_sizes: Arc::new(Mutex::new(Vec::new())),
            no_random_songs: Arc::new(AtomicUsize::new(0)),
            error_cover_gate: Arc::default(),
            listed_covers: Arc::new(AtomicUsize::new(0)),
            newer_albums: Arc::new(AtomicUsize::new(0)),
            listed_songs: Arc::new(AtomicUsize::new(4)),
            repeated_albums: Arc::new(AtomicUsize::new(0)),
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
            Url::parse(&url).unwrap(),
            SubsonicCredentials::Password {
                user: "alice".into(),
                password: "cover-secret-password".into(),
            },
            Client::new(),
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
            Arc::new(Visualizer::new()),
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
                form_urlencoded::parse(query.as_bytes())
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
                    jukebox.entries = params
                        .iter()
                        .filter(|(key, _)| key == "id")
                        .map(|(_, id)| mock_song(id))
                        .collect();
                    // A real server starts a replaced queue over, stopped.
                    jukebox.current_index = Some(0);
                    jukebox.position = 0;
                    jukebox.playing = false;
                    serde_json::json!({})
                }
                Some("add") => {
                    for (_, id) in params.iter().filter(|(key, _)| key == "id") {
                        jukebox.entries.push(mock_song(id));
                    }
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
            state.scrobble_calls.lock().unwrap().push(query.clone());
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
                let _open = state.error_cover_gate.read().await;
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
            "star" => {
                for (_, id) in params.iter().filter(|(key, _)| key == "id") {
                    state.starred.lock().unwrap().insert(id.clone());
                }
                serde_json::json!({})
            }
            "unstar" => {
                for (_, id) in params.iter().filter(|(key, _)| key == "id") {
                    state.starred.lock().unwrap().remove(id);
                }
                serde_json::json!({})
            }
            "getStarred2" => {
                serde_json::json!({"starred2":{"song":state.starred.lock().unwrap().iter().map(|id| mock_song(id)).collect::<Vec<_>>()}})
            }
            "getRandomSongs" => {
                let size = query.get("size").cloned().unwrap_or_default();
                state.random_song_sizes.lock().unwrap().push(size.clone());
                if state.no_random_songs.load(Ordering::SeqCst) != 0 {
                    return mock_response(serde_json::json!({"randomSongs":{}}));
                }
                let songs = [
                    ("song-1", Some("cover-a")),
                    ("song-2", Some("cover-a")),
                    ("song-3", None),
                    ("song-4", Some("error-cover")),
                ]
                .into_iter()
                .take(size.parse().unwrap_or_default())
                .map(|(id, cover)| {
                    let mut song = mock_song(id);
                    if let Some(cover) = cover {
                        song["coverArt"] = serde_json::json!(cover);
                    }
                    song
                })
                .collect::<Vec<_>>();
                serde_json::json!({"randomSongs":{"song":songs}})
            }
            "getSong" => {
                let id = query.get("id").map(String::as_str).unwrap_or_default();
                if id == "minimal" {
                    return mock_response(
                        serde_json::json!({"song":{"id":"minimal","title":"Minimal"}}),
                    );
                }
                let mut song = mock_song(id);
                if state.starred.lock().unwrap().contains(id) {
                    song["starred"] = serde_json::json!("2026-01-01T00:00:00Z");
                }
                serde_json::json!({"song":song})
            }
            "getLyrics" if query.get("title").is_some_and(|title| title == "empty") => {
                serde_json::json!({})
            }
            "getLyrics" => {
                serde_json::json!({"lyrics":{"artist":"Artist","title":"song-1","value":"[00:01.00]plain line"}})
            }
            "getLyricsBySongId" if query.get("id").is_some_and(|id| id == "plain-extension") => {
                serde_json::json!({"lyricsList":{"plainLyrics":{"lang":"en","text":"extension plain text"}}})
            }
            "getLyricsBySongId" => {
                serde_json::json!({"lyricsList":{"structuredLyrics":{"lang":"en","synced":true,"line":[{"start":1200,"value":"timed line"}]}}})
            }
            "getScanStatus" => serde_json::json!({"scanStatus":{"scanning":false,"count":4}}),
            "search3"
                if query.get("query").is_some_and(String::is_empty)
                    && state.empty_search_rejected.load(Ordering::SeqCst) != 0 =>
            {
                return Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":10,"message":"query is required"}}}"#,
                    ))
                    .unwrap();
            }
            "search3" => {
                if query.get("query").is_some_and(|value| !value.is_empty()) {
                    return mock_response(serde_json::json!({"searchResult3": {
                        "song": [
                            {"id":"song-1","title":"One","coverArt":"cover-1"},
                            {"id":"song-2","title":"Two","coverArt":"cover-2"}
                        ]
                    }}));
                }
                let songs = ["song-1", "song-2", "song-3", "song-4"];
                let songs = &songs[..state.listed_songs.load(Ordering::SeqCst)];
                let offset = query
                    .get("songOffset")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or_default();
                let count = query
                    .get("songCount")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or_default()
                    .min(state.song_page_cap.load(Ordering::SeqCst));
                let mut search = Map::new();
                if query
                    .get("songCount")
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or_default()
                    > 0
                {
                    search.insert(
                        "song".into(),
                        serde_json::json!(
                            songs
                                .iter()
                                .skip(offset)
                                .take(count)
                                .map(|id| listed_song(&state, id))
                                .collect::<Vec<_>>()
                        ),
                    );
                }
                if query
                    .get("albumCount")
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or_default()
                    > 0
                {
                    search.insert(
                        "album".into(),
                        serde_json::json!([{"id":"album-1","name":"Album One","artist":"Artist","coverArt":"cover-album"}]),
                    );
                }
                if query
                    .get("artistCount")
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or_default()
                    > 0
                {
                    search.insert(
                        "artist".into(),
                        serde_json::json!([{"id":"artist-1","name":"Artist One","coverArt":"cover-artist"}]),
                    );
                }
                serde_json::json!({"searchResult3":search})
            }
            // A newer album holds one track of its own.
            "getAlbum"
                if query
                    .get("id")
                    .is_some_and(|id| id.starts_with("album-new-")) =>
            {
                let id = &query["id"];
                serde_json::json!({"album":{
                    "id":id,"name":id,"artist":"Artist","artistId":"artist-1",
                    "song":[mock_song(&format!("{id}-track"))]
                }})
            }
            "getAlbum" => {
                // Out of order, and a later disc's first track has the lower track number.
                let songs = [("song-2", 2, 1), ("song-1", 1, 2)].map(|(id, disc, track)| {
                    let mut song = mock_song(id);
                    song["discNumber"] = serde_json::json!(disc);
                    song["track"] = serde_json::json!(track);
                    song
                });
                serde_json::json!({"album":{
                    "id":"album-1","name":"Album One","artist":"Artist","artistId":"artist-1",
                    "song":songs
                }})
            }
            // The newest albums come a page of one at a time, however many are asked for.
            "getAlbumList2"
                if query
                    .get("type")
                    .is_some_and(|kind| kind == "alphabeticalByName")
                    && state.repeated_albums.load(Ordering::SeqCst) != 0 =>
            {
                let albums = (0..500)
                    .map(|n| serde_json::json!({"id":format!("album-r-{n:03}"),"name":format!("Album {n:03}")}))
                    .collect::<Vec<_>>();
                serde_json::json!({"albumList2":{"album":albums}})
            }
            "getAlbumList2" if query.get("type").is_some_and(|kind| kind == "newest") => {
                let offset = query
                    .get("offset")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or_default();
                let newer = (1..=state.newer_albums.load(Ordering::SeqCst))
                    .rev()
                    .map(|n| serde_json::json!({"id":format!("album-new-{n}"),"name":format!("New {n}"),"artist":"Artist"}));
                let albums = newer
                    .chain([
                        serde_json::json!({"id":"album-2","name":"Album Two","artist":"Artist"}),
                        serde_json::json!(
                            {"id":"album-1","name":"Album One","artist":"Artist","coverArt":"cover-album"}
                        ),
                    ])
                    .collect::<Vec<_>>();
                let page = albums.iter().skip(offset).take(1).collect::<Vec<_>>();
                serde_json::json!({"albumList2":{"album":page}})
            }
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

    /// A song as the song list gives it: the odd ones have a cover when `listed_covers` is set.
    fn listed_song(state: &MockState, id: &str) -> serde_json::Value {
        let mut song = mock_song(id);
        if state.listed_covers.load(Ordering::SeqCst) != 0
            && (id.ends_with('1') || id.ends_with('3'))
        {
            song["coverArt"] = serde_json::json!(format!("cover-{id}"));
        }
        song
    }

    fn mock_song(id: &str) -> serde_json::Value {
        serde_json::json!({"id":id,"title":id,"artist":"Artist","artistId":"artist-1","album":"Album","albumId":"album-1","duration":180,"track":7})
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
        tokio_time::timeout(Duration::from_secs(2), async {
            loop {
                if cover_ids
                    .iter()
                    .all(|cover| binding.covers.path_for(cover).is_file())
                {
                    break;
                }
                tokio_time::sleep(Duration::from_millis(10)).await;
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
                fields: vec![SearchField::Track],
                n: 10,
                refresh: false,
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
        assert!(
            first
                .media_files
                .iter()
                .all(|file| file.cover_art_id.is_some_and(|id| id > 0))
        );
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
                fields: vec![SearchField::Track],
                n: 10,
                refresh: false,
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
        let after_add = occurrence_page(&binding, playlist_id, None, 128)
            .await
            .unwrap();
        assert_eq!(after_add.entries.len(), 4);
        assert_eq!(after_add.entries[3].occurrence_id, added.occurrence_id);

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

        let before_reorder = occurrence_page(&binding, playlist_id, None, 128)
            .await
            .unwrap();
        reorder_playlist_item_position_request(
            &binding,
            &ReorderPlaylistItemPositionRequest {
                playlist_id,
                occurrence_id: before_reorder.entries[0].occurrence_id.clone(),
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

    async fn occurrence_page(
        binding: &SubsonicBinding,
        playlist_id: i32,
        after: Option<PlaylistOccurrenceCursor>,
        page_size: u32,
    ) -> anyhow::Result<crate::messages::FetchPlaylistOccurrencesResponse> {
        let page = fetch_playlist_occurrences_request(
            binding,
            &FetchPlaylistOccurrencesRequest {
                request_id: "detail".into(),
                playlist_id,
                after,
                page_size,
                bake_cover_arts: false,
                call_id: None,
            },
        )
        .await?;
        Ok(page.expect("an occurrence listing always answers"))
    }

    fn listed(page: &crate::messages::FetchPlaylistOccurrencesResponse) -> Vec<(i32, &str)> {
        page.entries
            .iter()
            .map(|entry| {
                (
                    entry.position,
                    entry.file.as_ref().map_or("", |file| file.track.as_str()),
                )
            })
            .collect()
    }

    fn playlist_edits(server: &MockServer) -> usize {
        server
            .state
            .playlist_calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(endpoint, _)| endpoint != "getPlaylists" && endpoint != "getPlaylist")
            .count()
    }

    fn assert_playlist_changed(error: &anyhow::Error) {
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::PlaylistChanged)
        ));
        assert_eq!(
            crate::request_error::classify(error),
            crate::messages::RequestErrorKind::Conflict
        );
    }

    #[tokio::test]
    async fn playlist_detail_pages_through_repeats_and_edits_the_listed_occurrence() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        let playlist_id =
            fetch_all_playlists_request(&binding, &FetchAllPlaylistsRequest { call_id: None })
                .await
                .unwrap()
                .unwrap()
                .playlists[0]
                .id;

        let first = occurrence_page(&binding, playlist_id, None, 2)
            .await
            .unwrap();
        assert_eq!(first.request_id, "detail");
        assert_eq!(listed(&first), [(0, "song-1"), (1, "song-2")]);
        let first_file = first.entries[0].file.as_ref().unwrap();
        assert_eq!(first_file.title, "song-1");
        assert_eq!(first_file.library_id, binding.library_id.to_string());
        assert_eq!(first_file.audio_availability, AudioAvailability::RemoteOnly);
        let second = occurrence_page(&binding, playlist_id, first.next.clone(), 2)
            .await
            .unwrap();
        assert_eq!(listed(&second), [(2, "song-1")]);
        assert!(second.next.is_none());
        assert_eq!(second.entries[0].file.as_ref().unwrap().id, first_file.id);
        let broadcasts = server.broadcasts.load(Ordering::SeqCst);

        remove_item_from_playlist_request(
            &binding,
            &RemoveItemFromPlaylistRequest {
                playlist_id,
                occurrence_id: second.entries[0].occurrence_id.clone(),
                call_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(server.broadcasts.load(Ordering::SeqCst), broadcasts + 1);
        let after_remove = occurrence_page(&binding, playlist_id, None, 128)
            .await
            .unwrap();
        assert_eq!(listed(&after_remove), [(0, "song-1"), (1, "song-2")]);

        add_item_to_playlist_request(
            &binding,
            &AddItemToPlaylistRequest {
                playlist_id,
                media_file_id: first_file.id,
                position: None,
                call_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(server.broadcasts.load(Ordering::SeqCst), broadcasts + 2);
        let repeated = occurrence_page(&binding, playlist_id, None, 128)
            .await
            .unwrap();
        assert_eq!(
            listed(&repeated),
            [(0, "song-1"), (1, "song-2"), (2, "song-1")]
        );
        reorder_playlist_item_position_request(
            &binding,
            &ReorderPlaylistItemPositionRequest {
                playlist_id,
                occurrence_id: repeated.entries[2].occurrence_id.clone(),
                new_position: 1,
                call_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(server.broadcasts.load(Ordering::SeqCst), broadcasts + 3);
        assert_eq!(
            fetch_remote_playlist(&binding, playlist_id)
                .await
                .unwrap()
                .1,
            ["song-1", "song-1", "song-2"]
        );
    }

    #[tokio::test]
    async fn an_occurrence_listed_before_a_playlist_change_edits_nothing() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        let playlist_id =
            fetch_all_playlists_request(&binding, &FetchAllPlaylistsRequest { call_id: None })
                .await
                .unwrap()
                .unwrap()
                .playlists[0]
                .id;
        let first_page = occurrence_page(&binding, playlist_id, None, 1)
            .await
            .unwrap();
        let listing = occurrence_page(&binding, playlist_id, None, 128)
            .await
            .unwrap();
        assert_eq!(
            listed(&listing),
            [(0, "song-1"), (1, "song-2"), (2, "song-1")]
        );
        remove_item_from_playlist_request(
            &binding,
            &RemoveItemFromPlaylistRequest {
                playlist_id,
                occurrence_id: listing.entries[0].occurrence_id.clone(),
                call_id: None,
            },
        )
        .await
        .unwrap();
        let edits = playlist_edits(&server);
        let broadcasts = server.broadcasts.load(Ordering::SeqCst);

        let stale_remove = remove_item_from_playlist_request(
            &binding,
            &RemoveItemFromPlaylistRequest {
                playlist_id,
                occurrence_id: listing.entries[1].occurrence_id.clone(),
                call_id: None,
            },
        )
        .await
        .err()
        .expect("the listed song-2 is no longer at its listed position");
        assert_playlist_changed(&stale_remove);
        let stale_reorder = reorder_playlist_item_position_request(
            &binding,
            &ReorderPlaylistItemPositionRequest {
                playlist_id,
                occurrence_id: listing.entries[2].occurrence_id.clone(),
                new_position: 0,
                call_id: None,
            },
        )
        .await
        .err()
        .expect("the listed second song-1 is no longer at its listed position");
        assert_playlist_changed(&stale_reorder);
        let stale_page = occurrence_page(&binding, playlist_id, first_page.next.clone(), 2)
            .await
            .err()
            .expect("a page cannot continue a listing of the old order");
        assert_playlist_changed(&stale_page);
        assert_eq!(playlist_edits(&server), edits);
        assert_eq!(server.broadcasts.load(Ordering::SeqCst), broadcasts);
        assert_eq!(
            fetch_remote_playlist(&binding, playlist_id)
                .await
                .unwrap()
                .1,
            ["song-2", "song-1"]
        );

        let relisted = occurrence_page(&binding, playlist_id, None, 128)
            .await
            .unwrap();
        let remote_id = resolve_playlist_id(&binding, playlist_id).unwrap();
        binding
            .client
            .update_playlist(&remote_id, None, &[], &[0])
            .await
            .unwrap();
        let edits = playlist_edits(&server);
        let changed_elsewhere = remove_item_from_playlist_request(
            &binding,
            &RemoveItemFromPlaylistRequest {
                playlist_id,
                occurrence_id: relisted.entries[1].occurrence_id.clone(),
                call_id: None,
            },
        )
        .await
        .err()
        .expect("another client changed the playlist after it was listed");
        assert_playlist_changed(&changed_elsewhere);
        assert_eq!(playlist_edits(&server), edits);
        assert_eq!(
            fetch_remote_playlist(&binding, playlist_id)
                .await
                .unwrap()
                .1,
            ["song-1"]
        );
    }

    #[tokio::test]
    async fn playlist_detail_lists_an_empty_playlist_and_rejects_pages_it_cannot_name() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        let road =
            fetch_all_playlists_request(&binding, &FetchAllPlaylistsRequest { call_id: None })
                .await
                .unwrap()
                .unwrap()
                .playlists[0]
                .id;
        let fresh = create_playlist_request(
            &binding,
            &CreatePlaylistRequest {
                name: "Fresh".into(),
                group: String::new(),
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap()
        .playlist
        .id;
        let empty = occurrence_page(&binding, fresh, None, 128).await.unwrap();
        assert!(empty.entries.is_empty());
        assert!(empty.next.is_none());
        let road_listing = occurrence_page(&binding, road, None, 128).await.unwrap();
        let second_entry = road_listing.entries[1].occurrence_id.as_str();

        let cursor = |position, occurrence_id: &str| PlaylistOccurrenceCursor {
            position,
            occurrence_id: occurrence_id.into(),
        };
        let before = server.state.requests.lock().unwrap().len();
        for (after, page_size) in [
            (None, 0),
            (None, 129),
            (Some(cursor(0, second_entry)), 2),
            (Some(cursor(-1, "-1")), 2),
            (Some(cursor(0, "first")), 2),
            (Some(cursor(0, "0")), 2),
            (Some(cursor(0, "0@")), 2),
            (Some(cursor(0, "0@not-a-hash")), 2),
            (Some(cursor(0, "0@0123456789ABCDEF")), 2),
            (Some(cursor(0, "0@0123456789abcdef0")), 2),
        ] {
            let error = occurrence_page(&binding, road, after, page_size)
                .await
                .err()
                .expect("a page the listing cannot name is rejected");
            assert!(matches!(
                error.downcast_ref::<ExpectedRequestError>(),
                Some(ExpectedRequestError::InvalidInput { .. })
            ));
        }
        assert_eq!(server.state.requests.lock().unwrap().len(), before);
    }

    #[tokio::test]
    async fn malformed_occurrences_edit_nothing_and_reach_no_server() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        let playlist_id =
            fetch_all_playlists_request(&binding, &FetchAllPlaylistsRequest { call_id: None })
                .await
                .unwrap()
                .unwrap()
                .playlists[0]
                .id;
        let listed_fingerprint = occurrence_page(&binding, playlist_id, None, 128)
            .await
            .unwrap()
            .entries[0]
            .occurrence_id
            .split_once('@')
            .unwrap()
            .1
            .to_owned();
        let before = server.state.requests.lock().unwrap().len();
        for occurrence_id in [
            "0".to_owned(),
            "0@".to_owned(),
            "0@not-a-hash".to_owned(),
            format!("0@{}", listed_fingerprint.to_uppercase()),
            format!("0@{listed_fingerprint}0"),
            format!("first@{listed_fingerprint}"),
        ] {
            let remove = remove_item_from_playlist_request(
                &binding,
                &RemoveItemFromPlaylistRequest {
                    playlist_id,
                    occurrence_id: occurrence_id.clone(),
                    call_id: None,
                },
            )
            .await
            .err()
            .expect("a malformed occurrence is not removed");
            let reorder = reorder_playlist_item_position_request(
                &binding,
                &ReorderPlaylistItemPositionRequest {
                    playlist_id,
                    occurrence_id,
                    new_position: 1,
                    call_id: None,
                },
            )
            .await
            .err()
            .expect("a malformed occurrence is not moved");
            for error in [remove, reorder] {
                assert!(matches!(
                    error.downcast_ref::<ExpectedRequestError>(),
                    Some(ExpectedRequestError::InvalidInput { .. })
                ));
            }
        }
        assert_eq!(server.state.requests.lock().unwrap().len(), before);
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
            FetchPlaylistOccurrencesRequest::slot(&slots),
            CreatePlaylistRequest::slot(&slots),
            UpdatePlaylistRequest::slot(&slots),
            RemovePlaylistRequest::slot(&slots),
            AddItemToPlaylistRequest::slot(&slots),
            RemoveItemFromPlaylistRequest::slot(&slots),
            ReorderPlaylistItemPositionRequest::slot(&slots),
        ] {
            assert_eq!(slot, Slot::Translate);
        }
        assert_eq!(
            CreateM3u8PlaylistRequest::slot(&slots),
            Slot::Unsupported {
                reason: UnsupportedReason::NotImplemented
            }
        );
    }

    fn add_mock_playlist(server: &MockServer, id: &str, name: &str) {
        server.state.playlists.lock().unwrap().push(MockPlaylist {
            id: id.into(),
            name: name.into(),
            songs: Vec::new(),
        });
    }

    async fn playlist_ids(binding: &SubsonicBinding) -> HashMap<String, i32> {
        fetch_all_playlists_request(binding, &FetchAllPlaylistsRequest { call_id: None })
            .await
            .unwrap()
            .unwrap()
            .playlists
            .into_iter()
            .map(|playlist| (playlist.name, playlist.id))
            .collect()
    }

    #[tokio::test]
    async fn the_playlists_page_lists_the_server_playlists_by_first_letter() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        add_mock_playlist(&server, "playlist-2", "Aurora");
        add_mock_playlist(&server, "playlist-3", "rain");

        let summary = fetch_collection_group_summary_request(
            &binding,
            &FetchCollectionGroupSummaryRequest {
                call_id: None,
                collection_type: CollectionType::Playlist,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(summary.collection_type, CollectionType::Playlist);
        assert_eq!(
            summary
                .groups
                .iter()
                .map(|group| (group.group_title.as_str(), group.count))
                .collect::<Vec<_>>(),
            [("A", 1), ("R", 2)]
        );

        let groups = fetch_collection_groups_request(
            &binding,
            &FetchCollectionGroupsRequest {
                call_id: None,
                collection_type: CollectionType::Playlist,
                bake_cover_arts: false,
                group_titles: vec!["A".into(), "R".into()],
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            groups
                .groups
                .iter()
                .map(|group| (
                    group.group_title.as_str(),
                    group
                        .collections
                        .iter()
                        .map(|collection| collection.name.as_str())
                        .collect::<Vec<_>>()
                ))
                .collect::<Vec<_>>(),
            [("A", vec!["Aurora"]), ("R", vec!["Road", "rain"])]
        );
        let ids = playlist_ids(&binding).await;
        for collection in groups.groups.iter().flat_map(|group| &group.collections) {
            assert_eq!(collection.collection_type, CollectionType::Playlist);
            assert_eq!(collection.id, ids[&collection.name]);
            assert!(!collection.readonly);
        }
    }

    #[tokio::test]
    async fn playlists_are_fetched_by_id() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        add_mock_playlist(&server, "playlist-2", "Aurora");
        let ids = playlist_ids(&binding).await;
        let request = |ids: Vec<i32>| FetchCollectionByIdsRequest {
            collection_type: CollectionType::Playlist,
            bake_cover_arts: false,
            ids,
            call_id: None,
        };

        let fetched =
            fetch_collection_by_ids_request(&binding, &request(vec![ids["Road"], ids["Aurora"]]))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(fetched.collection_type, CollectionType::Playlist);
        assert_eq!(
            fetched
                .result
                .iter()
                .map(|collection| (collection.id, collection.name.as_str()))
                .collect::<Vec<_>>(),
            [(ids["Road"], "Road"), (ids["Aurora"], "Aurora")]
        );
        assert!(fetched.result.iter().all(|collection| {
            collection.collection_type == CollectionType::Playlist && !collection.readonly
        }));

        server
            .state
            .playlists
            .lock()
            .unwrap()
            .retain(|playlist| playlist.name != "Aurora");
        let fetched =
            fetch_collection_by_ids_request(&binding, &request(vec![ids["Road"], ids["Aurora"]]))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            fetched
                .result
                .iter()
                .map(|collection| collection.name.as_str())
                .collect::<Vec<_>>(),
            ["Road"]
        );

        let unknown = fetch_collection_by_ids_request(&binding, &request(vec![i32::MAX]))
            .await
            .err()
            .unwrap();
        assert!(matches!(
            unknown.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::InvalidInput { .. })
        ));
    }

    #[tokio::test]
    async fn search_finds_playlists_by_name() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        add_mock_playlist(&server, "playlist-2", "Aurora");
        add_mock_playlist(&server, "playlist-3", "rain");
        let ids = playlist_ids(&binding).await;
        let search = |query_str: &str, fields: Vec<SearchField>, n: i32| SearchForRequest {
            call_id: None,
            query_str: query_str.into(),
            fields,
            n,
            refresh: false,
        };

        let everything = search_for_request(&binding, &search("ROA*", Vec::new(), 10))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(everything.playlists, [ids["Road"]]);

        let first = search_for_request(&binding, &search("r", vec![SearchField::Playlist], 1))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.playlists, [ids["Road"]]);

        server.state.requests.lock().unwrap().clear();
        let tracks_only = search_for_request(&binding, &search("r", vec![SearchField::Track], 10))
            .await
            .unwrap()
            .unwrap();
        assert!(tracks_only.playlists.is_empty());
        assert_eq!(
            server.state.requests.lock().unwrap().as_slice(),
            ["search3"]
        );
    }

    #[tokio::test]
    async fn subsonic_likes_and_lyrics_translate_and_reject_cross_library_items() {
        let root = TempDir::new().unwrap();
        let (mut binding, server) = mock_subsonic(&root, false).await;
        let item = PlayingItemRef::LibraryTrack {
            library_id: binding.library_id.to_string(),
            track: "song-1".into(),
        };
        let request = SetLikedRequest {
            item: Some(item.clone()),
            liked: true,
            call_id: Some(9),
        };
        let response = set_liked_request(&binding, &request)
            .await
            .unwrap()
            .unwrap();
        assert!(response.liked);
        assert!(
            get_liked_request(
                &binding,
                &GetLikedRequest {
                    item: Some(item.clone()),
                    call_id: None
                }
            )
            .await
            .unwrap()
            .unwrap()
            .liked
        );
        let before = server.state.requests.lock().unwrap().len();
        let foreign_item = PlayingItemRef::LibraryTrack {
            library_id: Uuid::new_v4().to_string(),
            track: "song-1".into(),
        };
        assert!(
            set_liked_request(
                &binding,
                &SetLikedRequest {
                    item: Some(foreign_item.clone()),
                    liked: false,
                    call_id: None
                }
            )
            .await
            .is_err()
        );
        assert!(
            get_liked_request(
                &binding,
                &GetLikedRequest {
                    item: Some(foreign_item.clone()),
                    call_id: None
                }
            )
            .await
            .is_err()
        );
        assert!(
            get_lyric_request(
                &binding,
                &GetLyricByTrackIdRequest {
                    item: Some(foreign_item),
                    call_id: None
                }
            )
            .await
            .is_err()
        );
        assert_eq!(server.state.requests.lock().unwrap().len(), before);
        set_liked_request(
            &binding,
            &SetLikedRequest {
                item: Some(item.clone()),
                liked: false,
                call_id: None,
            },
        )
        .await
        .unwrap();
        assert!(
            !get_liked_request(
                &binding,
                &GetLikedRequest {
                    item: Some(item.clone()),
                    call_id: None
                }
            )
            .await
            .unwrap()
            .unwrap()
            .liked
        );

        binding
            .probe
            .get_mut()
            .unwrap()
            .extensions
            .push(("songLyrics".into(), vec![1]));
        let lyrics = get_lyric_request(
            &binding,
            &GetLyricByTrackIdRequest {
                item: Some(item.clone()),
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(lyrics.lines[0].start_time, 1200);
        assert_eq!(lyrics.lines[0].sections[0].content, "timed line");
        let plain_extension_item = PlayingItemRef::LibraryTrack {
            library_id: binding.library_id.to_string(),
            track: "plain-extension".into(),
        };
        let lyrics = get_lyric_request(
            &binding,
            &GetLyricByTrackIdRequest {
                item: Some(plain_extension_item),
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(lyrics.lines[0].start_time, 0);
        assert_eq!(lyrics.lines[0].sections[0].content, "extension plain text");
        binding.probe.get_mut().unwrap().extensions.clear();
        let lyrics = get_lyric_request(
            &binding,
            &GetLyricByTrackIdRequest {
                item: Some(item),
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(lyrics.lines[0].sections[0].content, "plain line");
        let empty = PlayingItemRef::LibraryTrack {
            library_id: binding.library_id.to_string(),
            track: "empty".into(),
        };
        let lyrics = get_lyric_request(
            &binding,
            &GetLyricByTrackIdRequest {
                item: Some(empty),
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert!(lyrics.lines.is_empty());
    }

    #[test]
    fn subsonic_likes_and_lyrics_capabilities_are_supported_without_extensions() {
        let slots = RequestSlots::subsonic(&SubsonicProbe {
            api_version: "1.16.1".into(),
            open_subsonic: false,
            server_type: None,
            server_version: None,
            extensions: Vec::new(),
            roles: SubsonicRoles::default(),
            roles_known: true,
        });
        assert_eq!(SetLikedRequest::slot(&slots), Slot::Native);
        assert_eq!(GetLikedRequest::slot(&slots), Slot::Native);
        // Lyrics are routed by item, as likes are: the open library leaves an item of another
        // library to that library, and the binding answers for its own items.
        assert_eq!(GetLyricByTrackIdRequest::slot(&slots), Slot::Native);
        let items = RequestSlots::subsonic_items();
        assert_eq!(GetLyricByTrackIdRequest::slot(&items), Slot::Translate);
        assert_eq!(SetLikedRequest::slot(&items), Slot::Translate);
        assert_eq!(GetLikedRequest::slot(&items), Slot::Translate);
    }

    #[test]
    fn subsonic_track_and_summary_capabilities_exclude_directory_tree() {
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
            FetchMediaFilesRequest::slot(&slots),
            GetMediaFilesCountRequest::slot(&slots),
            FetchParsedMediaFileRequest::slot(&slots),
            SearchMediaFileSummaryRequest::slot(&slots),
            SearchCollectionSummaryRequest::slot(&slots),
        ] {
            assert_eq!(slot, Slot::Translate);
        }
        assert_eq!(
            FetchDirectoryTreeRequest::slot(&slots),
            Slot::Unsupported {
                reason: UnsupportedReason::NotImplemented
            }
        );
    }

    #[tokio::test]
    async fn subsonic_track_pages_walk_every_song_once_in_order() {
        let root = TempDir::new().unwrap();
        let (binding, _) = mock_subsonic(&root, false).await;
        let mut songs = Vec::new();
        for cursor in [0, 2, 4] {
            songs.extend(
                fetch_media_files_request(
                    &binding,
                    &FetchMediaFilesRequest {
                        cursor,
                        page_size: 2,
                        bake_cover_arts: false,
                        call_id: Some(22),
                    },
                )
                .await
                .unwrap()
                .unwrap()
                .media_files
                .into_iter()
                .map(|file| file.track),
            );
        }
        assert_eq!(songs, ["song-1", "song-2", "song-3", "song-4"]);
    }

    #[tokio::test]
    async fn subsonic_track_count_matches_scan_status_song_total() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        let count = get_media_files_count_request(
            &binding,
            &GetMediaFilesCountRequest { call_id: Some(23) },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(count.count, 4);
        assert_eq!(count.call_id, Some(23));
        let requests = server.state.requests.lock().unwrap();
        assert_eq!(requests.as_slice(), ["getScanStatus"]);
    }

    #[tokio::test]
    async fn subsonic_track_details_map_server_fields_and_keep_missing_fields_empty() {
        let root = TempDir::new().unwrap();
        let (binding, _) = mock_subsonic(&root, false).await;
        let page = fetch_media_files_request(
            &binding,
            &FetchMediaFilesRequest {
                cursor: 0,
                page_size: 1,
                bake_cover_arts: false,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        let details = fetch_parsed_media_file_request(
            &binding,
            &FetchParsedMediaFileRequest {
                id: page.media_files[0].id,
                call_id: Some(24),
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(details.file.track, "song-1");
        assert_eq!(details.file.title, "song-1");
        assert_eq!(details.file.artist, "Artist");
        assert_eq!(details.file.album, "Album");
        assert_eq!(details.file.duration, 180.0);
        assert_eq!(details.file.track_number, Some(7));
        assert_eq!(details.album.name, "Album");
        assert_eq!(details.artists[0].name, "Artist");
        assert_eq!(details.call_id, Some(24));
        let missing_id = binding
            .catalog
            .lock()
            .unwrap()
            .ids
            .intern(SubsonicIdKind::Track, "minimal")
            .unwrap();
        let missing = fetch_parsed_media_file_request(
            &binding,
            &FetchParsedMediaFileRequest {
                id: missing_id,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert!(missing.artists.is_empty());
        assert_eq!(missing.album.id, 0);
        assert!(missing.album.name.is_empty());
    }

    #[tokio::test]
    async fn subsonic_tracks_are_playable_in_every_track_list() {
        let root = TempDir::new().unwrap();
        let (binding, _) = mock_subsonic(&root, false).await;
        let page = fetch_media_files_request(
            &binding,
            &FetchMediaFilesRequest {
                cursor: 0,
                page_size: 4,
                bake_cover_arts: false,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap()
        .media_files;
        let by_ids = fetch_media_file_by_ids_request(
            &binding,
            &FetchMediaFileByIdsRequest {
                call_id: None,
                ids: page.iter().map(|file| file.id).collect(),
                bake_cover_arts: false,
            },
        )
        .await
        .unwrap()
        .unwrap()
        .media_files;
        let album = search_collection_summary_request(
            &binding,
            &SearchCollectionSummaryRequest {
                collection_type: Some(CollectionType::Album),
                bake_cover_arts: Some(false),
                n: 1,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap()
        .result
        .remove(0);
        let album_tracks = mix_query_request(
            &binding,
            &MixQueryRequest {
                queries: vec![MixQuery {
                    operator: "lib::album".into(),
                    parameter: album.id.to_string(),
                }],
                cursor: 0,
                page_size: 10,
                bake_cover_arts: false,
                queue: CallerQueue::default(),
                random_seed: None,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap()
        .files;
        let summaries = search_media_file_summary_request(
            &binding,
            &SearchMediaFileSummaryRequest {
                n: 4,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap()
        .result;
        assert!(!page.is_empty() && !by_ids.is_empty() && !album_tracks.is_empty());
        assert!(!summaries.is_empty());
        let playable = AudioAvailability::RemoteOnly;
        for file in page.iter().chain(&by_ids).chain(&album_tracks) {
            assert_eq!(file.audio_availability, playable, "{}", file.track);
        }
        for summary in &summaries {
            assert_eq!(summary.audio_availability, playable, "{}", summary.name);
        }
    }

    fn library_track(library_id: Uuid, track: &str) -> PlayingItemRef {
        PlayingItemRef::LibraryTrack {
            library_id: library_id.to_string(),
            track: track.to_owned(),
        }
    }

    fn library_tracks(library_id: Uuid, tracks: &[&str]) -> Vec<PlayingItemRef> {
        tracks
            .iter()
            .map(|track| library_track(library_id, track))
            .collect()
    }

    /// The resolve request a track list sends when `initial` is clicked at `hint_position`.
    fn click_resolve(
        queries: &[(&str, &str)],
        hint_position: i32,
        initial: PlayingItemRef,
    ) -> ResolveMixQueryRequest {
        ResolveMixQueryRequest {
            queries: queries
                .iter()
                .map(|(operator, parameter)| MixQuery {
                    operator: (*operator).into(),
                    parameter: (*parameter).into(),
                })
                .collect(),
            hint_position,
            initial_playback_item: Some(initial),
            instantly_play: true,
            fallback_playing_items: Vec::new(),
            excluded: ExcludedItems::default(),
            call_id: Some(31),
        }
    }

    #[tokio::test]
    async fn the_tracks_page_resolves_past_a_server_page_cap() {
        let root = TempDir::new().unwrap();
        let (binding, server) =
            mock_subsonic_mode(&root, false, SubsonicOpenMode::PlayOnThisDevice).await;
        server.state.song_page_cap.store(3, Ordering::SeqCst);
        let library = binding.library_id;

        let resolved = click_resolve(&[("lib::all", "true")], 3, library_track(library, "song-4"))
            .translate(&binding)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            resolved.playing_items,
            library_tracks(library, &["song-1", "song-2", "song-3", "song-4"])
        );
        assert_eq!(resolved.start_index, Some(3));
    }

    #[tokio::test]
    async fn an_album_page_lists_and_plays_its_tracks_in_disc_and_track_order() {
        let root = TempDir::new().unwrap();
        let (binding, _) =
            mock_subsonic_mode(&root, false, SubsonicOpenMode::PlayOnThisDevice).await;
        let library = binding.library_id;
        let album = search_collection_summary_request(
            &binding,
            &SearchCollectionSummaryRequest {
                collection_type: Some(CollectionType::Album),
                bake_cover_arts: Some(false),
                n: 1,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap()
        .result
        .remove(0)
        .id
        .to_string();
        // The queries of the album page, `/albums/detail`, in either order.
        let page = [
            ("lib::album", album.as_str()),
            ("sort::track_number", "true"),
        ];
        for queries in [page, [page[1], page[0]]] {
            let listed = mix_query_request(&binding, &mix_request(&queries))
                .await
                .unwrap()
                .unwrap();
            let tracks = listed
                .files
                .iter()
                .map(|file| file.track.as_str())
                .collect::<Vec<_>>();
            assert_eq!(tracks, ["song-1", "song-2"], "{queries:?}");

            let clicked = click_resolve(&queries, 1, library_track(library, "song-2"))
                .translate(&binding)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                clicked.playing_items,
                library_tracks(library, &["song-1", "song-2"]),
                "{queries:?}"
            );
            assert_eq!(clicked.start_index, Some(1), "{queries:?}");
        }
    }

    #[tokio::test]
    async fn clicked_tracks_resolve_the_list_they_were_clicked_in() {
        let root = TempDir::new().unwrap();
        let (binding, server) =
            mock_subsonic_mode(&root, false, SubsonicOpenMode::PlayOnThisDevice).await;
        let library = binding.library_id;
        assert_eq!(
            ResolveMixQueryRequest::slot(&RequestSlots::subsonic(&binding.probe())),
            Slot::Translate
        );

        let all = click_resolve(&[("lib::all", "true")], 2, library_track(library, "song-3"))
            .translate(&binding)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            all.playing_items,
            library_tracks(library, &["song-1", "song-2", "song-3", "song-4"])
        );
        assert_eq!(all.start_index, Some(2));
        assert!(
            server
                .state
                .requests
                .lock()
                .unwrap()
                .iter()
                .all(|endpoint| endpoint == "search3")
        );

        let album = search_collection_summary_request(
            &binding,
            &SearchCollectionSummaryRequest {
                collection_type: Some(CollectionType::Album),
                bake_cover_arts: Some(false),
                n: 1,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap()
        .result
        .remove(0);
        let album_tracks = click_resolve(
            &[("lib::album", &album.id.to_string())],
            1,
            library_track(library, "song-2"),
        )
        .translate(&binding)
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            album_tracks.playing_items,
            library_tracks(library, &["song-1", "song-2"])
        );
        assert_eq!(album_tracks.start_index, Some(1));

        let mut search_result = click_resolve(&[], 1, library_track(library, "song-2"));
        search_result.fallback_playing_items = library_tracks(library, &["song-4", "song-2"]);
        let requests_before_search = server.state.requests.lock().unwrap().len();
        let search_result = search_result.translate(&binding).await.unwrap().unwrap();
        assert_eq!(
            search_result.playing_items,
            library_tracks(library, &["song-4", "song-2"])
        );
        assert_eq!(search_result.start_index, Some(1));
        assert_eq!(
            server.state.requests.lock().unwrap().len(),
            requests_before_search
        );
    }

    #[tokio::test]
    async fn a_playlist_resolves_to_its_entries_in_order_with_repeats() {
        let root = TempDir::new().unwrap();
        let (binding, server) =
            mock_subsonic_mode(&root, false, SubsonicOpenMode::PlayOnThisDevice).await;
        let library = binding.library_id;
        let playlist = playlist_ids(&binding).await["Road"].to_string();
        let road = [("lib::playlist", playlist.as_str())];
        let entries = library_tracks(library, &["song-1", "song-2", "song-1"]);

        let clicked = click_resolve(&road, 2, library_track(library, "song-1"))
            .translate(&binding)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(clicked.playing_items, entries);
        assert_eq!(clicked.start_index, Some(2));
        assert_eq!(
            server
                .state
                .requests
                .lock()
                .unwrap()
                .last()
                .map(String::as_str),
            Some("getPlaylist")
        );

        // Start Playing on the playlist's tile names no track.
        let mut started = click_resolve(&road, -1, library_track(library, "song-1"));
        started.initial_playback_item = None;
        let started = started.translate(&binding).await.unwrap().unwrap();
        assert_eq!(started.playing_items, entries);
        assert_eq!(started.start_index, Some(0));

        let mut queued = click_resolve(&road, -1, library_track(library, "song-1"));
        queued.initial_playback_item = None;
        queued.instantly_play = false;
        queued.excluded.items = vec![library_track(library, "song-2")];
        let queued = queued.translate(&binding).await.unwrap().unwrap();
        assert_eq!(
            queued.playing_items,
            library_tracks(library, &["song-1", "song-1"])
        );
        assert_eq!(queued.start_index, None);
    }

    #[tokio::test]
    async fn resolving_leaves_out_the_queue_and_falls_back_like_a_local_library() {
        let root = TempDir::new().unwrap();
        let (binding, _server) =
            mock_subsonic_mode(&root, false, SubsonicOpenMode::PlayOnThisDevice).await;
        let library = binding.library_id;
        let other_library = Uuid::new_v4();

        let mut leaves_out_queue =
            click_resolve(&[("lib::all", "true")], 2, library_track(library, "song-3"));
        leaves_out_queue.excluded.items = vec![
            library_track(library, "song-1"),
            library_track(other_library, "song-2"),
        ];
        let resolved = leaves_out_queue.translate(&binding).await.unwrap().unwrap();
        assert_eq!(
            resolved.playing_items,
            library_tracks(library, &["song-2", "song-3", "song-4"])
        );
        assert_eq!(resolved.start_index, Some(1));

        let mut queued_track =
            click_resolve(&[("lib::all", "true")], 0, library_track(library, "song-1"));
        queued_track.excluded.items = vec![library_track(library, "song-1")];
        queued_track.fallback_playing_items = vec![library_track(library, "song-1")];
        let resolved = queued_track.translate(&binding).await.unwrap().unwrap();
        assert_eq!(
            resolved.playing_items,
            vec![library_track(library, "song-1")]
        );
        assert_eq!(resolved.start_index, Some(0));

        let mut queued_only = click_resolve(
            &[("lib::all", "true")],
            -1,
            library_track(library, "song-9"),
        );
        queued_only.instantly_play = false;
        let resolved = queued_only.translate(&binding).await.unwrap().unwrap();
        assert_eq!(
            resolved.playing_items,
            library_tracks(library, &["song-1", "song-2", "song-3", "song-4"])
        );
        assert_eq!(resolved.start_index, None);
    }

    #[tokio::test]
    async fn resolving_rejects_lists_and_tracks_a_subsonic_library_cannot_name() {
        let root = TempDir::new().unwrap();
        let (binding, server) =
            mock_subsonic_mode(&root, false, SubsonicOpenMode::PlayOnThisDevice).await;

        let liked = click_resolve(
            &[("filter::liked", "true")],
            0,
            library_track(binding.library_id, "song-1"),
        )
        .translate(&binding)
        .await
        .err()
        .unwrap();
        assert!(matches!(
            liked.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Unsupported {
                request: "ResolveMixQueryRequest",
                reason: UnsupportedReason::NotImplemented,
            })
        ));

        let random_covers = click_resolve(
            &[("lib::random", "30"), ("filter::with_cover_art", "true")],
            0,
            library_track(binding.library_id, "song-1"),
        )
        .translate(&binding)
        .await
        .err()
        .unwrap();
        assert!(matches!(
            random_covers.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Unsupported {
                request: "ResolveMixQueryRequest",
                reason: UnsupportedReason::NotImplemented,
            })
        ));

        let unlisted_playlist = click_resolve(
            &[("lib::playlist", "9999")],
            0,
            library_track(binding.library_id, "song-1"),
        )
        .translate(&binding)
        .await
        .err()
        .unwrap();
        assert!(matches!(
            unlisted_playlist.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::InvalidInput { .. })
        ));

        let foreign = click_resolve(
            &[("lib::all", "true")],
            0,
            library_track(Uuid::new_v4(), "song-1"),
        )
        .translate(&binding)
        .await
        .err()
        .unwrap();
        assert!(matches!(
            foreign.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::InvalidInput { .. })
        ));
        assert!(server.state.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_clicked_track_is_queued_and_started_on_the_jukebox() {
        let root = TempDir::new().unwrap();
        let (binding, server) =
            mock_subsonic_mode(&root, true, SubsonicOpenMode::RemoteControl).await;
        let library = binding.library_id;
        set_mock_jukebox(&server, &["track-a"], Some(0), false, 0);
        let slots = RequestSlots::subsonic(&binding.probe());
        assert_eq!(ResolveMixQueryRequest::slot(&slots), Slot::Translate);
        assert_eq!(PlaylistRevisionRequest::slot(&slots), Slot::Translate);
        assert_eq!(LoadResolvedItemsRequest::slot(&slots), Slot::Translate);

        let resolved = click_resolve(&[("lib::all", "true")], 2, library_track(library, "song-3"))
            .translate(&binding)
            .await
            .unwrap()
            .unwrap();
        let revision = PlaylistRevisionRequest { call_id: Some(32) }
            .translate(&binding)
            .await
            .unwrap()
            .unwrap()
            .revision;
        let snapshot = SessionSnapshotRequest { call_id: None }
            .translate(&binding)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(revision, snapshot.revision);
        assert_eq!(snapshot.items, library_tracks(library, &["track-a"]));
        LoadResolvedItemsRequest {
            items: resolved.playing_items,
            start_index: resolved.start_index,
            mode: PlaylistOperateMode::Replace,
            expected_revision: Some(revision),
            playback_mode: None,
            position_seconds: None,
            play: LoadPlay::Auto,
            call_id: None,
        }
        .translate(&binding)
        .await
        .unwrap()
        .unwrap();

        let jukebox = binding.client.jukebox_get().await.unwrap();
        assert_eq!(
            jukebox
                .playlist
                .songs
                .iter()
                .map(|song| song.id.as_str())
                .collect::<Vec<_>>(),
            ["song-1", "song-2", "song-3", "song-4"]
        );
        assert_eq!(jukebox.status.current_index, Some(2));
        assert!(jukebox.status.playing);
    }

    #[tokio::test]
    async fn subsonic_track_and_collection_summaries_return_requested_kinds() {
        let root = TempDir::new().unwrap();
        let (binding, _) = mock_subsonic(&root, false).await;
        let tracks = search_media_file_summary_request(
            &binding,
            &SearchMediaFileSummaryRequest {
                n: 3,
                call_id: Some(25),
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(tracks.result.len(), 3);
        assert_eq!(tracks.result[0].name, "song-1");
        let albums = search_collection_summary_request(
            &binding,
            &SearchCollectionSummaryRequest {
                collection_type: Some(CollectionType::Album),
                bake_cover_arts: Some(false),
                n: 2,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(albums.collection_type, CollectionType::Album);
        assert_eq!(albums.result[0].name, "Album One");
        let artists = search_collection_summary_request(
            &binding,
            &SearchCollectionSummaryRequest {
                collection_type: Some(CollectionType::Artist),
                bake_cover_arts: Some(false),
                n: 2,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(artists.collection_type, CollectionType::Artist);
        assert_eq!(artists.result[0].name, "Artist One");
    }

    #[tokio::test]
    async fn empty_search_rejection_is_typed_unsupported() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        server
            .state
            .empty_search_rejected
            .store(1, Ordering::SeqCst);
        let error = fetch_media_files_request(
            &binding,
            &FetchMediaFilesRequest {
                cursor: 0,
                page_size: 10,
                bake_cover_arts: false,
                call_id: None,
            },
        )
        .await
        .err()
        .expect("empty search should be rejected");
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Unsupported {
                reason: UnsupportedReason::NotImplemented,
                ..
            })
        ));
        assert_eq!(
            server.state.requests.lock().unwrap().as_slice(),
            ["search3"]
        );
    }

    fn mix_request(queries: &[(&str, &str)]) -> MixQueryRequest {
        MixQueryRequest {
            queries: queries
                .iter()
                .map(|&(operator, parameter)| MixQuery {
                    operator: operator.into(),
                    parameter: parameter.into(),
                })
                .collect(),
            cursor: 0,
            page_size: 40,
            bake_cover_arts: true,
            queue: CallerQueue::default(),
            random_seed: None,
            call_id: None,
        }
    }

    #[tokio::test]
    async fn the_cover_wall_query_returns_random_tracks_with_their_cached_covers() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        for queries in [
            [("lib::random", "40"), ("filter::with_cover_art", "true")],
            [("filter::with_cover_art", "true"), ("lib::random", "40")],
        ] {
            let response = mix_query_request(&binding, &mix_request(&queries))
                .await
                .unwrap()
                .unwrap();
            let tracks = response
                .files
                .iter()
                .map(|file| file.track.as_str())
                .collect::<Vec<_>>();
            assert_eq!(tracks, ["song-1", "song-2"], "{queries:?}");
            assert_eq!(response.cover_art_map.len(), 2, "{queries:?}");
            for file in &response.files {
                let cover = &response.cover_art_map[&file.id];
                assert_eq!(fs::read(cover).unwrap(), b"image-cover-a", "{cover}");
            }
        }
        assert_eq!(
            server.state.random_song_sizes.lock().unwrap().as_slice(),
            ["40", "40"]
        );
    }

    #[tokio::test]
    async fn the_cover_wall_query_leaves_out_tracks_whose_cover_was_evicted() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        let held = server.state.error_cover_gate.write().await;
        let binding = Arc::new(binding);
        let query = {
            let binding = Arc::clone(&binding);
            tokio::spawn(async move {
                mix_query_request(
                    &binding,
                    &mix_request(&[("lib::random", "40"), ("filter::with_cover_art", "true")]),
                )
                .await
            })
        };
        // `cover-a` is cached while `error-cover` is held, then removed as eviction would.
        wait_for_covers(&binding, &["cover-a"]).await;
        fs::remove_file(binding.covers.path_for("cover-a")).unwrap();
        drop(held);

        let response = query.await.unwrap().unwrap().unwrap();
        assert!(response.files.is_empty(), "{:?}", response.files);
        assert!(response.cover_art_map.is_empty());
    }

    #[tokio::test]
    async fn the_cover_wall_query_on_an_empty_library_returns_nothing() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        server.state.no_random_songs.store(1, Ordering::SeqCst);
        let response = mix_query_request(
            &binding,
            &mix_request(&[("lib::random", "40"), ("filter::with_cover_art", "true")]),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(response.files.is_empty());
        assert!(response.cover_art_map.is_empty());
    }

    #[tokio::test]
    async fn other_mix_queries_stay_unsupported() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        let cases: [&[(&str, &str)]; 5] = [
            &[("lib::random", "40")],
            &[("filter::with_cover_art", "true")],
            &[("lib::random", "40"), ("filter::with_cover_art", "false")],
            &[
                ("lib::random", "40"),
                ("filter::with_cover_art", "true"),
                ("lib::album", "1"),
            ],
            &[("lib::album", "1"), ("sort::track_number", "false")],
        ];
        let seeded = MixQueryRequest {
            random_seed: Some(7),
            ..mix_request(&[("lib::random", "40"), ("filter::with_cover_art", "true")])
        };
        let requests = cases.into_iter().map(mix_request).chain(iter::once(seeded));
        for request in requests {
            let queries = &request.queries;
            let error = mix_query_request(&binding, &request)
                .await
                .err()
                .unwrap_or_else(|| panic!("{queries:?} should be unsupported"));
            assert!(
                matches!(
                    error.downcast_ref::<ExpectedRequestError>(),
                    Some(ExpectedRequestError::Unsupported {
                        reason: UnsupportedReason::NotImplemented,
                        ..
                    })
                ),
                "{queries:?}: {error:#}"
            );
        }
        assert!(server.state.requests.lock().unwrap().is_empty());
    }

    fn track_ref(binding: &SubsonicBinding, track: &str) -> PlayingItemRef {
        PlayingItemRef::LibraryTrack {
            library_id: binding.library_id.to_string(),
            track: track.into(),
        }
    }

    fn resolve(
        binding: &SubsonicBinding,
        album: i32,
        initial: &str,
        excluded: &[&str],
    ) -> ResolveMixQueryRequest {
        resolve_queries(
            binding,
            vec![query("lib::album", &album.to_string())],
            initial,
            excluded,
        )
    }

    fn resolve_queries(
        binding: &SubsonicBinding,
        queries: Vec<MixQuery>,
        initial: &str,
        excluded: &[&str],
    ) -> ResolveMixQueryRequest {
        ResolveMixQueryRequest {
            queries,
            hint_position: 0,
            initial_playback_item: Some(track_ref(binding, initial)),
            instantly_play: true,
            fallback_playing_items: vec![track_ref(binding, "fallback")],
            excluded: ExcludedItems {
                items: excluded.iter().map(|id| track_ref(binding, id)).collect(),
            },
            call_id: None,
        }
    }

    fn query(operator: &str, parameter: &str) -> MixQuery {
        MixQuery {
            operator: operator.into(),
            parameter: parameter.into(),
        }
    }

    fn keys(items: &[PlayingItemRef]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                PlayingItemRef::LibraryTrack { track, .. } => track.clone(),
                PlayingItemRef::IndependentFile { path } => path.clone(),
            })
            .collect()
    }

    fn endpoint_count(server: &MockServer, endpoint: &str) -> usize {
        server
            .state
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|asked| asked.as_str() == endpoint)
            .count()
    }

    #[tokio::test]
    async fn subsonic_resolves_a_mix_query_in_order_without_the_queue() {
        let root = TempDir::new().unwrap();
        let (binding, _server) = mock_subsonic(&root, false).await;
        let album = binding
            .catalog
            .lock()
            .unwrap()
            .ids
            .intern(SubsonicIdKind::Album, "album-1")
            .unwrap();
        let tracks = |response: &ResolveMixQueryResponse| keys(&response.playing_items);
        let all = resolve_mix_query_request(&binding, &resolve(&binding, album, "song-2", &[]))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tracks(&all), ["song-1", "song-2"]);
        assert_eq!(all.start_index, Some(1));
        // The caller's own queue is left out, and the start index follows.
        let without =
            resolve_mix_query_request(&binding, &resolve(&binding, album, "song-2", &["song-1"]))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(tracks(&without), ["song-2"]);
        assert_eq!(without.start_index, Some(0));
        // A track the query does not hold starts the fallback items at the hint.
        let mut elsewhere = resolve(&binding, album, "elsewhere", &[]);
        elsewhere
            .fallback_playing_items
            .push(track_ref(&binding, "fallback-2"));
        elsewhere.hint_position = 1;
        let missing = resolve_mix_query_request(&binding, &elsewhere)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tracks(&missing), ["fallback", "fallback-2"]);
        assert_eq!(missing.start_index, Some(1));
        // Nothing to play: no index.
        let mut quiet = resolve(&binding, album, "song-2", &[]);
        quiet.instantly_play = false;
        let quiet = resolve_mix_query_request(&binding, &quiet)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (tracks(&quiet), quiet.start_index),
            (vec!["song-1".into(), "song-2".into()], None)
        );
    }

    #[tokio::test]
    async fn subsonic_home_lists_its_groups_and_leaves_the_rest_empty() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        assert_eq!(
            ComplexQueryRequest::slot(&RequestSlots::subsonic(&binding.probe())),
            Slot::Translate
        );
        server
            .state
            .starred
            .lock()
            .unwrap()
            .insert("song-2".to_owned());
        let group = |title: &str, query| ComplexQuery {
            title: title.into(),
            query,
        };
        let mode = LibraryHomeListMode::Forward;
        let response = complex_query_request(
            &binding,
            &ComplexQueryRequest {
                queries: vec![
                    group("Artists", LibraryHomeQuery::Artists(mode)),
                    group("Albums", LibraryHomeQuery::Albums(mode)),
                    group("Playlists", LibraryHomeQuery::Playlists(mode)),
                    group("Tracks", LibraryHomeQuery::Tracks(mode)),
                    group("Liked", LibraryHomeQuery::Liked),
                    group("Mixes", LibraryHomeQuery::Mixes(mode)),
                    group("Genres", LibraryHomeQuery::Genres(mode)),
                    group("Most", LibraryHomeQuery::Most),
                ],
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        let names = |index: usize| {
            response.result[index]
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            response
                .result
                .iter()
                .map(|group| group.title.as_str())
                .collect::<Vec<_>>(),
            [
                "Artists",
                "Albums",
                "Playlists",
                "Tracks",
                "Liked",
                "Mixes",
                "Genres",
                "Most"
            ]
        );
        assert_eq!(names(0), ["Artist One", "Artist Two"]);
        assert_eq!(names(1), ["Album One", "Album Two"]);
        assert_eq!(names(2), ["Road"]);
        assert_eq!(names(3), ["song-1", "song-2", "song-3", "song-4"]);
        // The newest first is the list reversed.
        let reversed = complex_query_request(
            &binding,
            &ComplexQueryRequest {
                queries: vec![group(
                    "Albums",
                    LibraryHomeQuery::Albums(LibraryHomeListMode::Reverse),
                )],
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            reversed.result[0]
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["Album Two", "Album One"]
        );
        assert_eq!(names(4), ["song-2"]);
        for empty in 5..8 {
            assert!(response.result[empty].entries.is_empty());
        }
        // An album opens by the query that lists its tracks, and a playlist by the id the
        // playlists page gives it.
        let album = &response.result[1].entries[0];
        assert_eq!(album.queries[0].operator, "lib::album");
        assert_eq!(album.queries[0].parameter, album.id.to_string());
        let playlist = &response.result[2].entries[0];
        assert_eq!(playlist.queries[0].operator, "lib::playlist");
        assert_eq!(playlist.id, playlist_ids(&binding).await["Road"]);
        // A track plays the listed tracks from itself.
        let track = &response.result[3].entries[1];
        let ids: Vec<_> = track
            .queries
            .iter()
            .map(|query| query.parameter.clone())
            .collect();
        assert_eq!(ids.len(), 4);
        assert_eq!(ids[0], track.id.to_string());
    }

    #[tokio::test]
    async fn home_playlists_can_be_renamed_and_removed_as_on_the_playlists_page() {
        let root = TempDir::new().unwrap();
        let (binding, _server) = mock_subsonic(&root, false).await;
        let group = |title: &str, query| ComplexQuery {
            title: title.into(),
            query,
        };
        let mode = LibraryHomeListMode::Forward;
        let home = complex_query_request(
            &binding,
            &ComplexQueryRequest {
                queries: vec![
                    group("Playlists", LibraryHomeQuery::Playlists(mode)),
                    group("Albums", LibraryHomeQuery::Albums(mode)),
                    group("Artists", LibraryHomeQuery::Artists(mode)),
                ],
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        let readonly = |index: usize| {
            home.result[index]
                .entries
                .iter()
                .map(|entry| entry.readonly)
                .collect::<Vec<_>>()
        };
        assert_eq!(readonly(0), [false]);
        assert_eq!(readonly(1), [true, true]);
        assert_eq!(readonly(2), [true, true]);
    }

    #[tokio::test]
    async fn subsonic_home_track_entries_resolve_to_their_tracks() {
        let root = TempDir::new().unwrap();
        let (binding, _server) = mock_subsonic(&root, false).await;
        let home = complex_query_request(
            &binding,
            &ComplexQueryRequest {
                queries: vec![ComplexQuery {
                    title: "Tracks".into(),
                    query: LibraryHomeQuery::Tracks(LibraryHomeListMode::Forward),
                }],
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        // The oldest first: the sample as the server lists it.
        let entries = &home.result[0].entries;
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["song-1", "song-2", "song-3", "song-4"]
        );
        let resolved = resolve_mix_query_request(
            &binding,
            &resolve_queries(&binding, entries[1].queries.clone(), "song-2", &[]),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            keys(&resolved.playing_items),
            ["song-2", "song-3", "song-4", "song-1"]
        );
        assert_eq!(resolved.start_index, Some(0));
        // Ids no one gave, and repeats, name no track.
        let odd = resolve_mix_query_request(
            &binding,
            &resolve_queries(
                &binding,
                vec![
                    query("lib::track", &entries[0].id.to_string()),
                    query("lib::track", "9999"),
                    query("lib::track", &entries[0].id.to_string()),
                ],
                "song-1",
                &[],
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(keys(&odd.playing_items), ["song-1"]);
    }

    #[tokio::test]
    async fn subsonic_takes_the_queries_the_tracks_and_album_pages_send() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        let album = binding
            .catalog
            .lock()
            .unwrap()
            .ids
            .intern(SubsonicIdKind::Album, "album-1")
            .unwrap();
        // The tracks page: every song, in the order the server lists them.
        let all = vec![query("lib::all", "true")];
        let resolved = resolve_mix_query_request(
            &binding,
            &resolve_queries(&binding, all.clone(), "song-3", &[]),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            keys(&resolved.playing_items),
            ["song-1", "song-2", "song-3", "song-4"]
        );
        // Asking again without a click reads the list again, as the server has it now.
        let before = endpoint_count(&server, "search3");
        let mut again = resolve_queries(&binding, all.clone(), "song-1", &[]);
        again.instantly_play = false;
        resolve_mix_query_request(&binding, &again).await.unwrap();
        assert!(endpoint_count(&server, "search3") > before);
        let mut page = mix_query_request(
            &binding,
            &MixQueryRequest {
                call_id: None,
                queries: all,
                cursor: 1,
                page_size: 2,
                bake_cover_arts: false,
                queue: CallerQueue::default(),
                random_seed: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            page.files
                .drain(..)
                .map(|file| file.track)
                .collect::<Vec<_>>(),
            ["song-2", "song-3"]
        );
        // An album's page: its tracks, with the track-number sort it already has.
        let detail = vec![
            query("lib::album", &album.to_string()),
            query("sort::track_number", "true"),
        ];
        let resolved = resolve_mix_query_request(
            &binding,
            &resolve_queries(&binding, detail.clone(), "song-2", &[]),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(keys(&resolved.playing_items), ["song-1", "song-2"]);
        let page = mix_query_request(
            &binding,
            &MixQueryRequest {
                call_id: None,
                queries: detail,
                cursor: 0,
                page_size: 10,
                bake_cover_arts: false,
                queue: CallerQueue::default(),
                random_seed: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(page.files.len(), 2);
    }

    #[tokio::test]
    async fn subsonic_cover_filter_pages_through_the_songs_that_have_a_cover() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        server.state.listed_covers.store(1, Ordering::SeqCst);
        let covered = |cursor: i32| MixQueryRequest {
            cursor,
            page_size: 30,
            ..mix_request(&[("lib::all", "true"), ("filter::with_cover_art", "true")])
        };
        let tracks = |response: crate::messages::MixQueryResponse| {
            response
                .files
                .into_iter()
                .map(|file| file.track)
                .collect::<Vec<_>>()
        };
        // Songs without a cover are dropped before the page is cut.
        let first = mix_query_request(&binding, &covered(0))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tracks(first), ["song-1", "song-3"]);
        // The list read for the first page is kept for the next one.
        let reads = endpoint_count(&server, "search3");
        assert!(reads > 0);
        let second = mix_query_request(&binding, &covered(1))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tracks(second), ["song-3"]);
        assert_eq!(endpoint_count(&server, "search3"), reads);
        assert_eq!(endpoint_count(&server, "getRandomSongs"), 0);
    }

    #[tokio::test]
    async fn the_pages_of_a_cover_filter_keep_their_songs_when_a_song_loses_its_cover() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        server.state.listed_covers.store(1, Ordering::SeqCst);
        let covered = |cursor: i32| MixQueryRequest {
            cursor,
            page_size: 1,
            ..mix_request(&[("lib::all", "true"), ("filter::with_cover_art", "true")])
        };
        let tracks = |response: crate::messages::MixQueryResponse| {
            response
                .files
                .into_iter()
                .map(|file| file.track)
                .collect::<Vec<_>>()
        };
        let first = mix_query_request(&binding, &covered(0))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tracks(first), ["song-1"]);
        // A later read found that the first song has no cover any more.
        {
            let mut catalog = binding.catalog.lock().unwrap();
            let id = catalog.ids.intern(SubsonicIdKind::Track, "song-1").unwrap();
            catalog.songs.get_mut(&id).unwrap().cover_art = None;
        }
        let second = mix_query_request(&binding, &covered(1))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tracks(second), ["song-3"]);
    }

    #[tokio::test]
    async fn a_kept_cover_pool_leaves_newer_song_metadata_alone() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        server.state.listed_covers.store(1, Ordering::SeqCst);
        let covered = mix_request(&[("lib::all", "true"), ("filter::with_cover_art", "true")]);
        mix_query_request(&binding, &covered)
            .await
            .unwrap()
            .unwrap();
        // A later read found a new title for a song of the kept pool.
        let id = {
            let mut catalog = binding.catalog.lock().unwrap();
            let id = catalog.ids.intern(SubsonicIdKind::Track, "song-1").unwrap();
            catalog.songs.get_mut(&id).unwrap().title = "New".into();
            id
        };
        let page = mix_query_request(&binding, &covered)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(page.files[0].title, "New");
        assert_eq!(binding.catalog.lock().unwrap().songs[&id].title, "New");
    }

    #[tokio::test]
    async fn subsonic_playlist_writes_announce_the_change() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        let announced = || server.broadcasts.load(Ordering::SeqCst);
        let before = announced();
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
        .unwrap()
        .playlist
        .id;
        assert_eq!(announced(), before + 1);
        update_playlist_request(
            &binding,
            &UpdatePlaylistRequest {
                playlist_id: created,
                name: "Renamed".into(),
                group: String::new(),
                call_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(announced(), before + 2);
        remove_playlist_request(
            &binding,
            &RemovePlaylistRequest {
                playlist_id: created,
                call_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(announced(), before + 3);
        assert!(!playlist_ids(&binding).await.contains_key("Renamed"));
    }

    #[tokio::test]
    async fn a_created_subsonic_playlist_is_announced_when_it_cannot_be_read_back() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        // The server gives the new playlist the id of one it lists already, so the read back
        // cannot tell the new one apart.
        server.state.playlists.lock().unwrap()[0].id = "playlist-2".into();
        let before = server.broadcasts.load(Ordering::SeqCst);
        let created = create_playlist_request(
            &binding,
            &CreatePlaylistRequest {
                name: "Fresh".into(),
                group: String::new(),
                call_id: None,
            },
        )
        .await;
        assert!(created.is_err());
        assert_eq!(server.broadcasts.load(Ordering::SeqCst), before + 1);
    }

    #[tokio::test]
    async fn every_resolve_reads_the_current_song_list() {
        let root = TempDir::new().unwrap();
        let (binding, server) =
            mock_subsonic_mode(&root, false, SubsonicOpenMode::PlayOnThisDevice).await;
        let library = binding.library_id;
        let listed = |count: usize| server.state.listed_songs.store(count, Ordering::SeqCst);
        let click =
            |track: &str| click_resolve(&[("lib::all", "true")], 0, library_track(library, track));
        listed(3);
        let resolved = click("song-1").translate(&binding).await.unwrap().unwrap();
        assert_eq!(
            resolved.playing_items,
            library_tracks(library, &["song-1", "song-2", "song-3"])
        );
        // The server added a song: a click on it finds it.
        listed(4);
        let resolved = click("song-4").translate(&binding).await.unwrap().unwrap();
        assert_eq!(
            resolved.playing_items,
            library_tracks(library, &["song-1", "song-2", "song-3", "song-4"])
        );
        assert_eq!(resolved.start_index, Some(3));
        // The server dropped two: a resolve that plays no clicked track, as one that adds the
        // list to the queue, leaves them out.
        listed(2);
        let mut queued = click("song-1");
        queued.instantly_play = false;
        let queued = queued.translate(&binding).await.unwrap().unwrap();
        assert_eq!(
            queued.playing_items,
            library_tracks(library, &["song-1", "song-2"])
        );
    }

    #[tokio::test]
    async fn home_artists_keep_the_server_order_whatever_was_numbered_first() {
        let root = TempDir::new().unwrap();
        let (binding, _server) = mock_subsonic(&root, false).await;
        // A search numbered the second artist before the artist list was read.
        binding
            .catalog
            .lock()
            .unwrap()
            .ids
            .intern(SubsonicIdKind::Artist, "artist-2")
            .unwrap();
        let home = complex_query_request(
            &binding,
            &ComplexQueryRequest {
                queries: vec![ComplexQuery {
                    title: "Artists".into(),
                    query: LibraryHomeQuery::Artists(LibraryHomeListMode::Forward),
                }],
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            home.result[0]
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["Artist One", "Artist Two"]
        );
    }

    #[tokio::test]
    async fn home_tracks_read_on_past_a_short_server_page() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        server.state.song_page_cap.store(3, Ordering::SeqCst);
        let home = complex_query_request(
            &binding,
            &ComplexQueryRequest {
                queries: vec![ComplexQuery {
                    title: "Tracks".into(),
                    query: LibraryHomeQuery::Tracks(LibraryHomeListMode::Forward),
                }],
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            home.result[0]
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["song-1", "song-2", "song-3", "song-4"]
        );
    }

    #[tokio::test]
    async fn a_page_of_every_track_reads_on_past_a_short_server_page() {
        let root = TempDir::new().unwrap();
        let (binding, server) =
            mock_subsonic_mode(&root, false, SubsonicOpenMode::PlayOnThisDevice).await;
        server.state.song_page_cap.store(2, Ordering::SeqCst);
        let mut request = mix_request(&[("lib::all", "true")]);
        request.cursor = 1;
        request.page_size = 3;
        let page = mix_query_request(&binding, &request)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            page.files
                .iter()
                .map(|file| file.title.as_str())
                .collect::<Vec<_>>(),
            ["song-2", "song-3", "song-4"]
        );
    }

    #[tokio::test]
    async fn the_newest_tracks_on_the_home_are_the_tracks_of_the_newest_albums() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        server.state.newer_albums.store(30, Ordering::SeqCst);
        let home = complex_query_request(
            &binding,
            &ComplexQueryRequest {
                queries: vec![ComplexQuery {
                    title: "Tracks".into(),
                    query: LibraryHomeQuery::Tracks(LibraryHomeListMode::Reverse),
                }],
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        let names: Vec<_> = home.result[0]
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        let newest: Vec<_> = (6..=30)
            .rev()
            .map(|n| format!("album-new-{n}-track"))
            .collect();
        assert_eq!(names, newest);
    }

    #[tokio::test]
    async fn the_newest_albums_on_the_home_read_only_the_pages_they_need() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        server.state.newer_albums.store(30, Ordering::SeqCst);
        let home = complex_query_request(
            &binding,
            &ComplexQueryRequest {
                queries: vec![ComplexQuery {
                    title: "Albums".into(),
                    query: LibraryHomeQuery::Albums(LibraryHomeListMode::Reverse),
                }],
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        let names: Vec<_> = home.result[0]
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        let newest: Vec<_> = (6..=30).rev().map(|n| format!("New {n}")).collect();
        assert_eq!(names, newest);
        // The server gives one album a page, so the 25 newest take 25 pages, not all 33.
        assert_eq!(endpoint_count(&server, "getAlbumList2"), 25);
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
        assert_eq!(PlaylistRevisionRequest::slot(&slots), Slot::Translate);
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
        assert_eq!(SessionSnapshotRequest::slot(&slots), Slot::Translate);
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

        tokio_time::timeout(
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
            play: LoadPlay::Auto,
            call_id: Some(17),
        }
    }

    fn jukebox_load(
        library_id: Uuid,
        tracks: &[&str],
        mode: PlaylistOperateMode,
    ) -> LoadResolvedItemsRequest {
        LoadResolvedItemsRequest {
            items: tracks
                .iter()
                .map(|track| PlayingItemRef::LibraryTrack {
                    library_id: library_id.to_string(),
                    track: (*track).to_owned(),
                })
                .collect(),
            start_index: None,
            mode,
            expected_revision: None,
            playback_mode: None,
            position_seconds: None,
            play: LoadPlay::Auto,
            call_id: None,
        }
    }

    fn set_mock_jukebox(
        server: &MockServer,
        tracks: &[&str],
        current_index: Option<u32>,
        playing: bool,
        position: u32,
    ) {
        let mut jukebox = server.state.jukebox.lock().unwrap();
        jukebox.entries = tracks.iter().map(|id| mock_song(id)).collect();
        jukebox.current_index = current_index;
        jukebox.playing = playing;
        jukebox.position = position;
        drop(jukebox);
        server.state.jukebox_calls.lock().unwrap().clear();
    }

    #[tokio::test]
    async fn next_at_the_last_song_stops_instead_of_skipping_past_the_end() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, true).await;
        set_mock_jukebox(&server, &["track-a", "track-b"], Some(1), true, 20);

        RequestTranslator::translate_next_request(&binding, &NextRequest { call_id: None })
            .await
            .unwrap()
            .unwrap();

        let status = binding.client.jukebox_status().await.unwrap();
        assert!(
            !status.playing,
            "sequential playback ends after the last song"
        );
        assert_eq!(status.current_index, Some(1));
        let calls = server.state.jukebox_calls.lock().unwrap();
        assert!(!calls.iter().any(|call| {
            call.get("action").is_some_and(|action| action == "skip")
                && call
                    .get("index")
                    .and_then(|index| index.parse::<u32>().ok())
                    .is_some_and(|index| index >= 2)
        }));
    }

    #[tokio::test]
    async fn play_next_inserts_after_the_current_song_and_keeps_its_place() {
        for playing in [true, false] {
            let root = TempDir::new().unwrap();
            let (binding, server) = mock_subsonic(&root, true).await;
            set_mock_jukebox(
                &server,
                &["track-a", "track-b", "track-c"],
                Some(1),
                playing,
                37,
            );

            RequestTranslator::translate_load_resolved_items_request(
                &binding,
                &jukebox_load(
                    binding.library_id,
                    &["track-x"],
                    PlaylistOperateMode::PlayNext,
                ),
            )
            .await
            .unwrap()
            .unwrap();

            let queue = binding.client.jukebox_get().await.unwrap();
            let ids = queue
                .playlist
                .songs
                .iter()
                .map(|song| song.id.as_str())
                .collect::<Vec<_>>();
            assert_eq!(ids, ["track-a", "track-b", "track-x", "track-c"]);
            assert_eq!(queue.status.current_index, Some(1));
            assert_eq!(queue.status.position_secs, Some(37));
            assert_eq!(queue.status.playing, playing);
        }
    }

    #[tokio::test]
    async fn load_starts_the_requested_song_at_the_requested_position() {
        let tracks = ["t-1", "t-2", "t-3", "t-4", "t-5", "t-6"];
        for (play, expected_playing) in [(LoadPlay::Play, true), (LoadPlay::Pause, false)] {
            let root = TempDir::new().unwrap();
            let (binding, _server) = mock_subsonic(&root, true).await;
            let mut request =
                jukebox_load(binding.library_id, &tracks, PlaylistOperateMode::Replace);
            request.start_index = Some(4);
            request.position_seconds = Some(12.0);
            request.play = play;

            RequestTranslator::translate_load_resolved_items_request(&binding, &request)
                .await
                .unwrap()
                .unwrap();

            let queue = binding.client.jukebox_get().await.unwrap();
            assert_eq!(queue.playlist.songs.len(), tracks.len());
            assert_eq!(queue.status.current_index, Some(4));
            assert_eq!(queue.status.position_secs, Some(12));
            assert_eq!(queue.status.playing, expected_playing);
        }
    }

    #[tokio::test]
    async fn load_of_an_appended_list_starts_inside_the_new_songs() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, true).await;
        set_mock_jukebox(&server, &["track-a", "track-b"], Some(0), false, 0);
        let mut request = jukebox_load(
            binding.library_id,
            &["t-1", "t-2"],
            PlaylistOperateMode::AppendToEnd,
        );
        request.start_index = Some(1);

        RequestTranslator::translate_load_resolved_items_request(&binding, &request)
            .await
            .unwrap()
            .unwrap();

        let queue = binding.client.jukebox_get().await.unwrap();
        assert_eq!(queue.playlist.songs.len(), 4);
        assert_eq!(queue.status.current_index, Some(3));
        assert!(queue.status.playing);
    }

    #[tokio::test]
    async fn load_refuses_a_start_index_outside_the_loaded_songs() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, true).await;
        let mut request = jukebox_load(binding.library_id, &["t-1"], PlaylistOperateMode::Replace);
        request.start_index = Some(1);

        let error = RequestTranslator::translate_load_resolved_items_request(&binding, &request)
            .await
            .err()
            .and_then(|error| error.downcast::<ExpectedRequestError>().ok());

        assert!(matches!(
            error,
            Some(ExpectedRequestError::InvalidInput { .. })
        ));
        assert!(server.state.jukebox_calls.lock().unwrap().is_empty());
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
                mode: PlaybackMode::Shuffle,
                call_id: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        let unsupported_mode = RequestTranslator::translate_set_playback_mode_request(
            &binding,
            &SetPlaybackModeRequest {
                mode: PlaybackMode::RepeatOne,
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
        request.items[0] = PlayingItemRef::from(player::LibraryItem {
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
        let slots = RequestSlots::subsonic(&binding.probe());
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
            PlaylistRevisionRequest::slot(&slots),
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
        tokio_time::timeout(Duration::from_secs(4), async {
            while server.broadcasts.load(Ordering::SeqCst) < 2 {
                tokio_time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        tokio_time::sleep(Duration::from_millis(3100)).await;
        assert_eq!(server.broadcasts.load(Ordering::SeqCst), 2);
        server.state.jukebox.lock().unwrap().playing = true;
        tokio_time::timeout(Duration::from_secs(4), async {
            while server.broadcasts.load(Ordering::SeqCst) < 4 {
                tokio_time::sleep(Duration::from_millis(20)).await;
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
        tokio_time::sleep(Duration::from_millis(3200)).await;
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

    #[tokio::test]
    async fn rejected_request_credentials_are_typed_as_not_authorized() {
        use subsonic::SubsonicServerError;
        let root = TempDir::new().unwrap();
        let (binding, _server) = mock_subsonic(&root, false).await;
        let failures = [
            SubsonicServerError::WrongCredentials,
            SubsonicServerError::NotAuthorized,
            SubsonicServerError::InvalidApiKey,
        ];
        for kind in failures {
            let error = subsonic_request_error(
                &binding,
                SubsonicError::Server {
                    kind,
                    message: "rejected".into(),
                },
            );
            assert!(matches!(
                error.downcast_ref::<ExpectedRequestError>(),
                Some(ExpectedRequestError::ServiceSignInRequired { record_id })
                    if *record_id == binding.library_id.to_string()
            ));
            assert_eq!(classify(&error), RequestErrorKind::PermissionDenied);
        }
    }

    #[tokio::test]
    async fn jukebox_failures_keep_authorization_unavailable_and_disabled_types() {
        use subsonic::SubsonicServerError;

        let root = TempDir::new().unwrap();
        let (binding, _server) = mock_subsonic(&root, true).await;
        let unauthorized = jukebox_error(
            &binding,
            SubsonicError::Server {
                kind: SubsonicServerError::WrongCredentials,
                message: "rejected".into(),
            },
            PlayRequest::wire_name(),
        );
        assert!(matches!(
            unauthorized.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::ServiceSignInRequired { record_id })
                if *record_id == binding.library_id.to_string()
        ));
        let unavailable = jukebox_error(
            &binding,
            SubsonicError::Transport("offline".into()),
            PlayRequest::wire_name(),
        );
        assert!(matches!(
            unavailable.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Unavailable { .. })
        ));
        let disabled = jukebox_error(
            &binding,
            SubsonicError::JukeboxUnsupported,
            PlayRequest::wire_name(),
        );
        assert!(matches!(
            disabled.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Unsupported {
                reason: UnsupportedReason::NotImplemented,
                ..
            })
        ));
    }

    /// A binding of the library `binding` serves, made again as the library is when it is
    /// opened again, with a client of its own for the server at `url`.
    fn reopened(root: &TempDir, binding: &SubsonicBinding, url: &str) -> SubsonicBinding {
        SubsonicBinding::new(
            SubsonicClient::new(
                Url::parse(url).unwrap(),
                SubsonicCredentials::Password {
                    user: "alice".into(),
                    password: "cover-secret-password".into(),
                },
                Client::new(),
            ),
            binding.probe(),
            root.path().to_path_buf(),
            binding.library_id,
            binding.play_on_this_device,
            Arc::new(CountingBroadcaster(Arc::default())),
            Arc::new(Visualizer::new()),
        )
    }

    /// Waits until the library `binding` serves keeps each of `lists` on disk.
    async fn kept_on_disk(root: &TempDir, binding: &SubsonicBinding, lists: &[&str]) {
        use crate::service::lists::lists_folder;
        use crate::test_wait::wait_until;

        let folder = lists_folder(root.path(), binding.library_id);
        for list in lists {
            let file = folder.join(format!("{list}.json"));
            wait_until("the kept lists on disk", || {
                let kept = file.is_file();
                async move { Ok(kept.then_some(())) }
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn a_library_opened_again_while_its_server_is_down_shows_its_lists_from_disk() {
        use crate::messages::{
            ComplexQuery, ComplexQueryRequest, ComplexQueryResponse, LibraryHomeListMode,
            LibraryHomeQuery, MixQueryResponse,
        };

        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        server.state.listed_covers.store(1, Ordering::SeqCst);
        let group = |title: &str, query| ComplexQuery {
            title: title.into(),
            query,
        };
        let home = ComplexQueryRequest {
            queries: vec![
                group(
                    "Artists",
                    LibraryHomeQuery::Artists(LibraryHomeListMode::Forward),
                ),
                group(
                    "Albums",
                    LibraryHomeQuery::Albums(LibraryHomeListMode::Forward),
                ),
            ],
            call_id: None,
        };
        let names = |response: &ComplexQueryResponse| {
            response
                .result
                .iter()
                .map(|group| {
                    group
                        .entries
                        .iter()
                        .map(|entry| entry.name.clone())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        let covered = mix_request(&[("lib::all", "true"), ("filter::with_cover_art", "true")]);
        let tracks = |response: &MixQueryResponse| {
            response
                .files
                .iter()
                .map(|file| file.track.clone())
                .collect::<Vec<_>>()
        };
        let artists = FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
        };

        let first = complex_query_request(&binding, &home)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            names(&first),
            [
                vec!["Artist One".to_owned(), "Artist Two".to_owned()],
                vec!["Album One".to_owned(), "Album Two".to_owned()]
            ]
        );
        let first_covered = mix_query_request(&binding, &covered)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tracks(&first_covered), ["song-1", "song-3"]);
        kept_on_disk(&root, &binding, &["artists", "newest-albums", "songs"]).await;

        // The server stops, and the library is opened again.
        let MockServer {
            url, _task: task, ..
        } = server;
        task.abort();
        let _ = task.await;
        let again = reopened(&root, &binding, &url);
        drop(binding);
        let home = complex_query_request(&again, &home).await.unwrap().unwrap();
        assert_eq!(names(&home), names(&first));
        let covered = mix_query_request(&again, &covered).await.unwrap().unwrap();
        assert_eq!(tracks(&covered), ["song-1", "song-3"]);
        let summary = fetch_collection_group_summary_request(&again, &artists)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(summary.groups[0].count, 2);
        // What no kept list answers still tells that the server cannot be reached.
        let playlists =
            fetch_all_playlists_request(&again, &FetchAllPlaylistsRequest { call_id: None })
                .await
                .err()
                .unwrap();
        assert!(matches!(
            playlists.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Unavailable { .. })
        ));
    }

    #[tokio::test]
    async fn only_a_sign_in_to_another_account_clears_the_kept_lists() {
        use crate::service::ServiceBinding;
        use crate::service::lists::lists_folder;
        use database::credentials::{Credential, Secret};

        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        let summary = FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
        };
        fetch_collection_group_summary_request(&binding, &summary)
            .await
            .unwrap();
        kept_on_disk(&root, &binding, &["artists"]).await;
        let kept = lists_folder(root.path(), binding.library_id).join("artists.json");
        let sign_in = |username: &str, password: &str| Credential {
            username: username.into(),
            secret: Secret::new(password.as_bytes().to_vec()),
        };

        // A new password for the same account keeps its lists.
        binding.renew_sign_in(&sign_in("alice", "new-password"));
        fetch_collection_group_summary_request(&binding, &summary)
            .await
            .unwrap();
        assert!(kept.is_file());
        assert_eq!(endpoint_count(&server, "getArtists"), 1);

        // The lists of another account are not this one's: they go, on disk too, and are read
        // again.
        binding.renew_sign_in(&sign_in("bob", "password"));
        assert!(!kept.exists());
        fetch_collection_group_summary_request(&binding, &summary)
            .await
            .unwrap();
        assert_eq!(endpoint_count(&server, "getArtists"), 2);
    }

    #[tokio::test]
    async fn an_album_list_whose_server_repeats_a_full_page_ends_and_lists_each_album_once() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root, false).await;
        server.state.repeated_albums.store(1, Ordering::SeqCst);
        let summary = tokio_time::timeout(
            Duration::from_secs(10),
            fetch_collection_group_summary_request(
                &binding,
                &FetchCollectionGroupSummaryRequest {
                    call_id: None,
                    collection_type: CollectionType::Album,
                },
            ),
        )
        .await
        .expect("the album list did not end")
        .unwrap()
        .unwrap();
        assert_eq!(
            summary
                .groups
                .iter()
                .map(|group| (group.group_title.as_str(), group.count))
                .collect::<Vec<_>>(),
            [("A", 500)]
        );
        // The page that added no album ends the list.
        assert_eq!(endpoint_count(&server, "getAlbumList2"), 2);
    }

    #[tokio::test]
    async fn another_account_asks_the_server_for_an_artist_the_first_one_listed() {
        use crate::service::ServiceBinding;
        use database::credentials::{Credential, Secret};

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
        let artist = binding
            .catalog
            .lock()
            .unwrap()
            .ids
            .lookup(SubsonicIdKind::Artist, "artist-1")
            .unwrap();
        let fetch = FetchCollectionByIdsRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
            bake_cover_arts: false,
            ids: vec![artist],
        };
        fetch_collection_by_ids_request(&binding, &fetch)
            .await
            .unwrap();
        assert_eq!(endpoint_count(&server, "getArtist"), 0);

        // What the server told the first account is not the next one's to see: the id the UI
        // holds still names the artist, and the server is asked about it.
        binding.renew_sign_in(&Credential {
            username: "bob".into(),
            secret: Secret::new(b"password".to_vec()),
        });
        let _ = fetch_collection_by_ids_request(&binding, &fetch).await;
        assert_eq!(endpoint_count(&server, "getArtist"), 1);
    }
}
