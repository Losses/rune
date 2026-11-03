use thiserror::Error;

/// Expected failures caused by a request or a missing catalog object.
#[derive(Debug, Error)]
pub enum ExpectedError {
    #[error("{what} not found")]
    NotFound { what: &'static str },
    #[error("{detail}")]
    InvalidInput { detail: String },
}
