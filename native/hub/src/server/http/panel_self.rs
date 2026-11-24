use std::sync::Arc;

use axum::{Json, extract::State};
use serde::Serialize;

use crate::server::ServerState;

use super::register::AppError;

#[derive(Serialize)]
pub struct DeviceInfoResponse {
    fingerprint: String,
    alias: String,
    broadcasting: bool,
}

pub async fn self_handler(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<DeviceInfoResponse>, AppError> {
    let device_info = state.discovery_device_info.read().await;

    Ok(Json(DeviceInfoResponse {
        fingerprint: device_info.fingerprint.clone(),
        alias: device_info.alias.clone(),
        broadcasting: state.device_scanner.is_announcing().await,
    }))
}
