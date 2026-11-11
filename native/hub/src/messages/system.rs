use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

use super::CastLibraryState;

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SystemInfoRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct SystemInfoResponse {
    pub build_date: String,
    pub build_sha: String,
    pub build_commit_timestamp: String,
    pub build_rustc_semver: String,
    pub system_name: String,
    pub system_kernel_version: String,
    pub system_os_version: String,
    pub system_host_name: String,
    pub users: Vec<String>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CrashResponse {
    pub detail: String,
}

/// The category of a failed request, shared by local and remote dispatch.
#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestErrorKind {
    NotFound,
    InvalidInput,
    Unavailable,
    Internal,
    PermissionDenied,
    Conflict,
    PeerNeedsUpdate,
}

#[derive(Deserialize, Serialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub enum RequestFailureCause {
    StorageBusy,
    CastNotPossible {
        library_id: String,
        state: CastLibraryState,
    },
    CertificateMismatch,
    PublicKeyMismatch,
    RegistrationConflict,
    Reconnecting {
        alias: String,
    },
    PeerNeedsUpdate {
        update_side: ProtocolUpdateSide,
        peer_alias: Option<String>,
        local_version: String,
        peer_version: Option<String>,
    },
    SubsonicNotAuthorized,
}

#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolUpdateSide {
    ThisDevice,
    Peer,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RequestFailedResponse {
    pub request: String,
    #[serde(default)]
    pub call_id: Option<u64>,
    pub kind: RequestErrorKind,
    pub detail: String,
    #[serde(default)]
    pub cause: Option<RequestFailureCause>,
}

impl RequestFailedResponse {
    pub fn for_request<T: crate::RequestCallId>(
        request: &T,
        request_name: &str,
        kind: RequestErrorKind,
        detail: String,
        cause: Option<RequestFailureCause>,
    ) -> Self {
        Self {
            request: request_name.to_owned(),
            call_id: request.call_id(),
            kind,
            detail,
            cause,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::RequestCallId;
    use crate::messages::*;

    #[test]
    fn search_request_uses_its_declared_call_id() {
        let request = SearchForRequest {
            call_id: Some(42),
            query_str: "search".to_owned(),
            fields: Vec::new(),
            n: 10,
        };
        assert_eq!(request.call_id(), Some(42));
    }

    #[test]
    fn requests_without_call_ids_use_the_default() {
        let request = SystemInfoRequest { call_id: None };
        assert_eq!(request.call_id(), None);
    }
}
