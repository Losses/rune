use anyhow::Error;
use database::error::ExpectedError;
use thiserror::Error as ThisError;

use crate::access_policy::Denial;
use crate::messages::{CastLibraryState, RequestErrorKind, RequestFailureCause};

/// Expected failures at the request boundary.
#[derive(Debug, ThisError)]
pub enum ExpectedRequestError {
    #[error("{detail}")]
    InvalidInput { detail: String },
    #[error("{what} unavailable")]
    Unavailable { what: &'static str },
    #[error("Renderer request failed: {detail}")]
    Remote {
        kind: RequestErrorKind,
        detail: String,
        cause: Option<RequestFailureCause>,
    },
    #[error("Queue changed: expected revision {expected}, current revision {current}")]
    StaleQueueRevision { expected: u64, current: u64 },
    #[error("Cast is not possible for library {library_id}: {state:?}")]
    CastNotPossible {
        library_id: String,
        state: CastLibraryState,
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

/// Classify a request failure without relying on its diagnostic text.
pub fn classify(error: &Error) -> RequestErrorKind {
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
                ExpectedRequestError::Unavailable { .. } => RequestErrorKind::Unavailable,
                ExpectedRequestError::Remote { kind, .. } => *kind,
                ExpectedRequestError::StaleQueueRevision { .. } => RequestErrorKind::Conflict,
                ExpectedRequestError::CastNotPossible { state, .. } => match state {
                    CastLibraryState::NotHeld | CastLibraryState::AccessRevoked => {
                        RequestErrorKind::PermissionDenied
                    }
                    CastLibraryState::Unreachable => RequestErrorKind::Unavailable,
                    CastLibraryState::Ready => RequestErrorKind::Internal,
                },
            };
        }
    }
    RequestErrorKind::Internal
}

/// Preserve a typed detail for expected failures through remote request hops.
pub fn request_failure_cause(error: &Error) -> Option<RequestFailureCause> {
    for cause in error.chain() {
        if let Some(expected) = cause.downcast_ref::<ExpectedRequestError>() {
            match expected {
                ExpectedRequestError::CastNotPossible { library_id, state } => {
                    return Some(RequestFailureCause::CastNotPossible {
                        library_id: library_id.clone(),
                        state: *state,
                    });
                }
                ExpectedRequestError::Remote { cause, .. } => return cause.clone(),
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
    use crate::messages::{CastLibraryState, RequestErrorKind, RequestFailureCause};

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
    fn remote_request_error_preserves_renderer_kind() {
        let error = Error::new(ExpectedRequestError::Remote {
            kind: RequestErrorKind::InvalidInput,
            detail: "invalid cast item".into(),
            cause: None,
        })
        .context("CastLoadRequest");
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
}
