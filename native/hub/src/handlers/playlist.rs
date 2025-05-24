use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use database::actions::playlists::remove_item_from_playlist;
use sea_orm::TransactionTrait;

use ::database::actions::playlists::{
    add_item_to_playlist, create_m3u8_playlist, create_playlist, get_all_playlists,
    get_playlist_by_id, remove_playlist, reorder_playlist_item_position, update_playlist,
};
use ::database::connection::MainDbConnection;

use crate::utils::{GlobalParams, ParamsExtractor};
use crate::{messages::*, Session, Signal};

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
    type Params = (Arc<MainDbConnection>, Arc<String>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.node_id),
        )
    }
}

impl Signal for CreatePlaylistRequest {
    type Params = (Arc<MainDbConnection>, Arc<String>);
    type Response = CreatePlaylistResponse;
    async fn handle(
        &self,
        (main_db, node_id): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let name = &request.name;
        let group = &request.group;

        let txn = main_db.begin().await?;
        let playlist = create_playlist(&txn, &node_id, name.clone(), group.clone())
            .await
            .with_context(|| {
                format!("Failed to create playlist: name={}, group={}", name, group)
            })?;
        txn.commit().await?;

        Ok(Some(CreatePlaylistResponse {
            playlist: Some(Playlist {
                id: playlist.id,
                name: playlist.name,
                group: playlist.group,
            }),
        }))
    }
}

impl ParamsExtractor for UpdatePlaylistRequest {
    type Params = (Arc<MainDbConnection>, Arc<String>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.node_id),
        )
    }
}

impl Signal for UpdatePlaylistRequest {
    type Params = (Arc<MainDbConnection>, Arc<String>);
    type Response = UpdatePlaylistResponse;
    async fn handle(
        &self,
        (main_db, node_id): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let name = &request.name;
        let group = &request.group;

        let playlist = update_playlist(
            &main_db,
            &node_id,
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

        Ok(Some(UpdatePlaylistResponse {
            playlist: Some(Playlist {
                id: playlist.id,
                name: playlist.name,
                group: playlist.group,
            }),
        }))
    }
}

impl ParamsExtractor for RemovePlaylistRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for RemovePlaylistRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = RemovePlaylistResponse;
    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        remove_playlist(&main_db, request.playlist_id)
            .await
            .with_context(|| format!("Removing playlist: id={}", request.playlist_id))?;

        Ok(Some(RemovePlaylistResponse {
            playlist_id: request.playlist_id,
            success: true,
        }))
    }
}

impl ParamsExtractor for AddItemToPlaylistRequest {
    type Params = (Arc<MainDbConnection>, Arc<String>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.node_id),
        )
    }
}

impl Signal for AddItemToPlaylistRequest {
    type Params = (Arc<MainDbConnection>, Arc<String>);
    type Response = AddItemToPlaylistResponse;
    async fn handle(
        &self,
        (main_db, node_id): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        add_item_to_playlist(
            &main_db,
            &node_id,
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

        Ok(Some(AddItemToPlaylistResponse { success: true }))
    }
}

impl ParamsExtractor for ReorderPlaylistItemPositionRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for ReorderPlaylistItemPositionRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = ReorderPlaylistItemPositionResponse;
    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        reorder_playlist_item_position(
            &main_db,
            request.playlist_id,
            request.media_file_id,
            request.new_position,
        )
        .await
        .with_context(|| {
            format!(
                "Failed to reorder playlist item: playlist_id={}, media_file_id={}, new_position={}",
                request.playlist_id, request.media_file_id, request.new_position
            )
        })?;

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
            playlist: Some(Playlist {
                id: playlist.id,
                name: playlist.name,
                group: playlist.group,
            }),
        }))
    }
}

impl ParamsExtractor for CreateM3u8PlaylistRequest {
    type Params = (Arc<MainDbConnection>, Arc<String>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.node_id),
        )
    }
}

impl Signal for CreateM3u8PlaylistRequest {
    type Params = (Arc<MainDbConnection>, Arc<String>);
    type Response = CreateM3u8PlaylistResponse;
    async fn handle(
        &self,
        (main_db, node_id): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let name = &request.name;
        let group = &request.group;
        let path = &request.path;

        match create_m3u8_playlist(
            &main_db,
            &node_id,
            name.clone(),
            group.clone(),
            Path::new(&path),
        )
        .await
        {
            Ok((playlist, import_result)) => Ok(Some(CreateM3u8PlaylistResponse {
                playlist: Some(Playlist {
                    id: playlist.id,
                    name: playlist.name,
                    group: playlist.group,
                }),
                imported_count: Some(import_result.matched_ids.len() as i32),
                not_found_paths: import_result.unmatched_paths,
                success: true,
                error: String::new(),
            })),
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
    type Params = (Arc<MainDbConnection>, Arc<String>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.node_id),
        )
    }
}

impl Signal for RemoveItemFromPlaylistRequest {
    type Params = (Arc<MainDbConnection>, Arc<String>);
    type Response = RemoveItemFromPlaylistResponse;

    async fn handle(
        &self,
        (main_db, node_id): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        match remove_item_from_playlist(
            &main_db,
            &node_id,
            request.playlist_id,
            request.media_file_id,
            request.position,
        )
        .await
        {
            Ok(_) => Ok(Some(RemoveItemFromPlaylistResponse {
                success: true,
                error: String::new(),
            })),
            Err(e) => Ok(Some(RemoveItemFromPlaylistResponse {
                success: false,
                error: e.to_string(),
            })),
        }
    }
}
