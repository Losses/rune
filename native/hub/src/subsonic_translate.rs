use anyhow::{Result, anyhow};
use database::actions::collection::CollectionQueryType;
use database::actions::playlists::OCCURRENCE_PAGE_SIZES;
use database::actions::utils::generate_group_name;
use database::error::ExpectedError;
use futures::{StreamExt, stream};
use lyric::lrc::parse_lrc;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};
use subsonic::{
    SearchLimits, SubsonicAlbum, SubsonicArtist, SubsonicError, SubsonicPlaylist,
    SubsonicServerError, SubsonicSong,
};

use crate::TypedRequest;
use crate::macros::{RequestSlots, RequestTranslator};
use crate::messages::{
    AddItemToPlaylistRequest, AddItemToPlaylistResponse, Album, Artist, AudioAvailability,
    AudioOrigin, CastPreflightRequest, Collection, CollectionGroup, CollectionGroupSummary,
    CollectionGroupSummaryResponse, CollectionType, CreatePlaylistRequest, CreatePlaylistResponse,
    FetchAllPlaylistsRequest, FetchAllPlaylistsResponse, FetchCollectionByIdsRequest,
    FetchCollectionByIdsResponse, FetchCollectionGroupSummaryRequest, FetchCollectionGroupsRequest,
    FetchCollectionGroupsResponse, FetchMediaFileByIdsRequest, FetchMediaFileByIdsResponse,
    FetchMediaFilesRequest, FetchMediaFilesResponse, FetchParsedMediaFileRequest,
    FetchParsedMediaFileResponse, FetchPlaylistOccurrencesRequest,
    FetchPlaylistOccurrencesResponse, GetLikedRequest, GetLikedResponse, GetLyricByTrackIdRequest,
    GetLyricByTrackIdResponse, GetMediaFilesCountRequest, GetMediaFilesCountResponse,
    GetPlaylistByIdRequest, GetPlaylistByIdResponse, HandoffLoadRequest, LibraryContentChanged,
    LoadPlay, LoadRequest, LoadResolvedItemsRequest, LoadResolvedItemsResponse, LyricContentLine,
    LyricContentLineSection, MediaFile, MediaFileSummary, MixQuery, MixQueryRequest,
    MixQueryResponse, MovePlaylistItemRequest, NextRequest, PauseRequest, PlayRequest,
    PlaybackMode, PlayingItemRef, Playlist, PlaylistOccurrence, PlaylistOccurrenceCursor,
    PlaylistOperateMode, PlaylistRevisionRequest, PlaylistRevisionResponse, PreviousRequest,
    RefreshSessionRequest, RemoveItemFromPlaylistRequest, RemoveItemFromPlaylistResponse,
    RemovePlaylistRequest, RemovePlaylistResponse, RemoveRequest,
    ReorderPlaylistItemPositionRequest, ReorderPlaylistItemPositionResponse,
    ResolveMixQueryRequest, ResolveMixQueryResponse, SearchCollectionSummaryRequest,
    SearchCollectionSummaryResponse, SearchForRequest, SearchForResponse,
    SearchMediaFileSummaryRequest, SearchMediaFileSummaryResponse, SeekRequest,
    SessionSnapshotRequest, SessionSnapshotResponse, SetAdaptiveSwitchingEnabledRequest,
    SetLikedRequest, SetLikedResponse, SetPlaybackModeRequest, SetRealtimeFFTEnabledRequest,
    SwitchRequest, TakeBackHandoffBlockRequest, UpdatePlaylistRequest, UpdatePlaylistResponse,
    VolumeRequest, VolumeResponse,
};
use crate::request_error::ExpectedRequestError;
use crate::rpc::{RequestTranslationFuture, Slot, UnsupportedReason};
use crate::subsonic_binding::{SubsonicBinding, SubsonicIdKind};
use crate::utils::{RESOLVED_TRACK_LIMIT, resolved_start};
use database::actions::mixes::QueryOperator;
use reqwest::StatusCode as ReqwestStatusCode;
use std::future;
use std::result;
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
            .map_err(subsonic_request_error)?;
    } else {
        binding
            .client
            .unstar(&[&track])
            .await
            .map_err(subsonic_request_error)?;
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
        .map_err(subsonic_request_error)?;
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
        .map_err(subsonic_request_error)?;
    let needs_extension = binding
        .probe
        .extensions
        .iter()
        .any(|(name, _)| name == "songLyrics");
    let lyrics = if needs_extension {
        binding
            .client
            .lyrics_by_song_id(&track)
            .await
            .map_err(subsonic_request_error)?
    } else {
        binding
            .client
            .lyrics(song.artist.as_deref().unwrap_or_default(), &song.title)
            .await
            .map_err(subsonic_request_error)?
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
        .map_err(subsonic_request_error)?;
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
        .map_err(subsonic_request_error)?
        .songs)
}

/// The server's playlists, under the IDs this library gives them.
async fn interned_playlists(binding: &SubsonicBinding) -> Result<Vec<(i32, SubsonicPlaylist)>> {
    let remote = binding
        .client
        .playlists()
        .await
        .map_err(subsonic_request_error)?;
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
        .map_err(subsonic_request_error)?;
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

/// Tells open listings that a playlist's entries moved, so they list it again rather than
/// page or edit with occurrences of the old order.
fn playlist_entries_changed(binding: &SubsonicBinding) {
    binding
        .covers
        .broadcaster
        .broadcast(&LibraryContentChanged {
            path: String::new(),
        });
}

/// The length of a playlist fingerprint, in lowercase hexadecimal digits.
const PLAYLIST_FINGERPRINT_LEN: usize = 16;

/// Identifies one order of a playlist's entries.
fn playlist_fingerprint<'a>(songs: impl IntoIterator<Item = &'a str>) -> String {
    let mut hasher = blake3::Hasher::new();
    for song in songs {
        hasher.update(&(song.len() as u64).to_le_bytes());
        hasher.update(song.as_bytes());
    }
    hasher.finalize().to_hex()[..PLAYLIST_FINGERPRINT_LEN].to_owned()
}

fn is_playlist_fingerprint(value: &str) -> bool {
    value.len() == PLAYLIST_FINGERPRINT_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Names a playlist entry. Subsonic names entries only by their order, so an occurrence is
/// a position together with the fingerprint of the order it was listed in.
fn occurrence_id(position: impl std::fmt::Display, fingerprint: &str) -> String {
    format!("{position}@{fingerprint}")
}

/// The position and listing fingerprint an occurrence ID names.
fn parse_occurrence_id(value: &str) -> Result<(usize, &str)> {
    value
        .split_once('@')
        .filter(|(_, fingerprint)| is_playlist_fingerprint(fingerprint))
        .and_then(|(position, fingerprint)| Some((position.parse().ok()?, fingerprint)))
        .ok_or_else(|| {
            ExpectedRequestError::invalid_input("Invalid Subsonic playlist occurrence").into()
        })
}

/// The position of a listed occurrence in `songs`. A position names the entry it was listed
/// for only while the playlist keeps the order it was listed in, so an occurrence listed
/// before any change to the playlist is rejected.
fn occurrence_position((position, fingerprint): (usize, &str), songs: &[String]) -> Result<usize> {
    if fingerprint != playlist_fingerprint(songs.iter().map(String::as_str)) {
        return Err(ExpectedRequestError::PlaylistChanged.into());
    }
    if position >= songs.len() {
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
    let occurrence = parse_occurrence_id(&request.occurrence_id)?;
    let (remote_id, songs) = fetch_remote_playlist(binding, request.playlist_id).await?;
    let position = occurrence_position(occurrence, &songs)?;
    let position = u32::try_from(position)
        .map_err(|_| ExpectedRequestError::invalid_input("Subsonic playlist is too large"))?;
    binding
        .client
        .update_playlist(&remote_id, None, &[], &[position])
        .await
        .map_err(subsonic_request_error)?;
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
        .map_err(subsonic_request_error)?;
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

/// The occurrence a listing page continues after. A cursor names the last entry of the
/// previous page, so a cursor whose occurrence names another position did not come from a
/// listing.
fn occurrence_page_cursor(
    after: Option<&PlaylistOccurrenceCursor>,
) -> Result<Option<(usize, &str)>> {
    let Some(cursor) = after else {
        return Ok(None);
    };
    match parse_occurrence_id(&cursor.occurrence_id) {
        Ok((position, fingerprint))
            if i32::try_from(position).is_ok_and(|id| id == cursor.position) =>
        {
            Ok(Some((position, fingerprint)))
        }
        _ => Err(ExpectedRequestError::invalid_input(
            "Invalid Subsonic playlist occurrence cursor",
        )
        .into()),
    }
}

/// Lists a playlist's entries in order, repeats included, named as the playlist edits above
/// name them. A page continues only the order the previous page was listed in.
pub(crate) async fn fetch_playlist_occurrences_request(
    binding: &SubsonicBinding,
    request: &FetchPlaylistOccurrencesRequest,
) -> Result<Option<FetchPlaylistOccurrencesResponse>> {
    if !OCCURRENCE_PAGE_SIZES.contains(&request.page_size) {
        return Err(ExpectedRequestError::invalid_input(format!(
            "Playlist page size must be {}..={}",
            OCCURRENCE_PAGE_SIZES.start(),
            OCCURRENCE_PAGE_SIZES.end()
        ))
        .into());
    }
    let after = occurrence_page_cursor(request.after.as_ref())?;
    let songs = playlist_songs(binding, request.playlist_id).await?;
    let fingerprint = playlist_fingerprint(songs.iter().map(|song| song.id.as_str()));
    let start = match after {
        None => 0,
        Some((position, listed)) if listed == fingerprint => position + 1,
        Some(_) => return Err(ExpectedRequestError::PlaylistChanged.into()),
    };
    let end = songs
        .len()
        .min(start.saturating_add(usize::try_from(request.page_size)?));
    let more = end < songs.len();
    let mut covers = Vec::new();
    let entries = {
        let mut catalog = binding
            .catalog
            .lock()
            .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
        songs
            .into_iter()
            .enumerate()
            .take(end)
            .skip(start)
            .map(|(position, song)| {
                let position = i32::try_from(position).map_err(|_| {
                    ExpectedRequestError::invalid_input("Subsonic playlist is too large")
                })?;
                let id = catalog.ids.intern(SubsonicIdKind::Track, &song.id)?;
                let cover_art_id = song
                    .cover_art
                    .as_deref()
                    .map(|cover| catalog.ids.intern(SubsonicIdKind::CoverArt, cover))
                    .transpose()?;
                if let Some(cover) = &song.cover_art {
                    covers.push((id, cover.clone()));
                }
                let file = subsonic_media_file(binding, id, cover_art_id, &song);
                let track_uuid = song.id.clone();
                catalog.songs.insert(id, song);
                Ok(PlaylistOccurrence {
                    occurrence_id: occurrence_id(position, &fingerprint),
                    position,
                    track_uuid,
                    file: Some(file),
                })
            })
            .collect::<Result<Vec<_>>>()?
    };
    let next = more
        .then(|| entries.last())
        .flatten()
        .map(|entry| PlaylistOccurrenceCursor {
            position: entry.position,
            occurrence_id: entry.occurrence_id.clone(),
        });
    let cover_art_map = if request.bake_cover_arts {
        subsonic_cover_map(binding, covers).await?
    } else {
        HashMap::new()
    };
    Ok(Some(FetchPlaylistOccurrencesResponse {
        request_id: request.request_id.clone(),
        entries,
        next,
        cover_art_map,
        call_id: request.call_id,
    }))
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
        .map_err(subsonic_request_error)?;
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

fn empty_search_error(error: SubsonicError, request: &'static str) -> anyhow::Error {
    match error {
        SubsonicError::Server {
            kind: SubsonicServerError::MissingParameter,
            ..
        } => ExpectedRequestError::Unsupported {
            request,
            reason: UnsupportedReason::NotImplemented,
        }
        .into(),
        error => subsonic_request_error(error),
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
        .map_err(|error| empty_search_error(error, FetchMediaFilesRequest::wire_name()))?
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
        .map_err(subsonic_request_error)?;
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
        .map_err(subsonic_request_error)?;
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
        .map_err(|error| empty_search_error(error, SearchMediaFileSummaryRequest::wire_name()))?
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
    let results =
        binding.client.search("", limits).await.map_err(|error| {
            empty_search_error(error, SearchCollectionSummaryRequest::wire_name())
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

fn jukebox_error(error: SubsonicError, request: &'static str) -> anyhow::Error {
    match error {
        SubsonicError::Transport(_) => ExpectedRequestError::Unavailable {
            what: "Subsonic jukebox",
        }
        .into(),
        SubsonicError::HttpStatus(status) if status == ReqwestStatusCode::UNAUTHORIZED => {
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
            .map_err(|error| jukebox_error(error, NextRequest::wire_name()))?;
    } else {
        binding
            .client
            .jukebox_stop()
            .await
            .map_err(|error| jukebox_error(error, NextRequest::wire_name()))?;
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
        .map_err(|error| jukebox_error(error, request))?;
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
            .map_err(|error| jukebox_error(error, request))?;
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
        .map_err(|error| jukebox_error(error, request))?;
    binding
        .client
        .jukebox_skip(current, queue.status.position_secs.unwrap_or_default())
        .await
        .map_err(|error| jukebox_error(error, request))?;
    if queue.status.playing {
        binding
            .client
            .jukebox_start()
            .await
            .map_err(|error| jukebox_error(error, request))?;
    } else {
        binding
            .client
            .jukebox_stop()
            .await
            .map_err(|error| jukebox_error(error, request))?;
    }
    Ok(current + 1)
}

async fn jukebox_load_resolved_items_request(
    binding: &SubsonicBinding,
    request: &LoadResolvedItemsRequest,
) -> Result<Option<LoadResolvedItemsResponse>> {
    let error = |error| jukebox_error(error, LoadResolvedItemsRequest::wire_name());
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

async fn collection_groups(
    binding: &SubsonicBinding,
    collection_type: CollectionType,
    group_titles: &[String],
) -> Result<Vec<CollectionGroup>> {
    let playlists = match collection_type {
        CollectionType::Playlist => interned_playlists(binding).await?,
        _ => Vec::new(),
    };
    let entries: Vec<(i32, String, String, Option<String>)> = {
        let catalog = binding
            .catalog
            .lock()
            .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
        match collection_type {
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
            CollectionType::Playlist => playlists
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
    let playlists = match request.collection_type {
        CollectionType::Artist => {
            ensure_artists(binding).await?;
            Vec::new()
        }
        CollectionType::Album => {
            ensure_albums(binding).await?;
            Vec::new()
        }
        CollectionType::Playlist => interned_playlists(binding).await?,
        _ => {
            return Err(ExpectedRequestError::Unsupported {
                request: FetchCollectionGroupSummaryRequest::wire_name(),
                reason: UnsupportedReason::NotImplemented,
            }
            .into());
        }
    };
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
        CollectionType::Playlist => playlists
            .iter()
            .map(|(_, item)| item.name.as_str())
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
        CollectionType::Playlist => {}
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

fn parse_detail_query(queries: &[MixQuery], request: &'static str) -> Result<DetailQuery> {
    let Ok(operators) = queries
        .iter()
        .map(|query| QueryOperator::parse(&query.operator, &query.parameter))
        .collect::<result::Result<Vec<_>, _>>()
    else {
        return Err(unsupported_detail_query(request).into());
    };
    match operators.as_slice() {
        [QueryOperator::LibArtist(id)] => Ok(DetailQuery::Artist(*id)),
        [QueryOperator::LibAlbum(id)] => Ok(DetailQuery::Album(*id)),
        // An album's page asks for its tracks in track-number order, and `detail_songs` already
        // lists an album by disc and track number, the order a local library's track numbers
        // (disc * 1000 + track) give.
        [
            QueryOperator::LibAlbum(id),
            QueryOperator::SortTrackNumber(true),
        ]
        | [
            QueryOperator::SortTrackNumber(true),
            QueryOperator::LibAlbum(id),
        ] => Ok(DetailQuery::Album(*id)),
        [
            QueryOperator::LibRandom(count),
            QueryOperator::FilterWithCoverArt(true),
        ]
        | [
            QueryOperator::FilterWithCoverArt(true),
            QueryOperator::LibRandom(count),
        ] => Ok(DetailQuery::RandomCovers(
            u32::try_from(*count)
                .unwrap_or(1)
                .clamp(1, SUBSONIC_RANDOM_SONGS_MAX),
        )),
        _ => Err(unsupported_detail_query(request).into()),
    }
}

fn unsupported_detail_query(request: &'static str) -> ExpectedRequestError {
    ExpectedRequestError::Unsupported {
        request,
        reason: UnsupportedReason::NotImplemented,
    }
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
                .map_err(subsonic_request_error)?;
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
        intern_songs(binding, &album.songs)?;
        songs.extend(album.songs);
    }
    Ok(songs)
}

/// Waits until every cover of `songs` is in the local cover cache, fetching at most four at a
/// time, and returns the IDs of the covers that are now cached. A cover that cannot be fetched
/// is left out rather than failing the others.
async fn cache_covers(binding: &SubsonicBinding, songs: &[SubsonicSong]) -> HashSet<String> {
    let cover_ids = songs
        .iter()
        .filter_map(|song| song.cover_art.clone())
        .collect::<HashSet<_>>();
    stream::iter(cover_ids)
        .map(|cover_id| async move {
            match binding
                .covers
                .ensure_cached(Arc::clone(&binding.client), &cover_id)
                .await
            {
                Ok(Some(_)) => Some(cover_id),
                Ok(None) => {
                    log::debug!("Subsonic cover {cover_id} is not available");
                    None
                }
                Err(error) => {
                    log::debug!("Subsonic cover {cover_id} could not be cached: {error:#}");
                    None
                }
            }
        })
        // The cover cache downloads four covers at a time, so waiting on more only queues them.
        .buffer_unordered(4)
        .filter_map(future::ready)
        .collect()
        .await
}

pub(crate) async fn mix_query_request(
    binding: &SubsonicBinding,
    request: &MixQueryRequest,
) -> Result<Option<MixQueryResponse>> {
    let query = parse_detail_query(&request.queries, MixQueryRequest::wire_name())?;
    let cover_wall = matches!(query, DetailQuery::RandomCovers(_));
    // `getRandomSongs` takes no seed, so the pages of a seeded random list could not agree.
    if cover_wall && request.random_seed.is_some() {
        return Err(unsupported_detail_query(MixQueryRequest::wire_name()).into());
    }
    let mut songs = detail_songs(binding, query)
        .await?
        .into_iter()
        .skip(usize::try_from(request.cursor.max(0)).unwrap_or(usize::MAX))
        .take(usize::try_from(request.page_size.max(0)).unwrap_or(usize::MAX))
        .collect::<Vec<_>>();
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
    let translated = {
        let mut catalog = binding
            .catalog
            .lock()
            .map_err(|_| anyhow!("Subsonic catalog lock poisoned"))?;
        songs
            .into_iter()
            .map(|song| {
                let id = catalog.ids.intern(SubsonicIdKind::Track, &song.id)?;
                catalog.songs.insert(id, song.clone());
                let cover_art_id = song
                    .cover_art
                    .as_deref()
                    .map(|cover| catalog.ids.intern(SubsonicIdKind::CoverArt, cover))
                    .transpose()?;
                let media_file = subsonic_media_file(binding, id, cover_art_id, &song);
                Ok((media_file, song.cover_art))
            })
            .collect::<Result<Vec<_>>>()?
    };
    let covers = translated
        .iter()
        .filter_map(|(file, cover)| cover.clone().map(|cover| (file.id, cover)))
        .collect();
    let mut files = translated
        .into_iter()
        .map(|(file, _)| file)
        .collect::<Vec<_>>();
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

/// A track list a click plays from: the tracks page, an artist's or album's tracks, or a
/// playlist's entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrackListQuery {
    All,
    Detail(DetailQuery),
    Playlist(i32),
}

fn parse_track_list_query(queries: &[MixQuery]) -> Result<TrackListQuery> {
    if let [query] = queries {
        match QueryOperator::parse(&query.operator, &query.parameter) {
            Ok(QueryOperator::LibAll(true)) => return Ok(TrackListQuery::All),
            Ok(QueryOperator::LibPlaylist(id)) => return Ok(TrackListQuery::Playlist(id)),
            _ => {}
        }
    }
    match parse_detail_query(queries, ResolveMixQueryRequest::wire_name())? {
        // A random list cannot be listed again, so a click in it cannot be resolved.
        DetailQuery::RandomCovers(_) => {
            Err(unsupported_detail_query(ResolveMixQueryRequest::wire_name()).into())
        }
        detail => Ok(TrackListQuery::Detail(detail)),
    }
}

/// Songs asked for per search page while resolving the tracks page.
const RESOLVE_PAGE_SIZE: u32 = 500;

/// The server's songs in the order the tracks page lists them, as many as a resolve plays.
/// Only an empty page ends the list: a server may return fewer songs than a page asks for.
async fn tracks_page_songs(binding: &SubsonicBinding) -> Result<Vec<SubsonicSong>> {
    let mut songs = Vec::new();
    while songs.len() < RESOLVED_TRACK_LIMIT {
        let page = binding
            .client
            .search_with_song_offset(
                "",
                SearchLimits {
                    songs: RESOLVE_PAGE_SIZE,
                    ..Default::default()
                },
                u32::try_from(songs.len())?,
            )
            .await
            .map_err(|error| empty_search_error(error, ResolveMixQueryRequest::wire_name()))?
            .songs;
        if page.is_empty() {
            break;
        }
        songs.extend(page);
    }
    songs.truncate(RESOLVED_TRACK_LIMIT);
    Ok(songs)
}

/// Resolves the track list a track was clicked in. As in a local library, this library's
/// tracks in the caller's queue are left out, and when the clicked track is not in the
/// result the fallback tracks play instead.
pub(crate) async fn resolve_mix_query_request(
    binding: &SubsonicBinding,
    request: &ResolveMixQueryRequest,
) -> Result<Option<ResolveMixQueryResponse>> {
    let fallback = request
        .fallback_playing_items
        .iter()
        .map(|item| subsonic_track_id(binding, Some(item)))
        .collect::<Result<Vec<_>>>()?;
    let initial = if request.instantly_play {
        request
            .initial_playback_item
            .as_ref()
            .map(|item| subsonic_track_id(binding, Some(item)))
            .transpose()?
    } else {
        None
    };
    let library_id = binding.library_id.to_string();
    let tracks = if request.queries.is_empty() {
        fallback.clone()
    } else {
        let query = parse_track_list_query(&request.queries)?;
        let queued = request
            .excluded
            .items
            .iter()
            .filter_map(|item| match item {
                PlayingItemRef::LibraryTrack {
                    library_id: item_library,
                    track,
                } if *item_library == library_id => Some(track.as_str()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let songs = match query {
            TrackListQuery::All => tracks_page_songs(binding).await?,
            TrackListQuery::Detail(query) => detail_songs(binding, query).await?,
            TrackListQuery::Playlist(id) => playlist_songs(binding, id).await?,
        };
        songs
            .into_iter()
            .take(RESOLVED_TRACK_LIMIT)
            .map(|song| song.id)
            .filter(|track| !queued.contains(track.as_str()))
            .collect()
    };
    let (tracks, start_index) = if request.instantly_play {
        let (tracks, start_index) =
            resolved_start(request.hint_position, tracks, initial.as_ref(), fallback);
        let start_index = u32::try_from(start_index).map_err(|error| {
            ExpectedRequestError::invalid_input(format!("Invalid start index: {error}"))
        })?;
        (tracks, Some(start_index))
    } else {
        (tracks, None)
    };
    Ok(Some(ResolveMixQueryResponse {
        playing_items: tracks
            .into_iter()
            .map(|track| PlayingItemRef::LibraryTrack {
                library_id: library_id.clone(),
                track,
            })
            .collect(),
        start_index,
        call_id: request.call_id,
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
                $(slots.set::<$native_request>(Slot::Native);)*
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
    }
    routed_native {
        SetLikedRequest,
        GetLikedRequest,
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
        add_item_to_playlist_request, create_playlist_request, fetch_all_playlists_request,
        fetch_collection_by_ids_request, fetch_collection_group_summary_request,
        fetch_collection_groups_request, fetch_media_file_by_ids_request,
        fetch_media_files_request, fetch_parsed_media_file_request,
        fetch_playlist_occurrences_request, fetch_remote_playlist, get_liked_request,
        get_lyric_request, get_media_files_count_request, get_playlist_by_id_request,
        jukebox_error, mix_query_request, remove_item_from_playlist_request,
        remove_playlist_request, reorder_playlist_item_position_request, resolve_playlist_id,
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
                                .map(|id| mock_song(id))
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
        binding.probe.extensions.clear();
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
        assert_eq!(GetLyricByTrackIdRequest::slot(&slots), Slot::Translate);
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
            ResolveMixQueryRequest::slot(&RequestSlots::subsonic(&binding.probe)),
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
        let slots = RequestSlots::subsonic(&binding.probe);
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
