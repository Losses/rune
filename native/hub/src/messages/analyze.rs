use rinf::{DartSignal, RustSignal};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, DartSignal)]
pub struct IfAnalyzeExistsRequest {
    pub file_id: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct IfAnalyzeExistsResponse {
    pub file_id: i32,
    pub exists: bool,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetAnalyzeCountRequest {}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetAnalyzeCountResponse {
    pub count: u64,
}
