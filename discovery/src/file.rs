use std::collections::HashMap;

use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use tokio::{fs::File, io::AsyncWriteExt};

use crate::http_api::DiscoveryState;

#[derive(Debug)]
pub enum AppError {
    IoError(std::io::Error),
    ValidationError(&'static str),
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::IoError(err)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::IoError(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Internal server error: {}", err),
            ),
            AppError::ValidationError(msg) => (StatusCode::BAD_REQUEST, msg.to_string()),
        };

        (status, message).into_response()
    }
}

pub async fn upload<S: DiscoveryState>(
    State(state): State<S>,
    Query(params): Query<HashMap<String, String>>,
    body: axum::body::Bytes,
) -> Result<(), AppError> {
    let session_id = params
        .get("sessionId")
        .ok_or(AppError::ValidationError("Missing sessionId"))?;
    let file_id = params
        .get("fileId")
        .ok_or(AppError::ValidationError("Missing fileId"))?;
    let token = params
        .get("token")
        .ok_or(AppError::ValidationError("Missing token"))?;

    let active_sessions = state.active_sessions();
    let sessions = active_sessions.read().await;
    let session = sessions
        .get(session_id)
        .ok_or(AppError::ValidationError("Invalid session"))?;

    if session.tokens.get(file_id) != Some(token) {
        return Err(AppError::ValidationError("Invalid token"));
    }

    let file_meta = session
        .files
        .get(file_id)
        .ok_or(AppError::ValidationError("Invalid file"))?;
    let path = format!("downloads/{}", file_meta.file_name);

    let mut file = File::create(path).await?;
    file.write_all(&body).await?;

    Ok(())
}

pub async fn download<S: DiscoveryState>(
    State(state): State<S>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let result: Result<Vec<u8>, AppError> = async {
        let session_id = params
            .get("sessionId")
            .ok_or(AppError::ValidationError("Missing sessionId"))?;
        let file_id = params
            .get("fileId")
            .ok_or(AppError::ValidationError("Missing fileId"))?;

        let binding = state.active_sessions();
        let sessions = binding.read().await;
        let session = sessions
            .get(session_id)
            .ok_or(AppError::ValidationError("Invalid session"))?;
        let file_meta = session
            .files
            .get(file_id)
            .ok_or(AppError::ValidationError("Invalid file"))?;

        let path = format!("uploads/{}", file_meta.file_name);
        let contents = tokio::fs::read(path).await?;

        Ok(contents)
    }
    .await;

    match result {
        Ok(contents) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(contents))
            .unwrap()
            .into_response(),
        Err(err) => err.into_response(),
    }
}
