use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};

use ::database::actions::playlists::{get_all_playlists, get_playlist_by_id};
use ::database::connection::MainDbConnection;

use crate::utils::{GlobalParams, ParamsExtractor};
use crate::{Session, Signal, messages::*};

type PlaylistArgs = (
    Arc<MainDbConnection>,
    Option<database::sync::SeaOrmSyncStore>,
    Arc<String>,
);

fn playlist_args(params: &GlobalParams) -> PlaylistArgs {
    (
        params.main_db.clone(),
        params.library_sync.as_ref().map(|sync| sync.record_store()),
        params.node_id.clone(),
    )
}

impl ParamsExtractor for FetchAllPlaylistsRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for FetchAllPlaylistsRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = FetchAllPlaylistsResponse;
    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let playlists = get_all_playlists(&main_db)
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
    type Params = (
        PlaylistArgs,
        Arc<dyn crate::utils::Broadcaster>,
        Arc<String>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            playlist_args(all_params),
            all_params.broadcaster.clone(),
            all_params.lib_path.clone(),
        )
    }
}

impl Signal for CreatePlaylistRequest {
    type Params = (
        PlaylistArgs,
        Arc<dyn crate::utils::Broadcaster>,
        Arc<String>,
    );
    type Response = CreatePlaylistResponse;
    async fn handle(
        &self,
        ((db, store, _actor), broadcaster, path): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let name = &request.name;
        let group = &request.group;

        let playlist = database::actions::playlists::create_playlist(
            &db,
            &store.context("playlist store unavailable")?,
            name.clone(),
            group.clone(),
        )
        .await
        .with_context(|| format!("Failed to create playlist: name={name}, group={group}"))?;

        broadcaster.broadcast(&LibraryContentChanged {
            path: path.to_string(),
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
    type Params = (
        PlaylistArgs,
        Arc<dyn crate::utils::Broadcaster>,
        Arc<String>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            playlist_args(all_params),
            all_params.broadcaster.clone(),
            all_params.lib_path.clone(),
        )
    }
}

impl Signal for UpdatePlaylistRequest {
    type Params = (
        PlaylistArgs,
        Arc<dyn crate::utils::Broadcaster>,
        Arc<String>,
    );
    type Response = UpdatePlaylistResponse;
    async fn handle(
        &self,
        ((db, store, _actor), broadcaster, path): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let name = &request.name;
        let group = &request.group;

        let playlist = database::actions::playlists::update_playlist(
            &db,
            &store.context("playlist store unavailable")?,
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
            path: path.to_string(),
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
    type Params = (
        PlaylistArgs,
        Arc<dyn crate::utils::Broadcaster>,
        Arc<String>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            playlist_args(all_params),
            all_params.broadcaster.clone(),
            all_params.lib_path.clone(),
        )
    }
}

impl Signal for RemovePlaylistRequest {
    type Params = (
        PlaylistArgs,
        Arc<dyn crate::utils::Broadcaster>,
        Arc<String>,
    );
    type Response = RemovePlaylistResponse;
    async fn handle(
        &self,
        ((db, store, _actor), broadcaster, path): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        database::actions::playlists::remove_playlist(
            &db,
            &store.context("playlist store unavailable")?,
            request.playlist_id,
        )
        .await
        .with_context(|| format!("Removing playlist: id={}", request.playlist_id))?;

        broadcaster.broadcast(&LibraryContentChanged {
            path: path.to_string(),
        });
        Ok(Some(RemovePlaylistResponse {
            playlist_id: request.playlist_id,
            success: true,
        }))
    }
}

impl ParamsExtractor for AddItemToPlaylistRequest {
    type Params = (
        PlaylistArgs,
        Arc<dyn crate::utils::Broadcaster>,
        Arc<String>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            playlist_args(all_params),
            all_params.broadcaster.clone(),
            all_params.lib_path.clone(),
        )
    }
}

impl Signal for AddItemToPlaylistRequest {
    type Params = (
        PlaylistArgs,
        Arc<dyn crate::utils::Broadcaster>,
        Arc<String>,
    );
    type Response = AddItemToPlaylistResponse;
    async fn handle(
        &self,
        ((db, store, _actor), broadcaster, path): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let entry = database::actions::playlists::add_item_to_playlist(
            &db,
            &store.context("playlist store unavailable")?,
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
            path: path.to_string(),
        });
        Ok(Some(AddItemToPlaylistResponse {
            success: true,
            occurrence_id: entry.hlc_uuid,
        }))
    }
}

impl ParamsExtractor for ReorderPlaylistItemPositionRequest {
    type Params = (
        PlaylistArgs,
        Arc<dyn crate::utils::Broadcaster>,
        Arc<String>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            playlist_args(all_params),
            all_params.broadcaster.clone(),
            all_params.lib_path.clone(),
        )
    }
}

impl Signal for ReorderPlaylistItemPositionRequest {
    type Params = (
        PlaylistArgs,
        Arc<dyn crate::utils::Broadcaster>,
        Arc<String>,
    );
    type Response = ReorderPlaylistItemPositionResponse;
    async fn handle(
        &self,
        ((db, store, _actor), broadcaster, path): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        database::actions::playlists::reorder_playlist_item_position(
            &db, &store.context("playlist store unavailable")?, request.playlist_id,
            request.occurrence_id.parse().context("invalid occurrence UUID")?,
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
            path: path.to_string(),
        });
        Ok(Some(ReorderPlaylistItemPositionResponse { success: true }))
    }
}

impl ParamsExtractor for GetPlaylistByIdRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for GetPlaylistByIdRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = GetPlaylistByIdResponse;
    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let playlist = get_playlist_by_id(&main_db, request.playlist_id)
            .await
            .with_context(|| format!("Failed to get playlist by id: {}", request.playlist_id))?
            .ok_or(anyhow!(
                "Playlist not found with id: {}",
                request.playlist_id
            ))?;

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
    type Params = (
        PlaylistArgs,
        Arc<dyn crate::utils::Broadcaster>,
        Arc<String>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            playlist_args(all_params),
            all_params.broadcaster.clone(),
            all_params.lib_path.clone(),
        )
    }
}

impl Signal for CreateM3u8PlaylistRequest {
    type Params = (
        PlaylistArgs,
        Arc<dyn crate::utils::Broadcaster>,
        Arc<String>,
    );
    type Response = CreateM3u8PlaylistResponse;
    async fn handle(
        &self,
        ((db, store, _actor), broadcaster, library_path): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let name = &request.name;
        let group = &request.group;
        let path = &request.path;

        match database::actions::playlists::create_m3u8_playlist(
            &db,
            &store.context("playlist store unavailable")?,
            name.clone(),
            group.clone(),
            Path::new(&path),
        )
        .await
        {
            Ok((playlist, import_result)) => {
                broadcaster.broadcast(&LibraryContentChanged {
                    path: library_path.to_string(),
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
    type Params = (
        PlaylistArgs,
        Arc<dyn crate::utils::Broadcaster>,
        Arc<String>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            playlist_args(all_params),
            all_params.broadcaster.clone(),
            all_params.lib_path.clone(),
        )
    }
}

impl Signal for RemoveItemFromPlaylistRequest {
    type Params = (
        PlaylistArgs,
        Arc<dyn crate::utils::Broadcaster>,
        Arc<String>,
    );
    type Response = RemoveItemFromPlaylistResponse;

    async fn handle(
        &self,
        ((db, store, _actor), broadcaster, path): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        match database::actions::playlists::remove_item_from_playlist(
            &db,
            &store.context("playlist store unavailable")?,
            request.playlist_id,
            request
                .occurrence_id
                .parse()
                .context("invalid occurrence UUID")?,
        )
        .await
        {
            Ok(_) => {
                broadcaster.broadcast(&LibraryContentChanged {
                    path: path.to_string(),
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
    type Params = (
        Arc<fsio::FsIo>,
        Arc<MainDbConnection>,
        Arc<String>,
        Arc<String>,
    );
    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        (
            params.fsio.clone(),
            params.main_db.clone(),
            params.lib_path.clone(),
            params.node_id.clone(),
        )
    }
}

impl Signal for FetchPlaylistOccurrencesRequest {
    type Params = (
        Arc<fsio::FsIo>,
        Arc<MainDbConnection>,
        Arc<String>,
        Arc<String>,
    );
    type Response = FetchPlaylistOccurrencesResponse;
    async fn handle(
        &self,
        (fsio, db, root, node): Self::Params,
        _session: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let page = database::actions::playlists::list_occurrences(
            &db,
            request.playlist_id,
            request
                .after
                .as_ref()
                .map(|cursor| database::actions::playlists::OccurrenceCursor {
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
        let summaries = database::actions::metadata::get_metadata_summary_by_files(
            db.as_ref(),
            source_files.clone(),
        )
        .await?;
        let files = crate::utils::parse_media_files(&fsio, &db, &node, summaries, root).await?;
        let files: std::collections::HashMap<_, _> =
            files.into_iter().map(|file| (file.id, file)).collect();
        let cover_art_map = if request.bake_cover_arts {
            database::actions::cover_art::bake_cover_art_by_media_files(&fsio, &db, source_files)
                .await?
        } else {
            std::collections::HashMap::new()
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
