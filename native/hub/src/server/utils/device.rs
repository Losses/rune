use std::path::Path;

use anyhow::Result;

use discovery::{
    capabilities::Capabilities,
    endpoint::DEFAULT_DEVICE_PORT,
    protocol_version::{PROTOCOL_VERSION, ProtocolVersion},
    utils::{DeviceInfo, DeviceType},
};
use serde::{Deserialize, Serialize};

use crate::server::{generate_or_load_certificates, get_or_generate_alias};

/// The version of the app, as the app's own manifest states it.
pub const APP_VERSION: &str = env!("RUNE_APP_VERSION");

/// How this process runs, which decides what kind of device it announces itself as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunMode {
    /// The app, on whatever platform it was built for.
    App,
    /// The headless server binary.
    Server,
}

impl RunMode {
    pub fn device_type(self) -> DeviceType {
        match self {
            Self::Server => DeviceType::Server,
            Self::App if cfg!(any(target_os = "android", target_os = "ios")) => DeviceType::Mobile,
            Self::App => DeviceType::Desktop,
        }
    }
}

/// The one place that describes this device to other devices, for announcements and for the
/// device information a peer fetches.
pub fn local_device_info(alias: String, fingerprint: String, mode: RunMode) -> DeviceInfo {
    DeviceInfo {
        alias,
        device_model: Some("RuneAudio".to_string()),
        version: APP_VERSION.to_owned(),
        device_type: Some(mode.device_type()),
        fingerprint,
        api_port: DEFAULT_DEVICE_PORT,
        protocol_version: Some(PROTOCOL_VERSION),
        capabilities: None,
    }
}

pub async fn load_device_info(config_path: &Path, mode: RunMode) -> Result<DeviceInfo> {
    let certificate_id = get_or_generate_alias(config_path).await?;
    let (fingerprint, _, _) = generate_or_load_certificates(config_path, &certificate_id).await?;

    Ok(local_device_info(certificate_id, fingerprint, mode))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SanitizedDeviceInfo {
    pub alias: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    pub device_type: DeviceType,
    pub protocol_version: Option<ProtocolVersion>,
    pub capabilities: Option<Capabilities>,
}
