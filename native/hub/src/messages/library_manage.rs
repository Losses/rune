use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, DartSignal)]
pub struct CloseLibraryRequest {
    pub path: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CloseLibraryResponse {
    pub path: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ScanAudioLibraryRequest {
    pub path: String,
    pub force: bool,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanTaskType {
    IndexFiles,
    ScanCoverArts,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ScanAudioLibraryProgress {
    pub path: String,
    pub progress: i32,
    pub total: i32,
    pub task: ScanTaskType,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ScanAudioLibraryResponse {
    pub path: String,
    pub progress: i32,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComputingDeviceRequest {
    Cpu,
    Gpu,
}

#[derive(Debug, Serialize, Deserialize, DartSignal)]
pub struct AnalyzeAudioLibraryRequest {
    pub path: String,
    pub computing_device: ComputingDeviceRequest,
    pub workload_factor: f32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct AnalyzeAudioLibraryProgress {
    pub path: String,
    pub progress: i32,
    pub total: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct AnalyzeAudioLibraryResponse {
    pub path: String,
    pub total: i32,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct DeduplicateAudioLibraryRequest {
    pub path: String,
    pub similarity_threshold: f32,
    pub workload_factor: f32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct DeduplicateAudioLibraryProgress {
    pub path: String,
    pub progress: i32,
    pub total: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct DeduplicateAudioLibraryResponse {
    pub path: String,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelTaskType {
    AnalyzeAudioLibrary,
    ScanAudioLibrary,
    DeduplicateAudioLibrary,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct CancelTaskRequest {
    pub path: String,
    pub r#type: CancelTaskType,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct CancelTaskResponse {
    pub path: String,
    pub r#type: CancelTaskType,
    pub success: bool,
}
