use std::{fmt, str::FromStr};

use chrono::{DateTime, Utc};
use rustls::{
    Error as RustlsError,
    pki_types::{IpAddr, ServerName},
};
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

pub fn ip_to_string(x: &IpAddr) -> String {
    match x {
        rustls::pki_types::IpAddr::V4(ipv4_addr) => {
            let octets = ipv4_addr.as_ref();
            format!("{}.{}.{}.{}", octets[0], octets[1], octets[2], octets[3])
        }

        rustls::pki_types::IpAddr::V6(ipv6_addr) => {
            let octets = ipv6_addr.as_ref();
            // Convert octets to 16-bit segments
            let segments = [
                ((octets[0] as u16) << 8) | (octets[1] as u16),
                ((octets[2] as u16) << 8) | (octets[3] as u16),
                ((octets[4] as u16) << 8) | (octets[5] as u16),
                ((octets[6] as u16) << 8) | (octets[7] as u16),
                ((octets[8] as u16) << 8) | (octets[9] as u16),
                ((octets[10] as u16) << 8) | (octets[11] as u16),
                ((octets[12] as u16) << 8) | (octets[13] as u16),
                ((octets[14] as u16) << 8) | (octets[15] as u16),
            ];

            // Find longest run of zeros for :: compression
            let mut longest_zero_run = 0;
            let mut longest_zero_start = 0;
            let mut current_zero_run = 0;
            let mut current_zero_start = 0;

            for (i, &segment) in segments.iter().enumerate() {
                if segment == 0 {
                    if current_zero_run == 0 {
                        current_zero_start = i;
                    }
                    current_zero_run += 1;
                    if current_zero_run > longest_zero_run {
                        longest_zero_run = current_zero_run;
                        longest_zero_start = current_zero_start;
                    }
                } else {
                    current_zero_run = 0;
                }
            }

            // Build the string
            let mut result = String::with_capacity(39); // Max IPv6 string length
            let mut i = 0;
            let mut first = true;

            while i < 8 {
                if longest_zero_run > 1 && i == longest_zero_start {
                    if first {
                        result.push_str("::");
                    } else {
                        result.push(':');
                    }
                    i += longest_zero_run;
                    first = false;
                    continue;
                }

                if !first {
                    result.push(':');
                }
                result.push_str(&format!("{:x}", segments[i]));
                first = false;
                i += 1;
            }

            result
        }
    }
}

pub fn server_name_to_string(x: &ServerName<'_>) -> Result<String, RustlsError> {
    let result = match x {
        ServerName::DnsName(dns) => dns.as_ref().to_string(),
        ServerName::IpAddress(ip) => ip_to_string(ip),
        _ => return Err(RustlsError::General("Oh! Invalid server name".into())),
    };

    Ok(result)
}
