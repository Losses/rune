use rinf::{DartSignal, RustSignal};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RunFsSelfTestRequest {
    pub path: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FsSelfTestProgress {
    pub layer: String,
    pub ok: bool,
    pub skipped: bool,
    pub elapsed_ms: i64,
    pub detail: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RunFsSelfTestResponse {
    pub success: bool,
    pub report_json: String,
}
