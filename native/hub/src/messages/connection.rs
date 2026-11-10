use super::system::{RequestErrorKind, RequestFailureCause};
use database::library_registry::{
    CatalogPlacement, FsAdapter, InvalidRecord, LibraryAccess, LibraryConnectionAccess,
    LibraryRecord, ProtocolAdapter, RuneForwardMode, SubsonicOpenMode,
};
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
pub enum LibraryOpenErrorKind {
    RemoteUntrusted,
    RemoteNotAuthorized,
    RemoteUnreachable,
    RemoteProtocol,
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

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubsonicOpenModeMessage {
    RemoteControl,
    PlayOnThisDevice,
}

impl From<SubsonicOpenModeMessage> for SubsonicOpenMode {
    fn from(value: SubsonicOpenModeMessage) -> Self {
        match value {
            SubsonicOpenModeMessage::RemoteControl => Self::RemoteControl,
            SubsonicOpenModeMessage::PlayOnThisDevice => Self::PlayOnThisDevice,
        }
    }
}

impl From<SubsonicOpenMode> for SubsonicOpenModeMessage {
    fn from(value: SubsonicOpenMode) -> Self {
        match value {
            SubsonicOpenMode::RemoteControl => Self::RemoteControl,
            SubsonicOpenMode::PlayOnThisDevice => Self::PlayOnThisDevice,
        }
    }
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RegisterSubsonicLibraryRequest {
    pub endpoint: String,
    pub username: String,
    pub password: String,
    pub alias: String,
    pub mode: Option<SubsonicOpenModeMessage>,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub enum OpenDalLibraryService {
    WebDav {
        endpoint: String,
        root: String,
        username: String,
        password: String,
    },
    S3 {
        endpoint: String,
        bucket: String,
        region: String,
        root: String,
        access_key_id: String,
        secret_access_key: String,
    },
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RegisterOpenDalLibraryRequest {
    pub service: OpenDalLibraryService,
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
        owner_fingerprint: String,
        owner_alias: Option<String>,
    },
    RemoteControl {
        fingerprint: String,
        device_alias: Option<String>,
        endpoint: Option<String>,
    },
    Subsonic {
        endpoint: String,
        account: String,
    },
    OpenDal {
        library_id: String,
        service: String,
        endpoint: String,
        root: String,
    },
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub struct LibraryRecordSummary {
    pub record_id: String,
    pub alias: String,
    pub kind: LibraryRecordKindSummary,
    pub background_sync: Option<bool>,
    pub connection_access: Option<LibraryConnectionAccessSummary>,
    pub sync_access: Option<LibraryAccessSummary>,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub struct InvalidLibraryRecordSummary {
    pub record_id: String,
    pub alias: String,
    pub kind: LibraryRecordKindSummary,
    pub reason: InvalidRecordReasonSummary,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidRecordReasonSummary {
    NativeInvalid,
    ReplicaInvalid,
    RemoteControlInvalid,
    SubsonicInvalid,
    BackgroundSyncWithoutCatalog,
    DuplicateLibraryId,
    OpenDalInvalid,
}

impl From<InvalidRecord> for InvalidRecordReasonSummary {
    fn from(value: InvalidRecord) -> Self {
        match value {
            InvalidRecord::NativeInvalid => Self::NativeInvalid,
            InvalidRecord::ReplicaInvalid => Self::ReplicaInvalid,
            InvalidRecord::RemoteControlInvalid => Self::RemoteControlInvalid,
            InvalidRecord::SubsonicInvalid => Self::SubsonicInvalid,
            InvalidRecord::BackgroundSyncWithoutCatalog => Self::BackgroundSyncWithoutCatalog,
            InvalidRecord::DuplicateLibraryId => Self::DuplicateLibraryId,
            InvalidRecord::OpenDalInvalid => Self::OpenDalInvalid,
        }
    }
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryAccessSummary {
    NotShared,
    Read,
    ReadWrite,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryConnectionAccessSummary {
    None,
    View,
    ViewEdit,
}

impl From<&LibraryRecord> for LibraryRecordSummary {
    fn from(record: &LibraryRecord) -> Self {
        let kind = match &record.protocol {
            ProtocolAdapter::Native => match record.fs.first() {
                Some(FsAdapter::WebDav { endpoint, root, .. }) => {
                    LibraryRecordKindSummary::OpenDal {
                        library_id: record.library_id.unwrap_or_default().to_string(),
                        service: "webdav".into(),
                        endpoint: endpoint.clone(),
                        root: root.clone(),
                    }
                }
                Some(FsAdapter::S3 { endpoint, root, .. }) => LibraryRecordKindSummary::OpenDal {
                    library_id: record.library_id.unwrap_or_default().to_string(),
                    service: "s3".into(),
                    endpoint: endpoint.clone(),
                    root: root.clone(),
                },
                _ => LibraryRecordKindSummary::Local {
                    library_id: record.library_id.unwrap_or_default().to_string(),
                    path: record.local_folder_path().unwrap_or_default().to_owned(),
                },
            },
            ProtocolAdapter::RuneForward {
                owner_fingerprint,
                mode: RuneForwardMode::Replica { .. },
            } => LibraryRecordKindSummary::Replica {
                library_id: record.library_id.unwrap_or_default().to_string(),
                owner_fingerprint: owner_fingerprint.clone(),
                owner_alias: None,
            },
            ProtocolAdapter::RuneForward {
                owner_fingerprint: fingerprint,
                mode: RuneForwardMode::RemoteControl,
            } => LibraryRecordKindSummary::RemoteControl {
                fingerprint: fingerprint.clone(),
                device_alias: None,
                endpoint: None,
            },
            ProtocolAdapter::SubsonicTranslate { endpoint, .. } => {
                LibraryRecordKindSummary::Subsonic {
                    endpoint: endpoint.clone(),
                    account: String::new(),
                }
            }
        };
        Self {
            record_id: record.record_id.0.to_string(),
            alias: record.alias.clone(),
            kind,
            background_sync: (record.catalog != CatalogPlacement::None)
                .then(|| record.background_sync()),
            connection_access: record.connection_access().map(Into::into),
            sync_access: record.access_level().map(Into::into),
        }
    }
}

impl From<LibraryAccess> for LibraryAccessSummary {
    fn from(value: LibraryAccess) -> Self {
        match value {
            LibraryAccess::NotShared => Self::NotShared,
            LibraryAccess::Read => Self::Read,
            LibraryAccess::ReadWrite => Self::ReadWrite,
        }
    }
}

impl From<LibraryConnectionAccess> for LibraryConnectionAccessSummary {
    fn from(value: LibraryConnectionAccess) -> Self {
        match value {
            LibraryConnectionAccess::None => Self::None,
            LibraryConnectionAccess::View => Self::View,
            LibraryConnectionAccess::ViewEdit => Self::ViewEdit,
        }
    }
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ListLibraryRecordsRequest {}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ListLibraryRecordsResponse {
    pub records: Vec<LibraryRecordSummary>,
    pub invalid_records: Vec<InvalidLibraryRecordSummary>,
    pub open: Option<OpenRecord>,
    pub last_open_record: Option<String>,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub struct OpenRecord {
    pub record_id: String,
    pub generation: u64,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct LibraryRecordsChanged {
    pub records: Vec<LibraryRecordSummary>,
    pub invalid_records: Vec<InvalidLibraryRecordSummary>,
    pub open: Option<OpenRecord>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RemoveLibraryRecordRequest {
    pub record_id: String,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveLibraryRecordFailureKind {
    RecordNotFound,
    RecordIsOpen,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub enum RemoveLibraryRecordOutcome {
    Removed,
    Failed(RemoveLibraryRecordFailureKind),
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoveLibraryRecordResponse {
    pub outcome: RemoveLibraryRecordOutcome,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterLibraryFailureKind {
    InvalidRecord,
    Untrusted,
    InvalidSource,
    Storage,
    RemoteUntrusted,
    RemoteNotAuthorized,
    RemoteUnreachable,
    RemoteProtocol,
    SubsonicUnreachable,
    SubsonicWrongCredentials,
    SubsonicServerTooOld,
    SubsonicClientTooOld,
    SubsonicRoleMissing,
    SubsonicNotSubsonic,
    OpenDalAuthenticationRejected,
    OpenDalUnreachable,
    OpenDalNotFolder,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub enum RegisterLibraryOutcome {
    Registered,
    NeedsInitializeMode,
    NeedsSubsonicMode {
        allowed: Vec<SubsonicOpenModeMessage>,
    },
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
    InvalidRecord,
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
    pub generation: u64,
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
crate::implement_rinf_rust_signal_trait!(LibraryRecordsChanged);

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

#[cfg(test)]
mod tests {
    use super::{OpenDalLibraryService, RegisterOpenDalLibraryRequest};

    fn round_trip(service: OpenDalLibraryService) {
        let request = RegisterOpenDalLibraryRequest {
            service: service.clone(),
            alias: "music".into(),
        };
        let bytes = rinf::serialize(&request).unwrap();
        let decoded: RegisterOpenDalLibraryRequest = rinf::deserialize(&bytes).unwrap();
        assert_eq!(decoded.service, service);
        assert_eq!(decoded.alias, "music");
    }

    // Dart sends this request in rinf's binary format, which cannot decode
    // internally tagged enums.
    #[test]
    fn opendal_register_request_survives_the_dart_wire_format() {
        round_trip(OpenDalLibraryService::WebDav {
            endpoint: "http://127.0.0.1:1".into(),
            root: "/music".into(),
            username: "u".into(),
            password: "p".into(),
        });
        round_trip(OpenDalLibraryService::S3 {
            endpoint: "http://127.0.0.1:1".into(),
            bucket: "b".into(),
            region: "r".into(),
            root: "/".into(),
            access_key_id: "k".into(),
            secret_access_key: "s".into(),
        });
    }
}
