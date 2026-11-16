use database::actions::logging::LogLevel;
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SignalPiece)]
pub enum LogDetailLevel {
    Info,
    Warning,
    Error,
    Debug,
}

impl From<LogLevel> for LogDetailLevel {
    fn from(value: LogLevel) -> Self {
        match value {
            LogLevel::Info => Self::Info,
            LogLevel::Warning => Self::Warning,
            LogLevel::Error => Self::Error,
            LogLevel::Debug => Self::Debug,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct LogDetail {
    pub id: i32,
    pub level: LogDetailLevel,
    pub domain: String,
    pub detail: String,
    pub date: i64,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ListLogRequest {
    pub cursor: i32,
    pub page_size: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ListLogResponse {
    pub result: Vec<LogDetail>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ClearLogRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ClearLogResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RemoveLogRequest {
    pub id: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoveLogResponse {
    pub id: i32,
    #[serde(default)]
    pub call_id: Option<u64>,
}
