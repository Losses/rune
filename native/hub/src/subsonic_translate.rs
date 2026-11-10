use anyhow::{Result, anyhow};
use database::actions::{collection::CollectionQueryType, search::convert_to_collection_types};
use subsonic::{SearchLimits, SubsonicAlbum, SubsonicArtist, SubsonicError, SubsonicServerError};

use crate::messages::{
    AudioAvailability, AudioOrigin, Collection, CollectionType, FetchCollectionByIdsRequest,
    FetchCollectionByIdsResponse, FetchMediaFileByIdsRequest, FetchMediaFileByIdsResponse,
    MediaFile, SearchForRequest, SearchForResponse,
};
use crate::request_error::ExpectedRequestError;
use crate::rpc::UnsupportedReason;
use crate::subsonic_binding::{SubsonicBinding, SubsonicIdKind};

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
                request: "FetchCollectionByIdsRequest",
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
