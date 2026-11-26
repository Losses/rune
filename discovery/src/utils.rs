use std::fmt;

use chrono::{DateTime, Utc};
use rustls::{Error as RustlsError, pki_types::ServerName};
use serde::{Deserialize, Serialize};

use crate::capabilities::Capabilities;
use crate::protocol_version::ProtocolVersion;

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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub alias: String,
    pub version: String,
    pub device_model: Option<String>,
    pub device_type: Option<DeviceType>,
    pub fingerprint: String,
    pub api_port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<ProtocolVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Capabilities>,
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

pub fn server_name_to_string(x: &ServerName<'_>) -> Result<String, RustlsError> {
    let result = match x {
        ServerName::DnsName(dns) => dns.as_ref().to_string(),
        ServerName::IpAddress(ip) => std::net::IpAddr::from(*ip).to_string(),
        _ => return Err(RustlsError::General("Oh! Invalid server name".into())),
    };

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::server_name_to_string;
    use rustls::pki_types::ServerName;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn ip_server_name(ip: std::net::IpAddr) -> ServerName<'static> {
        ServerName::from(ip)
    }

    #[test]
    fn ipv4_server_names_convert_to_their_literal() {
        let name = ip_server_name(Ipv4Addr::new(192, 168, 1, 5).into());
        assert_eq!(server_name_to_string(&name).unwrap(), "192.168.1.5");
    }

    #[test]
    fn ipv6_server_names_convert_to_their_compressed_literal() {
        let cases = [
            ("::1", Ipv6Addr::LOCALHOST),
            ("2001:db8::", "2001:db8::".parse().unwrap()),
            ("2001:db8::ff00:42", "2001:db8::ff00:42".parse().unwrap()),
            ("::", Ipv6Addr::UNSPECIFIED),
        ];
        for (expected, address) in cases {
            let name = ip_server_name(address.into());
            assert_eq!(server_name_to_string(&name).unwrap(), expected);
        }
    }
}
