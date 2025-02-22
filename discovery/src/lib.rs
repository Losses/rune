use utils::DeviceInfo;

pub mod server;
pub mod persistent;
pub mod request;
pub mod ssl;
pub mod protocol;
pub mod utils;
pub mod client;

pub struct DiscoveryParams {
    pub device_info: DeviceInfo,
}
