use super::system::{RequestErrorKind, RequestFailureCause};
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryInitializeMode {
    Portable,
    Redirected,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct TestLibraryInitializedRequest {
    pub path: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct TestLibraryInitializedResponse {
    pub path: String,
    pub success: bool,
    pub error: Option<String>,
    pub not_ready: bool,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetMediaLibraryPathRequest {
    pub path: String,
    pub db_path: String,
    pub config_path: String,
    pub alias: String,
    pub mode: Option<LibraryInitializeMode>,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryOpenErrorKind {
    RemoteUntrusted,
    RemoteNotAuthorized,
    RemoteUnreachable,
    RemoteProtocol,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct SetMediaLibraryPathResponse {
    pub restore_last_index: bool,
    pub error_kind: Option<LibraryOpenErrorKind>,
    pub path: String,
    pub success: bool,
    pub error: Option<String>,
    pub not_ready: bool,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct AttachedControllerInfo {
    pub id: String,
    pub fingerprint: String,
    pub alias: String,
    /// Unix timestamp in seconds when this WebSocket attached.
    pub connected_at: u64,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ListControllersRequest {}

#[derive(Serialize, Deserialize, SignalPiece, RustSignal)]
pub struct ListControllersResponse {
    pub controllers: Vec<AttachedControllerInfo>,
}

#[derive(Serialize, Deserialize, SignalPiece, RustSignal)]
pub struct ControllersUpdated {
    pub controllers: Vec<AttachedControllerInfo>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct DetachControllerRequest {
    pub id: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoteRequestFailed {
    pub kind: RequestErrorKind,
    pub request: String,
    pub detail: String,
    #[serde(default)]
    pub cause: Option<RequestFailureCause>,
}
