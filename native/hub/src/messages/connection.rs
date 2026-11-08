use super::system::{RequestErrorKind, RequestFailureCause};
use database::library_registry::{LibraryKind, LibraryRecord, OwnedStorage};
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryInitializeMode {
    Portable,
    Redirected,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct TestLibraryInitializedRequest {
    pub path: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct TestLibraryInitializedResponse {
    pub path: String,
    pub success: bool,
    pub error: Option<String>,
    pub not_ready: bool,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationDestination {
    Local,
    Remote,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetMediaLibraryPathRequest {
    pub path: String,
    pub db_path: String,
    pub config_path: String,
    pub alias: String,
    pub mode: Option<LibraryInitializeMode>,
    pub plays_on: OperationDestination,
    pub hosted_on: OperationDestination,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryOpenErrorKind {
    RemoteUntrusted,
    RemoteNotAuthorized,
    RemoteUnreachable,
    RemoteProtocol,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct SetMediaLibraryPathResponse {
    pub restore_last_index: bool,
    pub error_kind: Option<LibraryOpenErrorKind>,
    pub path: String,
    pub success: bool,
    pub error: Option<String>,
    pub not_ready: bool,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RegisterLocalLibraryRequest {
    pub path: String,
    pub alias: String,
    pub mode: Option<LibraryInitializeMode>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RegisterRemoteControlRequest {
    pub fingerprint: String,
    pub alias: String,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub enum LibraryRecordKindSummary {
    Local {
        library_id: String,
        path: String,
    },
    Replica {
        library_id: String,
        path: String,
        owner_fingerprint: String,
    },
    RemoteControl {
        fingerprint: String,
    },
    Subsonic {
        endpoint: String,
        account: String,
    },
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub struct LibraryRecordSummary {
    pub record_id: String,
    pub alias: String,
    pub kind: LibraryRecordKindSummary,
}

impl From<&LibraryRecord> for LibraryRecordSummary {
    fn from(record: &LibraryRecord) -> Self {
        let kind = match &record.kind {
            LibraryKind::Owned {
                library_id,
                storage: OwnedStorage::Local { path, .. },
                ..
            } => LibraryRecordKindSummary::Local {
                library_id: library_id.to_string(),
                path: path.clone(),
            },
            LibraryKind::RuneReplica {
                library_id,
                owner_fingerprint,
                ..
            } => LibraryRecordKindSummary::Replica {
                library_id: library_id.to_string(),
                path: record.app_name().unwrap_or_default(),
                owner_fingerprint: owner_fingerprint.clone(),
            },
            LibraryKind::RuneRemoteControl { fingerprint } => {
                LibraryRecordKindSummary::RemoteControl {
                    fingerprint: fingerprint.clone(),
                }
            }
            LibraryKind::Subsonic {
                endpoint, account, ..
            } => LibraryRecordKindSummary::Subsonic {
                endpoint: endpoint.clone(),
                account: account.clone(),
            },
        };
        Self {
            record_id: record.record_id.0.to_string(),
            alias: record.alias.clone(),
            kind,
        }
    }
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterLibraryFailureKind {
    Untrusted,
    InvalidSource,
    Storage,
    RemoteUntrusted,
    RemoteNotAuthorized,
    RemoteUnreachable,
    RemoteProtocol,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub enum RegisterLibraryOutcome {
    Registered,
    NeedsInitializeMode,
    Failed(RegisterLibraryFailureKind),
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RegisterLibraryResponse {
    pub record: Option<LibraryRecordSummary>,
    pub outcome: RegisterLibraryOutcome,
    pub detail: Option<String>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct OpenLibraryRequest {
    pub call_id: u64,
    pub record_id: String,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenLibraryFailureKind {
    RecordNotFound,
    MediaFolderMissing,
    RemoteUntrusted,
    RemoteNotAuthorized,
    RemoteUnreachable,
    RemoteProtocol,
    ProtocolIncompatible,
    NotSupportedYet,
    Storage,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub enum OpenLibraryResult {
    Opened,
    Failed(OpenLibraryFailure),
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub struct OpenLibraryFailure {
    pub kind: OpenLibraryFailureKind,
    pub detail: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct OpenLibraryResponse {
    pub call_id: u64,
    pub record_id: String,
    pub result: OpenLibraryResult,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteLossKind {
    Revoked,
    ServerStopped,
    NetworkLost,
    Detached,
    ProtocolIncompatible,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoteConnectionLost {
    pub kind: RemoteLossKind,
    pub detail: String,
}

crate::implement_rinf_rust_signal_trait!(RemoteConnectionLost);

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoteLibraryReconnecting {
    pub fingerprint: String,
    pub alias: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoteLibraryReconnected {
    pub fingerprint: String,
    pub alias: String,
}

crate::implement_rinf_rust_signal_trait!(RemoteLibraryReconnecting);
crate::implement_rinf_rust_signal_trait!(RemoteLibraryReconnected);

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct AttachedControllerInfo {
    pub id: String,
    pub fingerprint: String,
    pub alias: String,
    /// Unix timestamp in seconds when this WebSocket attached.
    pub connected_at: u64,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ListControllersRequest {}

#[derive(Serialize, Deserialize, SignalPiece, RustSignal)]
pub struct ListControllersResponse {
    pub controllers: Vec<AttachedControllerInfo>,
}

#[derive(Serialize, Deserialize, SignalPiece, RustSignal)]
pub struct ControllersUpdated {
    pub controllers: Vec<AttachedControllerInfo>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct DetachControllerRequest {
    pub id: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoteRequestFailed {
    pub kind: RequestErrorKind,
    pub request: String,
    /// The failed request's call id, for requests that carry one.
    #[serde(default)]
    pub call_id: Option<u64>,
    pub detail: String,
    #[serde(default)]
    pub cause: Option<RequestFailureCause>,
}
