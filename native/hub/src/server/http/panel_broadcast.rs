use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use serde::Deserialize;

use crate::{handlers::sync::update_sync_settings, server::ServerState};

use super::register::AppError;

#[derive(Deserialize)]
pub struct ToggleBroadcastRequest {
    enabled: bool,
}

pub async fn toggle_broadcast_handler(
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<ToggleBroadcastRequest>,
) -> Result<StatusCode, AppError> {
    let node = state
        .node
        .upgrade()
        .ok_or_else(|| AppError::Internal("node has shut down".to_owned()))?;
    update_sync_settings(&node, |settings| settings.broadcast = payload.enabled)
        .await
        .map_err(|error| AppError::Internal(format!("{error:#}")))?;

    Ok(StatusCode::NO_CONTENT)
}
