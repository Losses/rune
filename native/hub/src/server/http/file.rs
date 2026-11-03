use std::{path::Path as FilePath, sync::Arc};

use axum::{
    body::Body,
    extract::{Extension, Path, Query, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use tower::ServiceExt;
use tower_http::services::ServeDir;

use crate::server::{
    ServerManager, ServerState,
    libraries::{LibraryQuery, request_library},
};
use futures::StreamExt;

pub async fn file_handler(
    Path(file_path): Path<String>,
    State(state): State<Arc<ServerState>>,
    Extension(manager): Extension<Arc<ServerManager>>,
    Query(query): Query<LibraryQuery>,
) -> impl IntoResponse {
    let cover_temp_dir = &state.app_state.cover_temp_dir;

    // Parse the request path, splitting it into prefix and actual file path
    let path_parts: Vec<&str> = file_path.splitn(2, '/').collect();
    if path_parts.len() != 2 {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let (prefix, relative_path) = (path_parts[0], path_parts[1]);

    // Determine the actual root directory based on the prefix
    let library = if prefix == "library" {
        match request_library(&manager, query).await {
            Ok(library) => Some(library),
            Err(status) => return status.into_response(),
        }
    } else {
        None
    };
    let root_dir = match prefix {
        "library" => match library.as_ref() {
            Some(library) => FilePath::new(&library.media_root),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
        "cache" => cover_temp_dir.as_path(),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };

    // Construct the full file path and normalize it
    let requested_path = root_dir.join(relative_path);
    let canonical_path = match library.as_ref() {
        Some(library) => library.fsio.canonicalize_path(&requested_path).ok(),
        None => requested_path.canonicalize().ok(),
    };
    let Some(canonical_path) = canonical_path else {
        return StatusCode::FORBIDDEN.into_response();
    };

    // Security check: Ensure the accessed path does not go beyond the specified directory
    if !canonical_path.starts_with(root_dir) {
        return StatusCode::FORBIDDEN.into_response();
    }

    // Get the relative path
    let relative_path = match canonical_path.strip_prefix(root_dir) {
        Ok(path) => path,
        Err(_) => return StatusCode::FORBIDDEN.into_response(),
    };

    // Serve the file using ServeDir
    let service = ServeDir::new(root_dir);
    let request = match Request::builder()
        .uri(format!("/{}", relative_path.to_string_lossy()))
        .body(Body::empty())
    {
        Ok(request) => request,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    match service.oneshot(request).await {
        Ok(response) => {
            let (parts, body) = response.into_parts();
            let boxed_body =
                Body::from_stream(Body::new(body).into_data_stream().map(move |chunk| {
                    let _lease = &library;
                    chunk
                }));
            Response::from_parts(parts, boxed_body)
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
