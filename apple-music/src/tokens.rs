//! Where the tokens come from.

use std::fmt;

use async_trait::async_trait;

use crate::error::Result;

/// Supplies the two tokens the Apple Music API needs.
///
/// Implementations decide how tokens are obtained (MusicKit, a signed
/// developer token from configuration, MusicKit JS, the Android SDK, ...) and
/// should cache them. The client calls with `refresh = true` only after the
/// server rejected the token it got last time.
#[async_trait]
pub trait TokenSource: Send + Sync {
    /// Returns the developer token (JWT) sent as `Authorization: Bearer`.
    async fn developer_token(&self, refresh: bool) -> Result<String>;

    /// Returns the Music User Token, or `None` when the user has not signed in.
    async fn user_token(&self, refresh: bool) -> Result<Option<String>>;
}

/// A [`TokenSource`] with fixed tokens. It cannot refresh anything.
#[derive(Clone)]
pub struct StaticTokens {
    developer: String,
    user: Option<String>,
}

impl StaticTokens {
    /// Creates a source from a developer token and an optional user token.
    pub fn new(developer: impl Into<String>, user: Option<String>) -> Self {
        Self {
            developer: developer.into(),
            user,
        }
    }
}

impl fmt::Debug for StaticTokens {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StaticTokens")
            .field("developer", &"[REDACTED]")
            .field("user", &self.user.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[async_trait]
impl TokenSource for StaticTokens {
    async fn developer_token(&self, _refresh: bool) -> Result<String> {
        Ok(self.developer.clone())
    }

    async fn user_token(&self, _refresh: bool) -> Result<Option<String>> {
        Ok(self.user.clone())
    }
}
