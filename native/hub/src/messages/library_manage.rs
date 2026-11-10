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
    pub success: bool,
    pub error_kind: Option<super::RequestErrorKind>,
    pub error: Option<String>,
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

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ScanAudioLibraryRequest {
    pub scan_id: String,
    pub force: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanTaskType {
    IndexFiles,
    ScanCoverArts,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ScanAudioLibraryProgress {
    pub scan_id: String,
    pub record_id: String,
    pub progress: i32,
    pub total: i32,
    pub task: ScanTaskType,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ScanAudioLibraryResponse {
    pub scan_id: String,
    pub status: String,
    pub error: String,
    pub record_id: String,
    pub progress: i32,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComputingDeviceRequest {
    Cpu,
    Gpu,
}

#[derive(Debug, Serialize, Deserialize, DartSignal)]
pub struct AnalyzeAudioLibraryRequest {
    pub computing_device: ComputingDeviceRequest,
    pub workload_factor: f32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct AnalyzeAudioLibraryProgress {
    pub record_id: String,
    pub progress: i32,
    pub total: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct AnalyzeAudioLibraryResponse {
    pub record_id: String,
    pub total: i32,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct DeduplicateAudioLibraryRequest {
    pub similarity_threshold: f32,
    pub workload_factor: f32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct DeduplicateAudioLibraryProgress {
    pub record_id: String,
    pub progress: i32,
    pub total: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct DeduplicateAudioLibraryResponse {
    pub record_id: String,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelTaskType {
    AnalyzeAudioLibrary,
    ScanAudioLibrary,
    DeduplicateAudioLibrary,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct CancelTaskRequest {
    pub scan_id: Option<String>,
    pub record_id: String,
    pub r#type: CancelTaskType,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct CancelTaskResponse {
    pub record_id: String,
    pub r#type: CancelTaskType,
    pub success: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Library records changed after a sync transaction or audio import.
#[derive(Deserialize, Serialize, RustSignal)]
pub struct LibraryContentChanged {
    pub path: String,
}
