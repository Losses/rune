use anyhow::{Result, anyhow};
use database::actions::utils::generate_group_name;
use database::actions::{collection::CollectionQueryType, search::convert_to_collection_types};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
use subsonic::{
    SearchLimits, SubsonicAlbum, SubsonicArtist, SubsonicError, SubsonicServerError, SubsonicSong,
};

use crate::TypedRequest;
use crate::macros::{RequestSlots, RequestTranslator};
use crate::messages::{
    AudioAvailability, AudioOrigin, CastPreflightRequest, Collection, CollectionGroup,
    CollectionGroupSummary, CollectionGroupSummaryResponse, CollectionType,
    FetchCollectionByIdsRequest, FetchCollectionByIdsResponse, FetchCollectionGroupSummaryRequest,
    FetchCollectionGroupsRequest, FetchCollectionGroupsResponse, FetchMediaFileByIdsRequest,
    FetchMediaFileByIdsResponse, LoadRequest, LoadResolvedItemsRequest, MediaFile, MixQueryRequest,
    MixQueryResponse, MovePlaylistItemRequest, NextRequest, PauseRequest, PlayRequest,
    PreviousRequest, RefreshSessionRequest, RemoveRequest, SearchForRequest, SearchForResponse,
    SeekRequest, SessionSnapshotRequest, SetAdaptiveSwitchingEnabledRequest,
    SetPlaybackModeRequest, SetRealtimeFFTEnabledRequest, SwitchRequest, VolumeRequest,
};
use crate::request_error::ExpectedRequestError;
use crate::rpc::{RequestTranslationFuture, Slot, UnsupportedReason};
use crate::subsonic_binding::{SubsonicBinding, SubsonicIdKind};
use database::actions::mixes::{QueryOperator, parse_query};

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
                SubsonicIdKind::Track | SubsonicIdKind::CoverArt => None,
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
                SubsonicIdKind::Track | SubsonicIdKind::CoverArt => None,
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
                $(slots.set::<$request>(Slot::Translate);)*
                slots
            }
        }
    };
}

subsonic_requests! {
    supported {
        FetchMediaFileByIdsRequest => fetch_media_file_by_ids_request as translate_fetch_media_file_by_ids_request,
        FetchCollectionGroupSummaryRequest => fetch_collection_group_summary_request as translate_fetch_collection_group_summary_request,
        FetchCollectionGroupsRequest => fetch_collection_groups_request as translate_fetch_collection_groups_request,
        FetchCollectionByIdsRequest => fetch_collection_by_ids_request as translate_fetch_collection_by_ids_request,
        MixQueryRequest => mix_query_request as translate_mix_query_request,
        SearchForRequest => search_for_request as translate_search_for_request,
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
        extract::{Path, Query, State},
        http::{Response, StatusCode},
        routing::get,
    };
    use subsonic::{
        SubsonicClient, SubsonicCredentials, SubsonicError, SubsonicProbe, SubsonicRoles,
    };
    use tempfile::TempDir;
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;
    use uuid::Uuid;

    use super::{
        fetch_collection_group_summary_request, fetch_collection_groups_request,
        fetch_media_file_by_ids_request, search_for_request, subsonic_request_error,
    };
    use crate::{
        messages::{
            CollectionType, FetchCollectionGroupSummaryRequest, FetchCollectionGroupsRequest,
            FetchMediaFileByIdsRequest, SearchForRequest,
        },
        request_error::{ExpectedRequestError, classify},
        subsonic_binding::SubsonicBinding,
        utils::{Broadcaster, RinfRustSignal},
    };

    use crate::messages::RequestErrorKind;

    #[derive(Clone)]
    struct MockState {
        cover_calls: Arc<AtomicUsize>,
        requests: Arc<Mutex<Vec<String>>>,
    }

    struct MockServer {
        url: String,
        state: MockState,
        _task: JoinHandle<()>,
    }

    struct QuietBroadcaster;

    impl Broadcaster for QuietBroadcaster {
        fn broadcast(&self, _: &dyn RinfRustSignal) {}
    }

    async fn mock_subsonic(root: &TempDir) -> (SubsonicBinding, MockServer) {
        let state = MockState {
            cover_calls: Arc::new(AtomicUsize::new(0)),
            requests: Arc::new(Mutex::new(Vec::new())),
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
        let binding = SubsonicBinding::new(
            client,
            SubsonicProbe {
                api_version: "1.16.1".into(),
                open_subsonic: false,
                server_type: None,
                server_version: None,
                extensions: Vec::new(),
                roles: SubsonicRoles::default(),
                roles_known: true,
            },
            root.path().to_path_buf(),
            Uuid::new_v4(),
            false,
            Arc::new(QuietBroadcaster),
        );
        (
            binding,
            MockServer {
                url,
                state,
                _task: task,
            },
        )
    }

    async fn mock_endpoint(
        State(state): State<MockState>,
        Path(endpoint): Path<String>,
        Query(query): Query<HashMap<String, String>>,
    ) -> Response<Body> {
        let endpoint = endpoint.trim_end_matches(".view").to_owned();
        let cover_id = query.get("id").cloned().unwrap_or_default();
        state.requests.lock().unwrap().push(endpoint.clone());
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
        let (binding, server) = mock_subsonic(&root).await;
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

    fn assert_secret_free(value: &str) {
        for secret in ["t=", "s=", "p=", "cover-secret-password"] {
            assert!(!value.contains(secret));
        }
    }

    #[tokio::test]
    async fn browsed_collection_returns_cached_local_path_after_background_fetch() {
        let root = TempDir::new().unwrap();
        let (binding, server) = mock_subsonic(&root).await;
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
}
