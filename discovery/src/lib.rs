use utils::DeviceInfo;

pub mod access_policy;
pub mod capabilities;
pub mod client;
pub mod config;
pub mod endpoint;
pub mod persistent;
pub mod protocol;
pub mod protocol_version;
pub mod server;
pub mod ssl;
pub mod utils;

pub struct DiscoveryParams {
    pub device_info: DeviceInfo,
}
