use requests::NamedLibraries;
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, DartSignal)]
pub struct InitializeNodeRequest {
    pub config_path: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct InitializeNodeResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct CloseLibraryRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CloseLibraryResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct ScanAudioLibraryRequest {
    pub force: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(
    Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq, NamedLibraries,
)]
pub enum ComputingDeviceRequest {
    Cpu,
    Gpu,
}

#[derive(Debug, Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct AnalyzeAudioLibraryRequest {
    pub computing_device: ComputingDeviceRequest,
    pub workload_factor: f32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct DeduplicateAudioLibraryRequest {
    pub similarity_threshold: f32,
    pub workload_factor: f32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(
    Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq, NamedLibraries,
)]
pub enum CancelTaskType {
    AnalyzeAudioLibrary,
    ScanAudioLibrary,
    DeduplicateAudioLibrary,
}

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct CancelTaskRequest {
    pub record_id: String,
    pub r#type: CancelTaskType,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct CancelTaskResponse {
    pub record_id: String,
    pub r#type: CancelTaskType,
    pub state: super::LibraryTaskState,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Library records changed after a sync transaction or audio import.
#[derive(Deserialize, Serialize, RustSignal)]
pub struct LibraryContentChanged {
    pub path: String,
}
