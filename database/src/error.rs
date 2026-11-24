use thiserror::Error;

/// Expected failures caused by a request or a missing catalog object.
#[derive(Debug, Error)]
pub enum ExpectedError {
    #[error("{what} not found")]
    NotFound { what: &'static str },
    #[error("{detail}")]
    InvalidInput { detail: String },
}

/// A library database written by a newer version of Rune: it records
/// migrations newer than any this version has, so this version leaves it as
/// it is and does not open it.
#[derive(Debug, Error)]
#[error("the library database was written by a newer version of Rune ({})", .migrations.join(", "))]
pub struct NewerLibraryDatabase {
    pub migrations: Vec<String>,
}
