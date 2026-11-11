use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Serialize, Deserialize, SignalPiece)]
pub enum DevicePreset {
    MyDevice,
    Guest,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, SignalPiece)]
pub enum DeviceConnectionLevel {
    #[default]
    None,
    View,
    ViewEdit,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, SignalPiece)]
pub enum DeviceSyncLevel {
    #[default]
    None,
    Pull,
    PullPush,
}
#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct DeviceLibraryGrant {
    pub library_id: String,
    pub connection: DeviceConnectionLevel,
    pub sync: DeviceSyncLevel,
}
#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct DeviceGrantsInfo {
    pub control: bool,
    /// A wildcard level used by every library without a per-library override.
    pub all_connection: Option<DeviceConnectionLevel>,
    /// A wildcard level used by every library without a per-library override.
    pub all_sync: Option<DeviceSyncLevel>,
    pub libraries: Vec<DeviceLibraryGrant>,
    pub administer: bool,
}
#[derive(Serialize, Deserialize, SignalPiece)]
pub struct DeviceLibraryAccess {
    pub library_id: String,
    pub alias: String,
    pub connection: DeviceConnectionLevel,
    pub sync: DeviceSyncLevel,
}
#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetDeviceGrantsRequest {
    pub fingerprint: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}
#[derive(Serialize, Deserialize, RustSignal)]
pub struct GetDeviceGrantsResponse {
    pub grants: DeviceGrantsInfo,
    pub libraries: Vec<DeviceLibraryAccess>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, SignalPiece)]
pub enum DeviceGrantsChange {
    Preset(DevicePreset),
    Set(DeviceGrantsInfo),
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetDeviceGrantsRequest {
    pub fingerprint: String,
    pub grants: DeviceGrantsChange,
    #[serde(default)]
    pub call_id: Option<u64>,
}
#[derive(Serialize, Deserialize, RustSignal)]
pub struct SetDeviceGrantsResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}
