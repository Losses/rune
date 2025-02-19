use utils::DeviceInfo;

pub mod discovery_runtime;
pub mod permission;
pub mod persistent;
pub mod request;
pub mod ssl;
pub mod udp_multicast;
pub mod utils;
pub mod verifier;

pub struct DiscoveryParams {
    pub device_info: DeviceInfo,
}
