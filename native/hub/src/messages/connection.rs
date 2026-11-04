use super::system::RequestErrorKind;
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

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationDestination {
    Local,
    Remote,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetMediaLibraryPathRequest {
    pub path: String,
    pub db_path: String,
    pub config_path: String,
    pub alias: String,
    pub mode: Option<LibraryInitializeMode>,
    pub plays_on: OperationDestination,
    pub hosted_on: OperationDestination,
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

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteLossKind {
    Revoked,
    ServerStopped,
    NetworkLost,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoteConnectionLost {
    pub kind: RemoteLossKind,
    pub detail: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoteRequestFailed {
    pub kind: RequestErrorKind,
    pub request: String,
    pub detail: String,
}
