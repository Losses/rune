use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

use super::RequestErrorKind;

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
#[derive(Serialize, Deserialize, SignalPiece)]
pub struct DeviceLibraryGrant {
    pub library_id: String,
    pub connection: DeviceConnectionLevel,
    pub sync: DeviceSyncLevel,
}
#[derive(Serialize, Deserialize, SignalPiece)]
pub struct DeviceGrantsInfo {
    pub control: bool,
    pub all_connection: Option<DeviceConnectionLevel>,
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
}
#[derive(Serialize, Deserialize, RustSignal)]
pub struct GetDeviceGrantsResponse {
    pub grants: DeviceGrantsInfo,
    pub libraries: Vec<DeviceLibraryAccess>,
}
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetDeviceGrantsRequest {
    pub fingerprint: String,
    pub preset: Option<DevicePreset>,
    pub control: Option<bool>,
    pub all_connection: Option<DeviceConnectionLevel>,
    pub all_sync: Option<DeviceSyncLevel>,
    pub library_grants: Vec<DeviceLibraryGrant>,
}
#[derive(Serialize, Deserialize, RustSignal)]
pub struct SetDeviceGrantsResponse {
    pub error_kind: Option<RequestErrorKind>,
    pub error: Option<String>,
}
