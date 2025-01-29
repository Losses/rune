use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub enum DeviceType {
    Mobile,
    Desktop,
    Web,
    Headless,
    Server,
}

impl fmt::Display for DeviceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceType::Mobile => write!(f, "Mobile"),
            DeviceType::Desktop => write!(f, "Desktop"),
            DeviceType::Web => write!(f, "Web"),
            DeviceType::Headless => write!(f, "Headless"),
            DeviceType::Server => write!(f, "Server"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub alias: String,
    pub version: String,
    pub device_model: Option<String>,
    pub device_type: Option<DeviceType>,
    pub fingerprint: String,
    pub api_port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadata {
    pub file_name: String,
    pub size: u64,
    pub file_type: String,
    pub metadata: Option<FileExtraMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileExtraMetadata {
    pub modified: Option<DateTime<Utc>>,
    pub accessed: Option<DateTime<Utc>>,
}
