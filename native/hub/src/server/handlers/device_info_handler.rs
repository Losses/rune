use std::sync::Arc;

use axum::{extract::State, Json};
use serde::Serialize;

use discovery::utils::DeviceInfo;

use crate::server::ServerState;

#[derive(Debug, Serialize)]
pub struct SanitizedDeviceInfo {
    alias: String,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_model: Option<String>,
    device_type: String,
}

fn sanitize_device_info(original: &DeviceInfo) -> SanitizedDeviceInfo {
    SanitizedDeviceInfo {
        alias: original.alias.clone(),
        version: original.version.clone(),
        device_model: original.device_model.clone(),
        device_type: match original.device_type {
            Some(x) => x.to_string(),
            None => "Unknown".to_string(),
        },
    }
}

pub async fn device_info_handler(
    State(state): State<Arc<ServerState>>,
) -> Json<SanitizedDeviceInfo> {
    let sanitized = sanitize_device_info(&state.discovery_device_info);
    Json(sanitized)
}
