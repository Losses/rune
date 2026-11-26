use database::actions::metadata::MissingHostFilePolicy as MetadataMissingHostFilePolicy;
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

use super::MixQuery;

/// Typed failures reported by the background share worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, SignalPiece)]
pub enum ShareSyncFailure {
    ReadOnlyShare,
    ProbeFailed,
    ShareChanged,
    ObjectMissing,
    StoreUnavailable,
    CredentialsRejected,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct LibrarySyncStatusUpdated {
    pub library_id: String,
    pub share_failure: Option<ShareSyncFailure>,
}

crate::implement_rinf_rust_signal_trait!(LibrarySyncStatusUpdated);

/// A device-local selection for persistent audio downloads.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetDownloadRuleRequest {
    pub request_id: String,
    pub queries: Vec<MixQuery>,
    pub mix_id: Option<i32>,
    pub enabled: bool,
    pub delete_unselected_files: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct SetDownloadRuleResponse {
    pub request_id: String,
    pub rule_id: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RemoveDownloadRuleRequest {
    pub request_id: String,
    pub rule_id: String,
    pub delete_unselected_files: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}
#[derive(Serialize, Deserialize, RustSignal)]
pub struct RemoveDownloadRuleResponse {
    pub request_id: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Clone, Copy, Serialize, Deserialize, SignalPiece)]
pub enum MissingHostFilePolicy {
    RemoveEverywhere,
    KeepAsRemote,
}

impl From<MetadataMissingHostFilePolicy> for MissingHostFilePolicy {
    fn from(value: MetadataMissingHostFilePolicy) -> Self {
        match value {
            MetadataMissingHostFilePolicy::RemoveEverywhere => Self::RemoveEverywhere,
            MetadataMissingHostFilePolicy::KeepAsRemote => Self::KeepAsRemote,
        }
    }
}
impl From<MissingHostFilePolicy> for MetadataMissingHostFilePolicy {
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
    #[serde(default)]
    pub call_id: Option<u64>,
}
#[derive(Serialize, Deserialize, RustSignal)]
pub struct GetSyncSettingsResponse {
    pub request_id: String,
    pub media_server: bool,
    pub broadcast: bool,
    pub accept_remote_control: bool,
    pub accept_sync: bool,
    pub auto_device_sync: bool,
    pub auto_share_sync: bool,
    pub missing_host_file: MissingHostFilePolicy,
    #[serde(default)]
    pub call_id: Option<u64>,
}
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetSyncSettingsRequest {
    pub request_id: String,
    pub media_server: bool,
    pub broadcast: bool,
    pub accept_remote_control: bool,
    pub accept_sync: bool,
    pub auto_device_sync: bool,
    pub auto_share_sync: bool,
    pub missing_host_file: MissingHostFilePolicy,
    #[serde(default)]
    pub call_id: Option<u64>,
}
#[derive(Serialize, Deserialize, RustSignal)]
pub struct SetSyncSettingsResponse {
    pub request_id: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Sync one library stored in a shared folder with that folder now. The answer comes when the
/// pass has ended.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SyncShareLibraryRequest {
    pub library_id: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}
#[derive(Serialize, Deserialize, RustSignal)]
pub struct SyncShareLibraryResponse {
    pub share_failure: Option<ShareSyncFailure>,
    #[serde(default)]
    pub call_id: Option<u64>,
}
