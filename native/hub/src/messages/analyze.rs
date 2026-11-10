use rinf::{DartSignal, RustSignal};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, DartSignal)]
pub struct IfAnalyzeExistsRequest {
    pub file_id: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct IfAnalyzeExistsResponse {
    pub file_id: i32,
    pub exists: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetAnalyzeCountRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetAnalyzeCountResponse {
    pub count: u64,
    #[serde(default)]
    pub call_id: Option<u64>,
}
