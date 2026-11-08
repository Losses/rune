use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

use super::CastLibraryState;
use crate::CallIdentified;

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SystemInfoRequest {}

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
    pub fn for_request<T: crate::CallIdentified>(
        request: &T,
        request_name: &str,
        kind: RequestErrorKind,
        detail: String,
        cause: Option<RequestFailureCause>,
    ) -> Self {
        Self {
            request: request_name.to_owned(),
            call_id: CallIdentified::call_id(request),
            kind,
            detail,
            cause,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{CallIdentified, messages::*};

    #[test]
    fn search_request_uses_its_declared_call_id() {
        let request = SearchForRequest {
            call_id: 42,
            query_str: "search".to_owned(),
            fields: Vec::new(),
            n: 10,
        };
        assert_eq!(CallIdentified::call_id(&request), Some(42));
    }

    #[test]
    fn requests_without_call_ids_use_the_default() {
        let request = SystemInfoRequest {};
        assert_eq!(CallIdentified::call_id(&request), None);
    }
}
