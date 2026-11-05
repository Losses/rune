use anyhow::Error;
use database::error::ExpectedError;
use thiserror::Error as ThisError;

use crate::access_policy::Denial;
use crate::messages::RequestErrorKind;

/// Expected failures at the request boundary.
#[derive(Debug, ThisError)]
pub enum ExpectedRequestError {
    #[error("{detail}")]
    InvalidInput { detail: String },
    #[error("{what} unavailable")]
    Unavailable { what: &'static str },
    #[error("Queue changed: expected revision {expected}, current revision {current}")]
    StaleQueueRevision { expected: u64, current: u64 },
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
                ExpectedRequestError::StaleQueueRevision { .. } => RequestErrorKind::Conflict,
            };
        }
    }
    RequestErrorKind::Internal
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Error, anyhow};
    use database::error::ExpectedError;

    use super::{ExpectedRequestError, classify};
    use crate::messages::RequestErrorKind;

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
}
