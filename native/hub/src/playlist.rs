use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use database::actions::playlists::create_m3u8_playlist;
use rinf::DartSignal;

use database::actions::playlists::add_item_to_playlist;
use database::actions::playlists::create_playlist;
use database::actions::playlists::get_all_playlists;
use database::actions::playlists::get_playlist_by_id;
use database::actions::playlists::remove_playlist;
use database::actions::playlists::reorder_playlist_item_position;
use database::actions::playlists::update_playlist;
use database::connection::MainDbConnection;
use sea_orm::TransactionTrait;

use crate::CreateM3u8PlaylistRequest;
use crate::CreateM3u8PlaylistResponse;
use crate::RemovePlaylistRequest;
use crate::RemovePlaylistResponse;
use crate::{
    AddItemToPlaylistRequest, AddItemToPlaylistResponse, CreatePlaylistRequest,
    CreatePlaylistResponse, FetchAllPlaylistsRequest, FetchAllPlaylistsResponse,
    GetPlaylistByIdRequest, GetPlaylistByIdResponse, Playlist, ReorderPlaylistItemPositionRequest,
    ReorderPlaylistItemPositionResponse, UpdatePlaylistRequest, UpdatePlaylistResponse,
};

pub async fn fetch_all_playlists_request(
    main_db: Arc<MainDbConnection>,
    _dart_signal: DartSignal<FetchAllPlaylistsRequest>,
) -> Result<()> {
    let playlists = get_all_playlists(&main_db)
        .await
        .with_context(|| "Failed to fetch all playlists")?;

    FetchAllPlaylistsResponse {
        playlists: playlists
            .into_iter()
            .map(|playlist| Playlist {
                id: playlist.id,
                name: playlist.name,
                group: playlist.group,
            })
            .collect(),
    }
    .send_signal_to_dart();

    Ok(())
}

pub async fn create_playlist_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<CreatePlaylistRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    let name = request.name;
    let group = request.group;

    let txn = main_db.begin().await?;
    let playlist = create_playlist(&txn, name.clone(), group.clone())
        .await
        .with_context(|| format!("Failed to create playlist: name={}, group={}", name, group))?;
    txn.commit().await?;

    CreatePlaylistResponse {
        playlist: Some(Playlist {
            id: playlist.id,
            name: playlist.name,
            group: playlist.group,
        }),
    }
    .send_signal_to_dart();

    Ok(())
}

pub async fn update_playlist_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<UpdatePlaylistRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    let name = request.name;
    let group = request.group;

    let playlist = update_playlist(
        &main_db,
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

    UpdatePlaylistResponse {
        playlist: Some(Playlist {
            id: playlist.id,
            name: playlist.name,
            group: playlist.group,
        }),
    }
    .send_signal_to_dart();

    Ok(())
}

pub async fn remove_playlist_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<RemovePlaylistRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    remove_playlist(&main_db, request.playlist_id)
        .await
        .with_context(|| format!("Removing playlist: id={}", request.playlist_id))?;

    RemovePlaylistResponse {
        playlist_id: request.playlist_id,
        success: true,
    }
    .send_signal_to_dart();

    Ok(())
}

pub async fn add_item_to_playlist_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<AddItemToPlaylistRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    add_item_to_playlist(
        &main_db,
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

    AddItemToPlaylistResponse { success: true }.send_signal_to_dart();

    Ok(())
}

pub async fn reorder_playlist_item_position_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<ReorderPlaylistItemPositionRequest>,
) -> Result<()> {
    let request = dart_signal.message;

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

    ReorderPlaylistItemPositionResponse { success: true }.send_signal_to_dart();

    Ok(())
}

pub async fn get_playlist_by_id_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<GetPlaylistByIdRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    let playlist = get_playlist_by_id(&main_db, request.playlist_id)
        .await
        .with_context(|| format!("Failed to get playlist by id: {}", request.playlist_id))?
        .ok_or(anyhow!(
            "Playlist not found with id: {}",
            request.playlist_id
        ))?;

    GetPlaylistByIdResponse {
        playlist: Some(Playlist {
            id: playlist.id,
            name: playlist.name,
            group: playlist.group,
        }),
    }
    .send_signal_to_dart();

    Ok(())
}

pub async fn create_m3u8_playlist_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<CreateM3u8PlaylistRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    let name = request.name;
    let group = request.group;
    let path = request.path;

    match create_m3u8_playlist(&main_db, name.clone(), group.clone(), Path::new(&path)).await {
        Ok((playlist, import_result)) => {
            // On success, construct the response with playlist details and import results
            CreateM3u8PlaylistResponse {
                playlist: Some(Playlist {
                    id: playlist.id,
                    name: playlist.name,
                    group: playlist.group,
                }),
                imported_count: Some(import_result.matched_ids.len() as i32),
                not_found_paths: import_result.unmatched_paths,
                success: true,
                error: String::new(), // No error on success
            }
            .send_signal_to_dart();
        }
        Err(e) => {
            // On error, construct the response with the error message
            CreateM3u8PlaylistResponse {
                playlist: None,
                imported_count: Some(0),
                not_found_paths: vec![],
                success: false,
                error: e.to_string(), // Capture the error message
            }
            .send_signal_to_dart();
        }
    }

    Ok(())
}
