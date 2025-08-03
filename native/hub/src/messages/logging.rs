use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct LogDetail {
    pub id: i32,
    pub level: String,
    pub domain: String,
    pub detail: String,
    pub date: i64,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ListLogRequest {
    pub cursor: i32,
    pub page_size: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ListLogResponse {
    pub result: Vec<LogDetail>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ClearLogRequest {}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ClearLogResponse {
    pub success: bool,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RemoveLogRequest {
    pub id: i32,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoveLogResponse {
    pub id: i32,
    pub success: bool,
}
