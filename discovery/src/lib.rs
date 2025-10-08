use utils::DeviceInfo;

pub mod client;
pub mod config;
pub mod persistent;
pub mod protocol;
pub mod server;
pub mod ssl;
pub mod url;
pub mod utils;

pub struct DiscoveryParams {
    pub device_info: DeviceInfo,
}
