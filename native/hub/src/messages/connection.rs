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

#[derive(Deserialize, Serialize, RustSignal)]
pub struct SetMediaLibraryPathResponse {
    pub path: String,
    pub success: bool,
    pub error: Option<String>,
    pub not_ready: bool,
}
