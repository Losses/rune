use utils::DeviceInfo;

pub mod permission;
pub mod ssl;
pub mod udp_multicast;
pub mod utils;
pub mod verifier;

pub struct DiscoveryParams {
    pub device_info: DeviceInfo,
}
