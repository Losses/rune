use std::sync::Arc;

use http_api::FileProvider;
use utils::DeviceInfo;

pub mod http_api;
pub mod pin;
pub mod udp_multicast;
pub mod utils;

pub struct DiscoveryParams {
    pub device_info: DeviceInfo,
    pub pin: Option<String>,
    pub file_provider: Arc<dyn FileProvider>,
}
