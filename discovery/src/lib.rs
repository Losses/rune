use utils::DeviceInfo;

pub mod permission;
pub mod udp_multicast;
pub mod utils;

pub struct DiscoveryParams {
    pub device_info: DeviceInfo,
}
