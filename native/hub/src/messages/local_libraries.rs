use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

use super::{DeviceConnectionLevel, DeviceSyncLevel};

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ListLocalLibrariesRequest {}

#[derive(Serialize, Deserialize, SignalPiece, Debug)]
pub struct LocalLibrary {
    pub library_id: String,
    pub alias: String,
    pub path: String,
    pub owner_fingerprint: Option<String>,
    pub background_sync: bool,
    pub connection_access: Option<DeviceConnectionLevel>,
    pub sync_access: Option<DeviceSyncLevel>,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct ListLocalLibrariesResponse {
    pub libraries: Vec<LocalLibrary>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetLibraryBackgroundSyncRequest {
    pub library_id: String,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct SetLibraryBackgroundSyncResponse {
    pub error: Option<String>,
}
