use std::{
    path::{Component, Path as FilePath, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use axum::{
    body::Body,
    extract::{Extension, Path, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use database::error::ExpectedError;
use fsio::FsIo;
use tower::ServiceExt;
use tower_http::services::ServeDir;

use crate::{
    Session,
    server::{ServerState, access::AuthorizedLibrary},
};
use futures::StreamExt;

/// Canonicalize a requested media path and require it to remain within the library root.
pub fn canonicalize_media_path(
    fsio: &FsIo,
    media_root: &FilePath,
    requested: &FilePath,
) -> Result<PathBuf> {
    if requested
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(invalid_media_path().into());
    }

    let root = fsio
        .canonicalize_path(media_root)
        .map_err(|_| invalid_media_path())?;
    let requested_path = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        media_root.join(requested)
    };
    let canonical = fsio
        .canonicalize_path(&requested_path)
        .map_err(|_| invalid_media_path())?;
    if !canonical.starts_with(&root) {
        return Err(invalid_media_path().into());
    }
    Ok(canonical)
}

pub fn validate_remote_media_path(
    session: Option<&Session>,
    fsio: &FsIo,
    media_root: &FilePath,
    requested: &FilePath,
) -> Result<()> {
    if session.is_some() {
        canonicalize_media_path(fsio, media_root, requested)?;
    }
    Ok(())
}

fn invalid_media_path() -> ExpectedError {
    ExpectedError::InvalidInput {
        detail: "path is outside the library media root or does not exist".into(),
    }
}

pub async fn file_handler(
    Path(file_path): Path<String>,
    State(state): State<Arc<ServerState>>,
    Extension(AuthorizedLibrary(authorized_library)): Extension<AuthorizedLibrary>,
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
        Some(authorized_library)
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
    let canonical_root = match library.as_ref() {
        Some(library) => library.fsio.canonicalize_path(root_dir).ok(),
        None => root_dir.canonicalize().ok(),
    };
    let Some(canonical_root) = canonical_root else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let canonical_path = match library.as_ref() {
        Some(library) => canonicalize_media_path(
            &library.fsio,
            FilePath::new(&library.media_root),
            FilePath::new(relative_path),
        )
        .ok(),
        None => requested_path.canonicalize().ok(),
    };
    let Some(canonical_path) = canonical_path else {
        return StatusCode::FORBIDDEN.into_response();
    };

    // Security check: Ensure the accessed path does not go beyond the specified directory
    if library.is_none() && !canonical_path.starts_with(&canonical_root) {
        return StatusCode::FORBIDDEN.into_response();
    }

    // Get the relative path
    let relative_path = match canonical_path.strip_prefix(&canonical_root) {
        Ok(path) => path,
        Err(_) => return StatusCode::FORBIDDEN.into_response(),
    };

    // Serve the file using ServeDir
    let service = ServeDir::new(&canonical_root);
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
