use utils::DeviceInfo;

pub mod http_api;
pub mod udp_multicast;
pub mod utils;

pub struct DiscoveryParams {
    pub device_info: DeviceInfo,
}
