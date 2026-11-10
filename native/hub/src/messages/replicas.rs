use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

use super::{DeviceConnectionLevel, DeviceSyncLevel, LibraryOpenErrorKind};

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ListPeerLibrariesRequest {
    pub fingerprint: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, SignalPiece, Debug)]
pub struct PeerLibrary {
    pub library_id: String,
    pub alias: String,
    pub added: bool,
    pub connection: DeviceConnectionLevel,
    pub sync: DeviceSyncLevel,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct ListPeerLibrariesResponse {
    pub libraries: Vec<PeerLibrary>,
    pub error_kind: Option<LibraryOpenErrorKind>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct AddRemoteLibraryRequest {
    pub fingerprint: String,
    pub library_id: String,
    pub alias: String,
    pub connection: DeviceConnectionLevel,
    pub sync: DeviceSyncLevel,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct AddRemoteLibraryResponse {
    pub record_id: String,
    pub error: Option<String>,
    #[serde(default)]
    pub call_id: Option<u64>,
}
