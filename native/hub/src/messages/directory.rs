use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct DirectoryTreeResponse {
    pub name: String,
    pub path: String,
    pub children: Vec<DirectoryTreeResponse>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchDirectoryTreeRequest {}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchDirectoryTreeResponse {
    pub root: DirectoryTreeResponse,
}
