use std::fmt;

use super::system::{RequestErrorKind, RequestFailureCause};
use database::library_registry::{
    CatalogPlacement, FsAdapter, InvalidRecord, LibraryAccess, LibraryConnectionAccess,
    LibraryRecord, ProtocolAdapter, RuneForwardMode, SubsonicOpenMode,
};
use requests::NamedLibraries;
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct GetOpenLibraryRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetOpenLibraryResponse {
    pub library_id: Option<String>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryInitializeMode {
    Portable,
    Redirected,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct TestLibraryInitializedRequest {
    pub path: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct TestLibraryInitializedResponse {
    pub path: String,
    pub not_ready: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
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
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RegisterRemoteControlRequest {
    pub fingerprint: String,
    pub alias: String,
    #[serde(default)]
    pub call_id: Option<u64>,
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
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ReplaceSubsonicCredentialRequest {
    pub record_id: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ReplaceSubsonicCredentialResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// A streaming service whose library this device can register.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamingService {
    AppleMusic,
}

/// The kind of record a library of the service has.
impl From<StreamingService> for ProtocolAdapter {
    fn from(service: StreamingService) -> Self {
        match service {
            StreamingService::AppleMusic => Self::AppleMusic,
        }
    }
}

/// How this device signs in to a streaming service. The hub keeps what it gets in
/// memory only.
#[derive(Serialize, Deserialize, SignalPiece, Clone, PartialEq, Eq)]
pub enum ServiceSignIn {
    /// Tokens the platform's own service framework handed over.
    Platform {
        user_token: String,
        developer_token: Option<String>,
    },
}

impl fmt::Debug for ServiceSignIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Platform {
                developer_token, ..
            } => f
                .debug_struct("Platform")
                .field("user_token", &"<redacted>")
                .field(
                    "developer_token",
                    &developer_token.as_ref().map(|_| "<redacted>"),
                )
                .finish(),
        }
    }
}

/// Registers the library of a streaming service, signed in with `sign_in`. A service
/// already registered keeps its record and is only signed in again.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct RegisterServiceLibraryRequest {
    pub service: StreamingService,
    pub alias: String,
    pub sign_in: ServiceSignIn,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Signs the streaming service library of `record_id` in, for example after Rune started.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SignInServiceLibraryRequest {
    pub record_id: String,
    pub sign_in: ServiceSignIn,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Sent once the library is signed in. A failure is a typed request failure.
#[derive(Deserialize, Serialize, RustSignal)]
pub struct SignInServiceLibraryResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Forgets the sign-in of the streaming service library of `record_id` on this device.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SignOutServiceLibraryRequest {
    pub record_id: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Sent once the library is signed out. A failure is a typed request failure.
#[derive(Deserialize, Serialize, RustSignal)]
pub struct SignOutServiceLibraryResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Allows or stops looking the lyrics of the songs of the streaming service library of
/// `record_id` up at an online lyrics database, for a library whose lyrics come from one. The
/// choice is kept on this device.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct SetOnlineLyricsRequest {
    pub record_id: String,
    pub enabled: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Sent once the choice is kept. A failure is a typed request failure.
#[derive(Deserialize, Serialize, RustSignal)]
pub struct SetOnlineLyricsResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
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

/// Selects where Rune stores a new library's catalog for a share without a library yet.
/// A folder that already holds a library is always joined in the share.
#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShareDataChoice {
    KeepInShare,
    ThisDeviceOnly,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RegisterOpenDalLibraryRequest {
    pub service: OpenDalLibraryService,
    pub alias: String,
    /// Applies only when the folder does not already hold a library. Existing libraries are always joined in the share.
    pub share_data_choice: ShareDataChoice,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct DetectOpenDalShareDataRequest {
    pub service: OpenDalLibraryService,
    #[serde(default)]
    pub call_id: Option<u64>,
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
        service: OpenDalServiceKind,
        endpoint: String,
        root: String,
    },
    AppleMusic,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenDalServiceKind {
    WebDav,
    S3,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub struct LibraryRecordSummary {
    pub record_id: String,
    pub alias: String,
    pub kind: LibraryRecordKindSummary,
    /// Whether the library is stored in a shared folder, which it can be synced with on request.
    pub share_sync: bool,
    pub connection_access: Option<LibraryConnectionAccessSummary>,
    pub sync_access: Option<LibraryAccessSummary>,
    /// Whether the library is signed in now, for a library whose sign-in lives on this device
    /// rather than in a credential its record keeps. None for every other library.
    pub signed_in: Option<bool>,
    /// Whether the library may look its songs' lyrics up at an online lyrics database, for a
    /// library whose lyrics come from one. None for every other library.
    pub online_lyrics: Option<bool>,
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
    DuplicateLibraryId,
    OpenDalInvalid,
    ServiceInvalid,
}

impl From<InvalidRecord> for InvalidRecordReasonSummary {
    fn from(value: InvalidRecord) -> Self {
        match value {
            InvalidRecord::NativeInvalid => Self::NativeInvalid,
            InvalidRecord::ReplicaInvalid => Self::ReplicaInvalid,
            InvalidRecord::RemoteControlInvalid => Self::RemoteControlInvalid,
            InvalidRecord::SubsonicInvalid => Self::SubsonicInvalid,
            InvalidRecord::DuplicateLibraryId => Self::DuplicateLibraryId,
            InvalidRecord::OpenDalInvalid => Self::OpenDalInvalid,
            InvalidRecord::ServiceInvalid => Self::ServiceInvalid,
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
                        service: OpenDalServiceKind::WebDav,
                        endpoint: endpoint.clone(),
                        root: root.clone(),
                    }
                }
                Some(FsAdapter::S3 { endpoint, root, .. }) => LibraryRecordKindSummary::OpenDal {
                    library_id: record.library_id.unwrap_or_default().to_string(),
                    service: OpenDalServiceKind::S3,
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
            ProtocolAdapter::AppleMusic => LibraryRecordKindSummary::AppleMusic,
        };
        Self {
            record_id: record.record_id.0.to_string(),
            alias: record.alias.clone(),
            kind,
            share_sync: matches!(record.catalog, CatalogPlacement::InShare(_)),
            connection_access: record.connection_access().map(Into::into),
            sync_access: record.access_level().map(Into::into),
            signed_in: None,
            online_lyrics: None,
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
pub struct ListLibraryRecordsRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ListLibraryRecordsResponse {
    pub records: Vec<LibraryRecordSummary>,
    pub invalid_records: Vec<InvalidLibraryRecordSummary>,
    pub open: Option<OpenRecord>,
    pub last_open_record: Option<String>,
    #[serde(default)]
    pub call_id: Option<u64>,
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
    #[serde(default)]
    pub call_id: Option<u64>,
    /// Also delete what the library keeps only on this device, as a reset of the app does.
    #[serde(default)]
    pub delete_device_data: bool,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveLibraryRecordFailureKind {
    RecordNotFound,
    RecordIsOpen,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoveLibraryRecordResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
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
    OpenDalReadOnly,
    OpenDalShareInvalid,
    OpenDalShareChanged,
    SyncNotShared,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub enum OpenDalShareDataDetection {
    ExistingLibrary { library_id: String },
    WritableEmpty,
    ReadOnly,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct DetectOpenDalShareDataResponse {
    pub result: OpenDalShareDataDetection,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub enum RegisterLibraryOutcome {
    Registered,
    NeedsInitializeMode,
    NeedsSubsonicMode {
        allowed: Vec<SubsonicOpenModeMessage>,
    },
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RegisterLibraryResponse {
    pub record: Option<LibraryRecordSummary>,
    pub outcome: RegisterLibraryOutcome,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct OpenLibraryRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
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
    /// The library's database was written by a newer version of Rune.
    NewerVersion,
    Storage,
    DocumentFolderUnavailable,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct OpenLibraryResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
    pub record_id: String,
    pub generation: u64,
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
pub struct ListControllersRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, SignalPiece, RustSignal)]
pub struct ListControllersResponse {
    pub controllers: Vec<AttachedControllerInfo>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, SignalPiece, RustSignal)]
pub struct ControllersUpdated {
    pub controllers: Vec<AttachedControllerInfo>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct DetachControllerRequest {
    pub id: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// Sent once the controller is no longer attached.
#[derive(Debug, Serialize, Deserialize, RustSignal)]
pub struct DetachControllerResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
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
    use super::{
        OpenDalLibraryService, RegisterOpenDalLibraryRequest, RegisterServiceLibraryRequest,
        ServiceSignIn, ShareDataChoice, StreamingService,
    };

    fn round_trip(service: OpenDalLibraryService) {
        let request = RegisterOpenDalLibraryRequest {
            call_id: None,
            service: service.clone(),
            alias: "music".into(),
            share_data_choice: ShareDataChoice::ThisDeviceOnly,
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

    #[test]
    fn service_sign_in_survives_the_dart_wire_format_and_hides_its_tokens() {
        let request = RegisterServiceLibraryRequest {
            service: StreamingService::AppleMusic,
            alias: "Apple Music".into(),
            sign_in: ServiceSignIn::Platform {
                user_token: "user-secret".into(),
                developer_token: Some("developer-secret".into()),
            },
            call_id: Some(7),
        };
        let bytes = rinf::serialize(&request).unwrap();
        let decoded: RegisterServiceLibraryRequest = rinf::deserialize(&bytes).unwrap();
        assert_eq!(decoded.sign_in, request.sign_in);
        assert_eq!(decoded.call_id, Some(7));
        let debug = format!("{:?}", request.sign_in);
        assert!(!debug.contains("secret"), "{debug}");
    }
}
