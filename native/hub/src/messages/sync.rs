use rinf::{DartSignal, RustSignal, SignalPiece};
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

#[derive(Clone, Copy, Serialize, Deserialize, SignalPiece)]
pub enum MissingHostFilePolicy {
    RemoveEverywhere,
    KeepAsRemote,
}

impl From<database::actions::metadata::MissingHostFilePolicy> for MissingHostFilePolicy {
    fn from(value: database::actions::metadata::MissingHostFilePolicy) -> Self {
        match value {
            database::actions::metadata::MissingHostFilePolicy::RemoveEverywhere => {
                Self::RemoveEverywhere
            }
            database::actions::metadata::MissingHostFilePolicy::KeepAsRemote => Self::KeepAsRemote,
        }
    }
}
impl From<MissingHostFilePolicy> for database::actions::metadata::MissingHostFilePolicy {
    fn from(value: MissingHostFilePolicy) -> Self {
        match value {
            MissingHostFilePolicy::RemoveEverywhere => Self::RemoveEverywhere,
            MissingHostFilePolicy::KeepAsRemote => Self::KeepAsRemote,
        }
    }
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetSyncSettingsRequest {
    pub request_id: String,
}
#[derive(Serialize, Deserialize, RustSignal)]
pub struct GetSyncSettingsResponse {
    pub request_id: String,
    pub listen: bool,
    pub missing_host_file: MissingHostFilePolicy,
    pub error: Option<String>,
}
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetSyncSettingsRequest {
    pub request_id: String,
    pub listen: bool,
    pub missing_host_file: MissingHostFilePolicy,
}
#[derive(Serialize, Deserialize, RustSignal)]
pub struct SetSyncSettingsResponse {
    pub request_id: String,
    pub error: Option<String>,
}
