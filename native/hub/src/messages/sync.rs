use rinf::{DartSignal, RustSignal};
use serde::{Deserialize, Serialize};

use super::MixQuery;

/// A device-local selection for persistent audio downloads.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetDownloadRuleRequest {
    pub request_id: String,
    pub queries: Vec<MixQuery>,
    pub mix_id: Option<i32>,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct SetDownloadRuleResponse {
    pub request_id: String,
    pub rule_id: String,
    pub error: Option<String>,
}
