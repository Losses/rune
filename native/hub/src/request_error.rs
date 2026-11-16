use anyhow::Error;
use database::error::ExpectedError;
use thiserror::Error as ThisError;

use crate::access_policy::Denial;
use crate::messages::{
    CastLibraryState, LibraryOpenErrorKind, LibraryTaskKind, OpenLibraryFailureKind,
    ProtocolUpdateSide, RegisterLibraryFailureKind, RemoveLibraryRecordFailureKind,
    RequestErrorKind, RequestFailureCause, ScrobbleAuthenticationFailure,
};
use crate::rpc::UnsupportedReason;
use discovery::protocol_version::ProtocolVersion;
use playback::player::PlayerCommandError;

/// Expected failures at the request boundary.
#[derive(Debug, ThisError)]
pub enum ExpectedRequestError {
    #[error("{detail}")]
    InvalidInput { detail: String },
    #[error("reconnecting to {alias}")]
    Reconnecting { alias: String },
    #[error("{what} unavailable")]
    Unavailable { what: &'static str },
    #[error("Subsonic account is not authorized for this request")]
    SubsonicNotAuthorized,
    #[error("{request} is unsupported by this target: {reason}")]
    Unsupported {
        request: &'static str,
        reason: UnsupportedReason,
    },
    #[error("A host presented a different certificate")]
    CertificateMismatch,
    #[error("The submitted public key does not match its fingerprint")]
    PublicKeyMismatch,
    #[error("A device with this fingerprint is already registered")]
    RegistrationConflict,
    #[error(
        "protocol versions are incompatible: this device {local_version}, peer {peer_version:?}"
    )]
    PeerNeedsUpdate {
        update_side: ProtocolUpdateSide,
        peer_alias: Option<String>,
        local_version: ProtocolVersion,
        peer_version: Option<ProtocolVersion>,
    },
    #[error("Renderer request failed: {detail}")]
    Remote {
        kind: RequestErrorKind,
        detail: String,
        cause: Option<RequestFailureCause>,
    },
    #[error("Unexpected response: expected {expected}, received {actual}")]
    UnexpectedResponse { expected: String, actual: String },
    #[error("Queue changed: expected revision {expected}, current revision {current}")]
    StaleQueueRevision { expected: u64, current: u64 },
    #[error("Cannot start a library task while {task} is running")]
    LibraryTaskAlreadyRunning { task: LibraryTaskKind },
    #[error("Handoff was superseded by a newer request")]
    HandoffSuperseded,
    #[error("Player command was superseded by a newer request")]
    PlayerCommandSuperseded,
    #[error("Player audio output is unavailable")]
    PlayerOutputUnavailable,
    #[error("Player audio source is unavailable")]
    PlayerSourceUnavailable,
    #[error("Player command cannot be applied")]
    PlayerCommandCannotApply,
    #[error("Player command timed out")]
    PlayerCommandTimedOut,
    #[error("Cast is not possible for library {library_id}: {state:?}")]
    CastNotPossible {
        library_id: String,
        state: CastLibraryState,
    },
    #[error("The license key is invalid")]
    LicenseInvalid,
    #[error("Library registration failed: {kind:?}")]
    LibraryRegistrationFailed { kind: RegisterLibraryFailureKind },
    #[error("Remote library could not be opened: {kind:?}")]
    RemoteOpenFailed { kind: LibraryOpenErrorKind },
    #[error("Library could not be opened: {kind:?}")]
    LibraryOpenFailed { kind: OpenLibraryFailureKind },
    #[error("Library record could not be removed: {kind:?}")]
    LibraryRecordRemovalFailed {
        kind: RemoveLibraryRecordFailureKind,
    },
    #[error("Scrobble authentication failed: {failure:?}")]
    ScrobbleAuthentication {
        failure: ScrobbleAuthenticationFailure,
    },
}

impl ExpectedRequestError {
    /// Retain a request validation failure in an anyhow error chain.
    pub fn invalid_input(detail: impl Into<String>) -> Self {
        Self::InvalidInput {
            detail: detail.into(),
        }
    }
}

pub fn player_command_error(error: PlayerCommandError) -> ExpectedRequestError {
    match error {
        PlayerCommandError::ThreadUnavailable => ExpectedRequestError::Unavailable {
            what: "player thread",
        },
        PlayerCommandError::Superseded => ExpectedRequestError::PlayerCommandSuperseded,
        PlayerCommandError::OutputUnavailable => ExpectedRequestError::PlayerOutputUnavailable,
        PlayerCommandError::SourceUnavailable => ExpectedRequestError::PlayerSourceUnavailable,
        PlayerCommandError::CannotApply => ExpectedRequestError::PlayerCommandCannotApply,
        PlayerCommandError::TimedOut => ExpectedRequestError::PlayerCommandTimedOut,
    }
}

/// Context attached to a failed request that carries a call id, so the
/// failure sent back over the network names the call it belongs to.
#[derive(Debug, Clone, Copy)]
pub struct FailedCall {
    pub call_id: u64,
}

impl std::fmt::Display for FailedCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "call {}", self.call_id)
    }
}

/// The request failure kind of a library registration failure.
fn registration_failure_kind(kind: RegisterLibraryFailureKind) -> RequestErrorKind {
    use RegisterLibraryFailureKind as Kind;
    match kind {
        Kind::InvalidRecord
        | Kind::InvalidSource
        | Kind::SubsonicNotSubsonic
        | Kind::OpenDalNotFolder
        | Kind::OpenDalShareInvalid
        | Kind::OpenDalShareChanged => RequestErrorKind::InvalidInput,
        Kind::Untrusted
        | Kind::RemoteUntrusted
        | Kind::RemoteNotAuthorized
        | Kind::SubsonicWrongCredentials
        | Kind::SubsonicRoleMissing
        | Kind::OpenDalAuthenticationRejected
        | Kind::OpenDalReadOnly => RequestErrorKind::PermissionDenied,
        Kind::Storage
        | Kind::RemoteUnreachable
        | Kind::RemoteProtocol
        | Kind::SubsonicUnreachable
        | Kind::SubsonicServerTooOld
        | Kind::SubsonicClientTooOld
        | Kind::OpenDalUnreachable => RequestErrorKind::Unavailable,
    }
}

/// The request failure kind of a library that could not be opened.
fn open_failure_kind(kind: OpenLibraryFailureKind) -> RequestErrorKind {
    use OpenLibraryFailureKind as Kind;
    match kind {
        Kind::InvalidRecord => RequestErrorKind::InvalidInput,
        Kind::RecordNotFound | Kind::MediaFolderMissing => RequestErrorKind::NotFound,
        Kind::RemoteUntrusted | Kind::RemoteNotAuthorized => RequestErrorKind::PermissionDenied,
        Kind::ProtocolIncompatible => RequestErrorKind::PeerNeedsUpdate,
        Kind::RemoteUnreachable | Kind::RemoteProtocol | Kind::NotSupportedYet | Kind::Storage => {
            RequestErrorKind::Unavailable
        }
    }
}

/// Classify a request failure without relying on its diagnostic text.
pub fn classify(error: &Error) -> RequestErrorKind {
    if sync_orm::is_storage_busy(error) {
        return RequestErrorKind::Unavailable;
    }
    for cause in error.chain() {
        if cause.is::<Denial>() {
            return RequestErrorKind::PermissionDenied;
        }
        if let Some(expected) = cause.downcast_ref::<ExpectedError>() {
            return match expected {
                ExpectedError::NotFound { .. } => RequestErrorKind::NotFound,
                ExpectedError::InvalidInput { .. } => RequestErrorKind::InvalidInput,
            };
        }
        if let Some(expected) = cause.downcast_ref::<ExpectedRequestError>() {
            return match expected {
                ExpectedRequestError::InvalidInput { .. } => RequestErrorKind::InvalidInput,
                ExpectedRequestError::Unavailable { .. }
                | ExpectedRequestError::Reconnecting { .. }
                | ExpectedRequestError::Unsupported { .. } => RequestErrorKind::Unavailable,
                ExpectedRequestError::SubsonicNotAuthorized => RequestErrorKind::PermissionDenied,
                ExpectedRequestError::CertificateMismatch => RequestErrorKind::InvalidInput,
                ExpectedRequestError::PublicKeyMismatch => RequestErrorKind::InvalidInput,
                ExpectedRequestError::RegistrationConflict => RequestErrorKind::Conflict,
                ExpectedRequestError::PeerNeedsUpdate { .. } => RequestErrorKind::PeerNeedsUpdate,
                ExpectedRequestError::Remote { kind, .. } => *kind,
                ExpectedRequestError::UnexpectedResponse { .. } => RequestErrorKind::Internal,
                ExpectedRequestError::StaleQueueRevision { .. } => RequestErrorKind::Conflict,
                ExpectedRequestError::LibraryTaskAlreadyRunning { .. } => {
                    RequestErrorKind::Conflict
                }
                ExpectedRequestError::HandoffSuperseded => RequestErrorKind::Conflict,
                ExpectedRequestError::PlayerCommandSuperseded => RequestErrorKind::Conflict,
                ExpectedRequestError::PlayerOutputUnavailable
                | ExpectedRequestError::PlayerSourceUnavailable
                | ExpectedRequestError::PlayerCommandTimedOut => RequestErrorKind::Unavailable,
                ExpectedRequestError::PlayerCommandCannotApply => RequestErrorKind::InvalidInput,
                ExpectedRequestError::CastNotPossible { state, .. } => match state {
                    CastLibraryState::NotHeld | CastLibraryState::AccessRevoked => {
                        RequestErrorKind::PermissionDenied
                    }
                    CastLibraryState::Unreachable => RequestErrorKind::Unavailable,
                    CastLibraryState::Ready => RequestErrorKind::Internal,
                },
                ExpectedRequestError::LicenseInvalid => RequestErrorKind::InvalidInput,
                ExpectedRequestError::LibraryRegistrationFailed { kind } => {
                    registration_failure_kind(*kind)
                }
                ExpectedRequestError::RemoteOpenFailed { kind } => match kind {
                    LibraryOpenErrorKind::RemoteUntrusted
                    | LibraryOpenErrorKind::RemoteNotAuthorized => {
                        RequestErrorKind::PermissionDenied
                    }
                    LibraryOpenErrorKind::RemoteUnreachable
                    | LibraryOpenErrorKind::RemoteProtocol => RequestErrorKind::Unavailable,
                },
                ExpectedRequestError::LibraryOpenFailed { kind } => open_failure_kind(*kind),
                ExpectedRequestError::LibraryRecordRemovalFailed { kind } => match kind {
                    RemoveLibraryRecordFailureKind::RecordNotFound => RequestErrorKind::NotFound,
                    RemoveLibraryRecordFailureKind::RecordIsOpen => RequestErrorKind::Conflict,
                },
                ExpectedRequestError::ScrobbleAuthentication { failure } => match failure {
                    ScrobbleAuthenticationFailure::CredentialsRejected => {
                        RequestErrorKind::PermissionDenied
                    }
                    ScrobbleAuthenticationFailure::ServiceUnreachable => {
                        RequestErrorKind::Unavailable
                    }
                    ScrobbleAuthenticationFailure::LastFmApiCredentialsMissing { .. }
                    | ScrobbleAuthenticationFailure::LastFmApiCredentialsInvalid => {
                        RequestErrorKind::InvalidInput
                    }
                    ScrobbleAuthenticationFailure::Other { .. } => RequestErrorKind::Internal,
                },
            };
        }
    }
    RequestErrorKind::Internal
}

/// Preserve a typed detail for expected failures through remote request hops.
pub fn request_failure_cause(error: &Error) -> Option<RequestFailureCause> {
    if sync_orm::is_storage_busy(error) {
        return Some(RequestFailureCause::StorageBusy);
    }
    for cause in error.chain() {
        if let Some(expected) = cause.downcast_ref::<ExpectedRequestError>() {
            match expected {
                ExpectedRequestError::CastNotPossible { library_id, state } => {
                    return Some(RequestFailureCause::CastNotPossible {
                        library_id: library_id.clone(),
                        state: *state,
                    });
                }
                ExpectedRequestError::LibraryTaskAlreadyRunning { task } => {
                    return Some(RequestFailureCause::LibraryTaskAlreadyRunning { task: *task });
                }
                ExpectedRequestError::Reconnecting { alias } => {
                    return Some(RequestFailureCause::Reconnecting {
                        alias: alias.clone(),
                    });
                }
                ExpectedRequestError::Remote { cause, .. } => return cause.clone(),
                ExpectedRequestError::CertificateMismatch => {
                    return Some(RequestFailureCause::CertificateMismatch);
                }
                ExpectedRequestError::PublicKeyMismatch => {
                    return Some(RequestFailureCause::PublicKeyMismatch);
                }
                ExpectedRequestError::RegistrationConflict => {
                    return Some(RequestFailureCause::RegistrationConflict);
                }
                ExpectedRequestError::PeerNeedsUpdate {
                    update_side,
                    peer_alias,
                    local_version,
                    peer_version,
                } => {
                    return Some(RequestFailureCause::PeerNeedsUpdate {
                        update_side: *update_side,
                        peer_alias: peer_alias.clone(),
                        local_version: local_version.to_string(),
                        peer_version: peer_version.map(|version| version.to_string()),
                    });
                }
                ExpectedRequestError::SubsonicNotAuthorized => {
                    return Some(RequestFailureCause::SubsonicNotAuthorized);
                }
                ExpectedRequestError::LicenseInvalid => {
                    return Some(RequestFailureCause::LicenseInvalid);
                }
                ExpectedRequestError::LibraryRegistrationFailed { kind } => {
                    return Some(RequestFailureCause::LibraryRegistrationFailed { kind: *kind });
                }
                ExpectedRequestError::RemoteOpenFailed { kind } => {
                    return Some(RequestFailureCause::RemoteOpenFailed { kind: *kind });
                }
                ExpectedRequestError::LibraryOpenFailed { kind } => {
                    return Some(RequestFailureCause::LibraryOpenFailed { kind: *kind });
                }
                ExpectedRequestError::LibraryRecordRemovalFailed { kind } => {
                    return Some(RequestFailureCause::LibraryRecordRemovalFailed { kind: *kind });
                }
                ExpectedRequestError::ScrobbleAuthentication { failure } => {
                    return Some(RequestFailureCause::ScrobbleAuthentication {
                        failure: failure.clone(),
                    });
                }
                _ => {}
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Error, anyhow};
    use database::error::ExpectedError;

    use super::{ExpectedRequestError, classify, request_failure_cause};
    use crate::messages::{
        CastLibraryState, LibraryTaskKind, RequestErrorKind, RequestFailureCause,
    };
    use sea_orm::{
        ConnectionTrait, SqlxSqliteConnector,
        sqlx::{SqlitePool, sqlite::SqliteConnectOptions},
    };
    use std::time::Duration;

    #[test]
    fn database_not_found_survives_nested_context() {
        let error = Err::<(), _>(ExpectedError::NotFound { what: "playlist" })
            .context("inner")
            .context("outer")
            .unwrap_err();
        assert_eq!(classify(&error), RequestErrorKind::NotFound);
    }

    #[test]
    fn other_expected_errors_survive_context() {
        let errors = [
            (
                Error::new(ExpectedError::InvalidInput {
                    detail: "invalid catalog input".into(),
                }),
                RequestErrorKind::InvalidInput,
            ),
            (
                Error::new(ExpectedRequestError::invalid_input("invalid request")),
                RequestErrorKind::InvalidInput,
            ),
            (
                Error::new(ExpectedRequestError::Unavailable {
                    what: "playlist store",
                }),
                RequestErrorKind::Unavailable,
            ),
        ];
        for (error, kind) in errors {
            assert_eq!(classify(&error.context("outer")), kind);
        }
    }

    #[test]
    fn untyped_error_is_internal() {
        assert_eq!(classify(&anyhow!("boom")), RequestErrorKind::Internal);
    }

    #[test]
    fn stale_queue_revision_is_a_conflict_after_context() {
        let error = Error::new(ExpectedRequestError::StaleQueueRevision {
            expected: 7,
            current: 8,
        })
        .context("move playlist item");
        assert_eq!(classify(&error), RequestErrorKind::Conflict);
    }

    #[test]
    fn superseded_handoff_is_a_conflict_after_context() {
        let error = Error::new(ExpectedRequestError::HandoffSuperseded).context("handoff");
        assert_eq!(classify(&error), RequestErrorKind::Conflict);
    }

    #[test]
    fn running_library_task_is_a_typed_conflict_after_context() {
        let error = Error::new(ExpectedRequestError::LibraryTaskAlreadyRunning {
            task: LibraryTaskKind::Scan,
        })
        .context("AnalyzeAudioLibraryRequest");
        assert_eq!(classify(&error), RequestErrorKind::Conflict);
        assert_eq!(
            request_failure_cause(&error),
            Some(RequestFailureCause::LibraryTaskAlreadyRunning {
                task: LibraryTaskKind::Scan,
            })
        );
        assert!(format!("{error:#}").contains("scan is running"));
    }

    #[test]
    fn remote_request_error_preserves_renderer_kind() {
        let error = Error::new(ExpectedRequestError::Remote {
            kind: RequestErrorKind::InvalidInput,
            detail: "invalid cast item".into(),
            cause: None,
        })
        .context("LoadResolvedItemsRequest");
        assert_eq!(classify(&error), RequestErrorKind::InvalidInput);
    }

    #[test]
    fn request_failure_cause_preserves_typed_cast_failure_through_remote_hops() {
        let expected = RequestFailureCause::CastNotPossible {
            library_id: "library-h".into(),
            state: CastLibraryState::NotHeld,
        };
        assert_eq!(request_failure_cause(&anyhow!("plain")), None);

        let cast_error = Error::new(ExpectedRequestError::CastNotPossible {
            library_id: "library-h".into(),
            state: CastLibraryState::NotHeld,
        })
        .context("load cast items");
        assert_eq!(request_failure_cause(&cast_error), Some(expected.clone()));

        let remote_error = Error::new(ExpectedRequestError::Remote {
            kind: RequestErrorKind::PermissionDenied,
            detail: "renderer rejected library".into(),
            cause: Some(expected.clone()),
        })
        .context("forward cast load");
        assert_eq!(request_failure_cause(&remote_error), Some(expected));
    }

    #[tokio::test]
    async fn sqlite_write_lock_is_unavailable_with_typed_cause() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("busy.db");
        let lock_pool = SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .busy_timeout(Duration::from_secs(1)),
        )
        .await
        .unwrap();
        let test_pool = SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .busy_timeout(Duration::from_millis(1)),
        )
        .await
        .unwrap();
        let lock = SqlxSqliteConnector::from_sqlx_sqlite_pool(lock_pool);
        let under_test = SqlxSqliteConnector::from_sqlx_sqlite_pool(test_pool);
        lock.execute_unprepared("CREATE TABLE writes (value INTEGER)")
            .await
            .unwrap();
        lock.execute_unprepared("BEGIN IMMEDIATE").await.unwrap();

        let error = under_test
            .execute_unprepared("INSERT INTO writes VALUES (1)")
            .await
            .unwrap_err();
        let error = Error::new(error).context("write request");
        assert_eq!(classify(&error), RequestErrorKind::Unavailable);
        assert_eq!(
            request_failure_cause(&error),
            Some(RequestFailureCause::StorageBusy)
        );
    }
}
