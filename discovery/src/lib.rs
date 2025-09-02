use utils::DeviceInfo;

pub mod client;
pub mod persistent;
pub mod protocol;
pub mod server;
pub mod ssl;
pub mod url;
pub mod utils;
pub mod config;

pub struct DiscoveryParams {
    pub device_info: DeviceInfo,
}
