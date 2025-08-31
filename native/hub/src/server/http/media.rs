use std::sync::Arc;

use axum::{
    Json,
    extract::{Extension, Path as AxumPath, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use mimetype_detector::detect_file;
use serde::Serialize;

use crate::{
    messages::{Album, Artist, MediaFile},
    server::{ServerManager, ServerState},
    utils::parse_media_files,
};
use database::actions::{cover_art::bake_cover_art_by_file_ids, metadata::get_parsed_file_by_id};

#[derive(Serialize)]
pub struct MediaMetadataResponse {
    pub file: MediaFile,
    pub artists: Vec<Artist>,
    pub album: Album,
}

pub async fn get_media_metadata_handler(
    State(server_state): State<Arc<ServerState>>,
    Extension(server_manager): Extension<Arc<ServerManager>>,
    AxumPath(file_id): AxumPath<i64>,
) -> Result<Json<MediaMetadataResponse>, (StatusCode, String)> {
    let file_id_i32 = file_id
        .try_into()
        .map_err(|_| (StatusCode::BAD_REQUEST, "File ID out of range".to_string()))?;
    let (media_file, artists, album) =
        get_parsed_file_by_id(&server_manager.global_params.main_db, file_id_i32)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let lib_path_string = server_state
        .app_state
        .lib_path
        .to_string_lossy()
        .to_string();

    let parsed_files = parse_media_files(
        &server_state.fsio,
        vec![media_file],
        Arc::new(lib_path_string),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let media_file = parsed_files.first().cloned().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Parsed Files not found for file_id: {file_id}"),
        )
    })?;

    let album = album.ok_or((
        StatusCode::NOT_FOUND,
        format!("Parsed album not found for file_id: {file_id}"),
    ))?;

    Ok(Json(MediaMetadataResponse {
        file: media_file,
        artists: artists
            .into_iter()
            .map(|x| Artist {
                id: x.id,
                name: x.name,
            })
            .collect(),
        album: Album {
            id: album.id,
            name: album.name,
        },
    }))
}

pub async fn get_cover_art_handler(
    State(server_state): State<Arc<ServerState>>,
    Extension(server_manager): Extension<Arc<ServerManager>>,
    AxumPath(file_id): AxumPath<i64>,
) -> Result<Response, (StatusCode, String)> {
    let file_id_i32 = file_id
        .try_into()
        .map_err(|_| (StatusCode::BAD_REQUEST, "File ID out of range".to_string()))?;
    let cover_art_map = bake_cover_art_by_file_ids(
        &server_state.fsio,
        &server_manager.global_params.main_db,
        vec![file_id_i32],
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let cover_path = cover_art_map
        .get(&file_id_i32)
        .cloned()
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Cover art not found".to_string()))?;

    let cover_data = match tokio::fs::read(&cover_path).await {
        Ok(data) => data,
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read cover art file: {e}"),
            ));
        }
    };

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        detect_file(&cover_path)
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to parse mime type: {e}"),
                )
            })?
            .to_string()
            .parse()
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to parse mime resovling result: {e}"),
                )
            })?,
    );
    Ok((headers, cover_data).into_response())
}
