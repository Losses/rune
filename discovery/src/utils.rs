use std::{fmt, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Serialize, Deserialize, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DeviceType {
    Mobile,
    Desktop,
    Web,
    Headless,
    Server,
    Unknown,
}

impl fmt::Display for DeviceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                DeviceType::Mobile => "Mobile",
                DeviceType::Desktop => "Desktop",
                DeviceType::Web => "Web",
                DeviceType::Headless => "Headless",
                DeviceType::Server => "Server",
                DeviceType::Unknown => "Unknown",
            }
        )
    }
}

#[derive(Debug)]
pub struct ParseDeviceTypeError;

impl fmt::Display for ParseDeviceTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to parse device type")
    }
}

impl std::error::Error for ParseDeviceTypeError {}

impl FromStr for DeviceType {
    type Err = ParseDeviceTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "mobile" => Ok(DeviceType::Mobile),
            "desktop" => Ok(DeviceType::Desktop),
            "web" => Ok(DeviceType::Web),
            "headless" => Ok(DeviceType::Headless),
            "server" => Ok(DeviceType::Server),
            _ => Ok(DeviceType::Unknown),
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
