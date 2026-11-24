use std::sync::Arc;

use axum::{Json, extract::State};

use crate::server::{ServerState, utils::device::SanitizedDeviceInfo};
use discovery::{
    capabilities::Capabilities,
    utils::{DeviceInfo, DeviceType},
};

fn sanitize_device_info(original: &DeviceInfo) -> SanitizedDeviceInfo {
    SanitizedDeviceInfo {
        alias: original.alias.clone(),
        version: original.version.clone(),
        device_model: original.device_model.clone(),
        device_type: original.device_type.unwrap_or(DeviceType::Unknown),
        protocol_version: original.protocol_version,
        capabilities: Some(Capabilities::rune_native()),
    }
}

pub async fn device_info_handler(
    State(state): State<Arc<ServerState>>,
) -> Json<SanitizedDeviceInfo> {
    let sanitized = sanitize_device_info(&state.discovery_device_info.read().await.clone());
    Json(sanitized)
}
