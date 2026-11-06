use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

use super::CastLibraryState;

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
    CastNotPossible {
        library_id: String,
        state: CastLibraryState,
    },
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RequestFailedResponse {
    pub request: String,
    pub kind: RequestErrorKind,
    pub detail: String,
    #[serde(default)]
    pub cause: Option<RequestFailureCause>,
}
