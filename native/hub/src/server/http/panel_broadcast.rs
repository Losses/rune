use std::{sync::Arc, time::Duration};

use axum::{Json, extract::State, http::StatusCode};
use serde::Deserialize;

use crate::server::ServerState;

use super::register::AppError;

#[derive(Deserialize)]
pub struct ToggleBroadcastRequest {
    enabled: bool,
}

pub async fn toggle_broadcast_handler(
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<ToggleBroadcastRequest>,
) -> Result<StatusCode, AppError> {
    let device_info = state.discovery_device_info.clone();

    if payload.enabled {
        state
            .device_scanner
            .start_announcements(
                device_info.read().await.clone(),
                Duration::from_secs(3),
                None,
            )
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
    } else {
        state.device_scanner.stop_announcements().await;
    }

    Ok(StatusCode::NO_CONTENT)
}
