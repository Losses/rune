//! Error type for the Apple Music client.

use std::result;
use std::time::Duration;

use serde::Deserialize;

/// Result alias used throughout the crate.
pub type Result<T> = result::Result<T, Error>;

/// One entry of the `errors` array in an Apple Music error response.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ApiError {
    /// Unique identifier of this error instance.
    #[serde(default)]
    pub id: Option<String>,
    /// Short, human readable title.
    #[serde(default)]
    pub title: Option<String>,
    /// Longer explanation of the problem.
    #[serde(default)]
    pub detail: Option<String>,
    /// HTTP status as a string, as sent by the server.
    #[serde(default)]
    pub status: Option<String>,
    /// Apple's error code, for example `NOT_FOUND`.
    #[serde(default)]
    pub code: Option<String>,
}

/// Errors returned by [`crate::AppleMusicClient`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The developer token (or the user token) was rejected even after both
    /// were refreshed once.
    #[error("unauthorized: the developer token or the user token was rejected")]
    Unauthorized,
    /// The server answered 403: the user token is invalid, or the account has
    /// no Apple Music subscription or has not authorised this app.
    #[error("forbidden: no Apple Music subscription, or the app is not authorised")]
    Forbidden,
    /// The server answered 404 with a "client not found" style body. This
    /// happens for a while after an app identifier was registered.
    #[error("the Apple Music client is not registered (yet)")]
    ClientNotRegistered,
    /// The requested resource does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// Still rate limited (429) after all retries.
    #[error("rate limited by the server (retry after {retry_after:?})")]
    RateLimited {
        /// Value of the last `Retry-After` header, when it was a number of seconds.
        retry_after: Option<Duration>,
    },
    /// The server kept answering with a 5xx status after all retries.
    #[error("server error {status}")]
    Server {
        /// The last HTTP status.
        status: u16,
        /// Errors parsed from the body, when present.
        errors: Vec<ApiError>,
    },
    /// Any other non-success status.
    #[error("api error {status}: {}", summarize(.errors))]
    Api {
        /// The HTTP status.
        status: u16,
        /// Errors parsed from the body, when present.
        errors: Vec<ApiError>,
    },
    /// A `/v1/me/*` request needs a Music User Token but the token source has none.
    #[error("a music user token is required for this request")]
    MissingUserToken,
    /// The token source failed to produce a token.
    #[error("token source failed: {0}")]
    Token(String),
    /// The HTTP request could not be completed.
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    /// The response body was not the expected JSON.
    #[error("could not decode the response: {0}")]
    Decode(#[from] serde_json::Error),
}

impl Error {
    /// Another error that says the same, for callers that share one failure. A transport or
    /// decode error cannot be copied, so it is kept as its text.
    pub(crate) fn echo(&self) -> Self {
        let as_text = |message: String| {
            vec![ApiError {
                title: Some(message),
                ..ApiError::default()
            }]
        };
        match self {
            Self::Unauthorized => Self::Unauthorized,
            Self::Forbidden => Self::Forbidden,
            Self::ClientNotRegistered => Self::ClientNotRegistered,
            Self::NotFound(message) => Self::NotFound(message.clone()),
            Self::RateLimited { retry_after } => Self::RateLimited {
                retry_after: *retry_after,
            },
            Self::Server { status, errors } => Self::Server {
                status: *status,
                errors: errors.clone(),
            },
            Self::Api { status, errors } => Self::Api {
                status: *status,
                errors: errors.clone(),
            },
            Self::MissingUserToken => Self::MissingUserToken,
            Self::Token(message) => Self::Token(message.clone()),
            // Mapped like the transport error it stands for.
            Self::Http(e) => Self::Server {
                status: 0,
                errors: as_text(e.to_string()),
            },
            Self::Decode(e) => Self::Api {
                status: 0,
                errors: as_text(e.to_string()),
            },
        }
    }
}

fn summarize(errors: &[ApiError]) -> String {
    errors
        .iter()
        .map(|e| {
            e.detail
                .as_deref()
                .or(e.title.as_deref())
                .unwrap_or("unknown error")
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Parses the `errors` array out of a response body, ignoring anything unparsable.
pub(crate) fn parse_errors(body: &[u8]) -> Vec<ApiError> {
    #[derive(Deserialize)]
    struct Body {
        #[serde(default)]
        errors: Vec<ApiError>,
    }
    serde_json::from_slice::<Body>(body)
        .map(|b| b.errors)
        .unwrap_or_default()
}

/// Whether a 404 body looks like Apple's answer for an unregistered client.
pub(crate) fn is_client_not_registered(errors: &[ApiError]) -> bool {
    errors.iter().any(|e| {
        [e.title.as_deref(), e.detail.as_deref()]
            .into_iter()
            .flatten()
            .any(|t| {
                let t = t.to_ascii_lowercase();
                t.contains("client not found") || t.contains("client not registered")
            })
    })
}
