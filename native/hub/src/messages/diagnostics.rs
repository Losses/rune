use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RunFsSelfTestRequest {
    pub path: String,
    pub db_path: String,
    #[serde(default)]
    pub call_id: Option<u64>,
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
    pub wal_probe: WalSelfTestResult,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal, SignalPiece)]
pub struct WalSelfTestResult {
    pub supported: bool,
    pub detail: Option<String>,
}
