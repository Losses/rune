use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

use super::RequestErrorKind;

#[derive(Clone, Copy, Serialize, Deserialize, SignalPiece)]
pub enum DevicePreset {
    MyDevice,
    Guest,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, SignalPiece)]
pub enum DeviceLibraryLevel {
    NotShared,
    Read,
    ReadWrite,
}
#[derive(Serialize, Deserialize, SignalPiece)]
pub struct DeviceLibraryGrant {
    pub library_id: String,
    pub level: DeviceLibraryLevel,
}
#[derive(Serialize, Deserialize, SignalPiece)]
pub struct DeviceGrantsInfo {
    pub control: bool,
    pub all_libraries: Option<DeviceLibraryLevel>,
    pub libraries: Vec<DeviceLibraryGrant>,
    pub administer: bool,
}
#[derive(Serialize, Deserialize, SignalPiece)]
pub struct DeviceLibraryAccess {
    pub library_id: String,
    pub alias: String,
    pub level: DeviceLibraryLevel,
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
    pub library_levels: Vec<DeviceLibraryGrant>,
}
#[derive(Serialize, Deserialize, RustSignal)]
pub struct SetDeviceGrantsResponse {
    pub error_kind: Option<RequestErrorKind>,
    pub error: Option<String>,
}
