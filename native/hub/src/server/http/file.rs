use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use tower::ServiceExt;
use tower_http::services::ServeDir;

use crate::server::ServerState;

pub async fn file_handler(
    Path(file_path): Path<String>,
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    let lib_path = &state.app_state.lib_path;
    let cover_temp_dir = &state.app_state.cover_temp_dir;
    let fsio = state.fsio.clone();

    // Parse the request path, splitting it into prefix and actual file path
    let path_parts: Vec<&str> = file_path.splitn(2, '/').collect();
    if path_parts.len() != 2 {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let (prefix, relative_path) = (path_parts[0], path_parts[1]);

    // Determine the actual root directory based on the prefix
    let root_dir = match prefix {
        "library" => lib_path,
        "cache" => cover_temp_dir,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };

    // Construct the full file path and normalize it
    let requested_path = root_dir.join(relative_path);
    let canonical_path = match fsio.canonicalize_path(&requested_path) {
        Ok(path) => path,
        Err(_) => return StatusCode::FORBIDDEN.into_response(),
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
    let request = Request::builder()
        .uri(format!("/{}", relative_path.to_string_lossy()))
        .body(axum::body::Body::empty())
        .unwrap();

    match service.oneshot(request).await {
        Ok(response) => {
            let (parts, body) = response.into_parts();
            let boxed_body = Body::new(body);
            Response::from_parts(parts, boxed_body)
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
