use anyhow::{Result, anyhow};
use database::actions::utils::generate_group_name;
use database::actions::{collection::CollectionQueryType, search::convert_to_collection_types};
use std::collections::BTreeMap;
use subsonic::{
    SearchLimits, SubsonicAlbum, SubsonicArtist, SubsonicError, SubsonicServerError, SubsonicSong,
};

use crate::TypedRequest;
use crate::messages::{
    AudioAvailability, AudioOrigin, Collection, CollectionGroup, CollectionGroupSummary,
    CollectionGroupSummaryResponse, CollectionType, FetchCollectionByIdsRequest,
    FetchCollectionByIdsResponse, FetchCollectionGroupSummaryRequest, FetchCollectionGroupsRequest,
    FetchCollectionGroupsResponse, FetchMediaFileByIdsRequest, FetchMediaFileByIdsResponse,
    MediaFile, MixQueryRequest, MixQueryResponse, SearchForRequest, SearchForResponse,
};
use crate::request_error::ExpectedRequestError;
use crate::rpc::UnsupportedReason;
use crate::subsonic_binding::{SubsonicBinding, SubsonicIdKind};
use database::actions::mixes::{QueryOperator, parse_query};

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
        media_files.push(MediaFile {
            id: *id,
            path: String::new(),
            artist: song.artist.unwrap_or_default(),
            album: song.album.unwrap_or_default(),
            title: song.title,
            duration: f64::from(song.duration.unwrap_or_default()),
            cover_art_id: 0,
            track_number: song
                .track
                .and_then(|number| i32::try_from(number).ok())
                .unwrap_or_default(),
            audio_availability: AudioAvailability::Unavailable,
            audio_origin: AudioOrigin::Remote,
        });
    }
    Ok(Some(FetchMediaFileByIdsResponse {
        media_files,
        cover_art_map: Default::default(),
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
                SubsonicIdKind::Track => None,
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
                SubsonicIdKind::Track => {
                    return Err(anyhow!("Unexpected Subsonic track collection"));
                }
            },
        };
        result.push(Collection {
            id: *id,
            name,
            queries: Vec::new(),
            collection_type: request.collection_type,
            cover_art_map: Default::default(),
            readonly: true,
        });
    }
    Ok(Some(FetchCollectionByIdsResponse {
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
    let entries: Vec<(i32, String, String)> = match collection_type {
        CollectionType::Artist => catalog
            .artists
            .iter()
            .map(|(id, artist)| (*id, artist.name.clone(), generate_group_name(&artist.name)))
            .collect(),
        CollectionType::Album => catalog
            .albums
            .iter()
            .map(|(id, album)| (*id, album.name.clone(), generate_group_name(&album.name)))
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
                .filter(|(_, _, group)| group == title)
                .map(|(id, name, _)| Collection {
                    id: *id,
                    name: name.clone(),
                    queries: Vec::new(),
                    collection_type,
                    cover_art_map: Default::default(),
                    readonly: true,
                })
                .collect();
            collections.sort_by(|left, right| left.name.cmp(&right.name));
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
    let files = songs
        .into_iter()
        .skip(usize::try_from(request.cursor.max(0)).unwrap_or(usize::MAX))
        .take(usize::try_from(request.page_size.max(0)).unwrap_or(usize::MAX))
        .map(|song| {
            let id = catalog.ids.intern(SubsonicIdKind::Track, &song.id)?;
            catalog.songs.insert(id, song.clone());
            Ok(MediaFile {
                id,
                path: String::new(),
                artist: song.artist.unwrap_or_default(),
                album: song.album.unwrap_or_default(),
                title: song.title,
                duration: f64::from(song.duration.unwrap_or_default()),
                cover_art_id: 0,
                track_number: song
                    .track
                    .and_then(|value| i32::try_from(value).ok())
                    .unwrap_or_default(),
                audio_availability: AudioAvailability::Unavailable,
                audio_origin: AudioOrigin::Remote,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(MixQueryResponse {
        files,
        cover_art_map: Default::default(),
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

#[cfg(test)]
mod tests {
    use subsonic::{SubsonicError, SubsonicServerError};

    use super::subsonic_request_error;
    use crate::messages::RequestErrorKind;
    use crate::request_error::{ExpectedRequestError, classify};

    #[test]
    fn rejected_request_credentials_are_typed_as_not_authorized() {
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
