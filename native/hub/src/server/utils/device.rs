use std::path::Path;

use anyhow::Result;

use discovery::utils::{DeviceInfo, DeviceType};
use serde::{Deserialize, Serialize};

use crate::server::{generate_or_load_certificates, get_or_generate_certificate_id};

pub async fn load_device_info(config_path: &Path) -> Result<DeviceInfo> {
    let certificate_id = get_or_generate_certificate_id(config_path).await?;
    let (fingerprint, _, _) = generate_or_load_certificates(config_path, &certificate_id).await?;

    Ok(DeviceInfo {
        alias: certificate_id.clone(),
        device_model: Some("RuneAudio".to_string()),
        version: "Technical Preview".to_owned(),
        device_type: Some(DeviceType::Desktop),
        fingerprint: fingerprint.clone(),
        api_port: 7863,
        protocol: "http".to_owned(),
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SanitizedDeviceInfo {
    pub alias: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    pub device_type: String,
}
