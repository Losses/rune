use crate::request_error::ExpectedRequestError;
use database::actions::cover_art::bake_cover_art_by_media_files;
use database::actions::metadata::get_metadata_summary_by_files;
use database::actions::playlists::{
    OccurrenceCursor, add_item_to_playlist, create_m3u8_playlist, create_playlist,
    list_occurrences, remove_item_from_playlist, remove_playlist, reorder_playlist_item_position,
    update_playlist,
};
use database::error::ExpectedError;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

use anyhow::{Context, Result};

use crate::library_pool::LibraryHandle;
use crate::utils::{GlobalParams, ParamsExtractor, require_library};
use crate::{Session, Signal, messages::*};
use ::database::actions::playlists::{get_all_playlists, get_playlist_by_id};

type PlaylistArgs = Option<LibraryHandle>;

fn playlist_args(params: &GlobalParams) -> PlaylistArgs {
    params.library.clone()
}

impl ParamsExtractor for FetchAllPlaylistsRequest {
    type Params = Option<LibraryHandle>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.library.clone()
    }
}

impl Signal for FetchAllPlaylistsRequest {
    type Params = Option<LibraryHandle>;
    type Response = FetchAllPlaylistsResponse;
    async fn handle(
        &self,
        library: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let playlists = get_all_playlists(&library.main_db)
            .await
            .with_context(|| "Failed to fetch all playlists")?;

        Ok(Some(FetchAllPlaylistsResponse {
            playlists: playlists
                .into_iter()
                .map(|playlist| Playlist {
                    id: playlist.id,
                    name: playlist.name,
                    group: playlist.group,
                })
                .collect(),
        }))
    }
}

impl ParamsExtractor for CreatePlaylistRequest {
    type Params = (PlaylistArgs, Arc<dyn crate::utils::Broadcaster>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (playlist_args(all_params), all_params.broadcaster.clone())
    }
}

impl Signal for CreatePlaylistRequest {
    type Params = (PlaylistArgs, Arc<dyn crate::utils::Broadcaster>);
    type Response = CreatePlaylistResponse;
    async fn handle(
        &self,
        (library, broadcaster): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;

        let name = &request.name;
        let group = &request.group;

        let playlist = create_playlist(
            &library.main_db,
            &library.library_sync.record_store(),
            name.clone(),
            group.clone(),
        )
        .await
        .with_context(|| format!("Failed to create playlist: name={name}, group={group}"))?;

        broadcaster.broadcast(&LibraryContentChanged {
            path: library.descriptor.app_name(),
        });
        Ok(Some(CreatePlaylistResponse {
            playlist: Playlist {
                id: playlist.id,
                name: playlist.name,
                group: playlist.group,
            },
        }))
    }
}

impl ParamsExtractor for UpdatePlaylistRequest {
    type Params = (PlaylistArgs, Arc<dyn crate::utils::Broadcaster>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (playlist_args(all_params), all_params.broadcaster.clone())
    }
}

impl Signal for UpdatePlaylistRequest {
    type Params = (PlaylistArgs, Arc<dyn crate::utils::Broadcaster>);
    type Response = UpdatePlaylistResponse;
    async fn handle(
        &self,
        (library, broadcaster): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;

        let name = &request.name;
        let group = &request.group;

        let playlist = update_playlist(
            &library.main_db,
            &library.library_sync.record_store(),
            request.playlist_id,
            Some(name.clone()),
            Some(group.clone()),
        )
        .await
        .with_context(|| {
            format!(
                "Failed to update playlist: id={}, name={:?}, group={:?}",
                request.playlist_id, name, group
            )
        })?;

        broadcaster.broadcast(&LibraryContentChanged {
            path: library.descriptor.app_name(),
        });
        Ok(Some(UpdatePlaylistResponse {
            playlist: Playlist {
                id: playlist.id,
                name: playlist.name,
                group: playlist.group,
            },
        }))
    }
}

impl ParamsExtractor for RemovePlaylistRequest {
    type Params = (PlaylistArgs, Arc<dyn crate::utils::Broadcaster>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (playlist_args(all_params), all_params.broadcaster.clone())
    }
}

impl Signal for RemovePlaylistRequest {
    type Params = (PlaylistArgs, Arc<dyn crate::utils::Broadcaster>);
    type Response = RemovePlaylistResponse;
    async fn handle(
        &self,
        (library, broadcaster): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;

        remove_playlist(
            &library.main_db,
            &library.library_sync.record_store(),
            request.playlist_id,
        )
        .await
        .with_context(|| format!("Removing playlist: id={}", request.playlist_id))?;

        broadcaster.broadcast(&LibraryContentChanged {
            path: library.descriptor.app_name(),
        });
        Ok(Some(RemovePlaylistResponse {
            playlist_id: request.playlist_id,
            success: true,
        }))
    }
}

impl ParamsExtractor for AddItemToPlaylistRequest {
    type Params = (PlaylistArgs, Arc<dyn crate::utils::Broadcaster>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (playlist_args(all_params), all_params.broadcaster.clone())
    }
}

impl Signal for AddItemToPlaylistRequest {
    type Params = (PlaylistArgs, Arc<dyn crate::utils::Broadcaster>);
    type Response = AddItemToPlaylistResponse;
    async fn handle(
        &self,
        (library, broadcaster): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;

        let entry = add_item_to_playlist(
            &library.main_db,
            &library.library_sync.record_store(),
            request.playlist_id,
            request.media_file_id,
            request.position,
        )
        .await
        .with_context(|| {
            format!(
                "Failed to add item to playlist: playlist_id={}, media_file_id={}, position={:#?}",
                request.playlist_id, request.media_file_id, request.position
            )
        })?;

        broadcaster.broadcast(&LibraryContentChanged {
            path: library.descriptor.app_name(),
        });
        Ok(Some(AddItemToPlaylistResponse {
            success: true,
            occurrence_id: entry.hlc_uuid,
        }))
    }
}

impl ParamsExtractor for ReorderPlaylistItemPositionRequest {
    type Params = (PlaylistArgs, Arc<dyn crate::utils::Broadcaster>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (playlist_args(all_params), all_params.broadcaster.clone())
    }
}

impl Signal for ReorderPlaylistItemPositionRequest {
    type Params = (PlaylistArgs, Arc<dyn crate::utils::Broadcaster>);
    type Response = ReorderPlaylistItemPositionResponse;
    async fn handle(
        &self,
        (library, broadcaster): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;

        reorder_playlist_item_position(
            &library.main_db, &library.library_sync.record_store(), request.playlist_id,
            parse_occurrence_id(&request.occurrence_id)?,
            request.new_position,
        )
        .await
        .with_context(|| {
            format!(
                "Failed to reorder playlist item: playlist_id={}, occurrence_id={}, new_position={}",
                request.playlist_id, request.occurrence_id, request.new_position
            )
        })?;

        broadcaster.broadcast(&LibraryContentChanged {
            path: library.descriptor.app_name(),
        });
        Ok(Some(ReorderPlaylistItemPositionResponse { success: true }))
    }
}

impl ParamsExtractor for GetPlaylistByIdRequest {
    type Params = Option<LibraryHandle>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.library.clone()
    }
}

impl Signal for GetPlaylistByIdRequest {
    type Params = Option<LibraryHandle>;
    type Response = GetPlaylistByIdResponse;
    async fn handle(
        &self,
        library: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;

        let playlist = get_playlist_by_id(&library.main_db, request.playlist_id)
            .await
            .with_context(|| format!("Failed to get playlist by id: {}", request.playlist_id))?
            .ok_or(ExpectedError::NotFound { what: "playlist" })
            .with_context(|| format!("Playlist not found with id: {}", request.playlist_id))?;

        Ok(Some(GetPlaylistByIdResponse {
            playlist: Playlist {
                id: playlist.id,
                name: playlist.name,
                group: playlist.group,
            },
        }))
    }
}

impl ParamsExtractor for CreateM3u8PlaylistRequest {
    type Params = (PlaylistArgs, Arc<dyn crate::utils::Broadcaster>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (playlist_args(all_params), all_params.broadcaster.clone())
    }
}

impl Signal for CreateM3u8PlaylistRequest {
    type Params = (PlaylistArgs, Arc<dyn crate::utils::Broadcaster>);
    type Response = CreateM3u8PlaylistResponse;
    async fn handle(
        &self,
        (library, broadcaster): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;

        let name = &request.name;
        let group = &request.group;
        let path = &request.path;

        match create_m3u8_playlist(
            &library.main_db,
            &library.library_sync.record_store(),
            name.clone(),
            group.clone(),
            Path::new(&path),
        )
        .await
        {
            Ok((playlist, import_result)) => {
                broadcaster.broadcast(&LibraryContentChanged {
                    path: library.descriptor.app_name(),
                });
                Ok(Some(CreateM3u8PlaylistResponse {
                    playlist: Some(Playlist {
                        id: playlist.id,
                        name: playlist.name,
                        group: playlist.group,
                    }),
                    imported_count: Some(import_result.matched_ids.len() as i32),
                    not_found_paths: import_result.unmatched_paths,
                    success: true,
                    error: String::new(),
                }))
            }
            Err(e) => Ok(Some(CreateM3u8PlaylistResponse {
                playlist: None,
                imported_count: Some(0),
                not_found_paths: vec![],
                success: false,
                error: e.to_string(),
            })),
        }
    }
}

impl ParamsExtractor for RemoveItemFromPlaylistRequest {
    type Params = (PlaylistArgs, Arc<dyn crate::utils::Broadcaster>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (playlist_args(all_params), all_params.broadcaster.clone())
    }
}

impl Signal for RemoveItemFromPlaylistRequest {
    type Params = (PlaylistArgs, Arc<dyn crate::utils::Broadcaster>);
    type Response = RemoveItemFromPlaylistResponse;

    async fn handle(
        &self,
        (library, broadcaster): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;

        match remove_item_from_playlist(
            &library.main_db,
            &library.library_sync.record_store(),
            request.playlist_id,
            parse_occurrence_id(&request.occurrence_id)?,
        )
        .await
        {
            Ok(_) => {
                broadcaster.broadcast(&LibraryContentChanged {
                    path: library.descriptor.app_name(),
                });
                Ok(Some(RemoveItemFromPlaylistResponse {
                    success: true,
                    error: String::new(),
                }))
            }
            Err(e) => Ok(Some(RemoveItemFromPlaylistResponse {
                success: false,
                error: e.to_string(),
            })),
        }
    }
}

impl ParamsExtractor for FetchPlaylistOccurrencesRequest {
    type Params = (Option<LibraryHandle>, Arc<String>);
    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        (params.library.clone(), params.node_id.clone())
    }
}

impl Signal for FetchPlaylistOccurrencesRequest {
    type Params = (Option<LibraryHandle>, Arc<String>);
    type Response = FetchPlaylistOccurrencesResponse;
    async fn handle(
        &self,
        (library, node): Self::Params,
        _session: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let page = list_occurrences(
            &library.main_db,
            request.playlist_id,
            request.after.as_ref().map(|cursor| OccurrenceCursor {
                position: cursor.position,
                occurrence_id: cursor.occurrence_id.clone(),
            }),
            request.page_size,
        )
        .await?;
        let source_files = page
            .entries
            .iter()
            .filter_map(|entry| entry.file.clone())
            .collect::<Vec<_>>();
        let summaries =
            get_metadata_summary_by_files(library.main_db.as_ref(), source_files.clone()).await?;
        let root = Arc::new(library.media_root.clone());
        let files = crate::utils::parse_media_files(
            &library.fsio,
            &library.main_db,
            &node,
            summaries,
            root,
        )
        .await?;
        let files: std::collections::HashMap<_, _> =
            files.into_iter().map(|file| (file.id, file)).collect();
        let cover_art_map = if request.bake_cover_arts {
            bake_cover_art_by_media_files(&library.fsio, &library.main_db, source_files).await?
        } else {
            HashMap::new()
        };
        Ok(Some(FetchPlaylistOccurrencesResponse {
            request_id: request.request_id.clone(),
            entries: page
                .entries
                .into_iter()
                .map(|entry| PlaylistOccurrence {
                    occurrence_id: entry.occurrence_id,
                    position: entry.position,
                    track_uuid: entry.track_id.to_string(),
                    file: entry.file.and_then(|file| files.get(&file.id).cloned()),
                })
                .collect(),
            next: page.next.map(|cursor| PlaylistOccurrenceCursor {
                position: cursor.position,
                occurrence_id: cursor.occurrence_id,
            }),
            cover_art_map,
        }))
    }
}

fn parse_occurrence_id(value: &str) -> Result<Uuid> {
    value
        .parse()
        .map_err(|error| {
            ExpectedRequestError::invalid_input(format!("invalid occurrence UUID: {error}"))
        })
        .map_err(Into::into)
}

#[cfg(test)]
mod request_error_tests {
    use super::parse_occurrence_id;
    use crate::{messages::RequestErrorKind, request_error::classify};

    #[test]
    fn invalid_occurrence_uuid_is_invalid_input() {
        let error = parse_occurrence_id("not-a-uuid").unwrap_err();
        assert!(format!("{error:#}").contains("invalid occurrence UUID"));
        assert_eq!(classify(&error), RequestErrorKind::InvalidInput);
    }
}
