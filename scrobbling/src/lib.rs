pub mod last_fm;
pub mod libre_fm;
pub mod listen_brainz;
pub mod manager;

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Response;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LastFmApiCredentialField {
    ApiKey,
    ApiSecret,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScrobblingAuthFailure {
    CredentialsRejected { detail: String },
    ServiceUnreachable { detail: String },
    LastFmApiCredentialsMissing(LastFmApiCredentialField),
    LastFmApiCredentialsInvalid { detail: String },
    Other { detail: String },
}

impl std::fmt::Display for ScrobblingAuthFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CredentialsRejected { detail }
            | Self::ServiceUnreachable { detail }
            | Self::LastFmApiCredentialsInvalid { detail }
            | Self::Other { detail } => f.write_str(detail),
            Self::LastFmApiCredentialsMissing(LastFmApiCredentialField::ApiKey) => {
                f.write_str("Last.fm API key is missing")
            }
            Self::LastFmApiCredentialsMissing(LastFmApiCredentialField::ApiSecret) => {
                f.write_str("Last.fm API secret is missing")
            }
        }
    }
}

impl std::error::Error for ScrobblingAuthFailure {}

#[derive(Debug, Deserialize)]
struct AuthResponse {
    session: Option<SessionInfo>,
    error: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct SessionInfo {
    key: String,
}

#[derive(Debug, Serialize)]
pub struct ScrobblingTrack {
    /// Local routing context; never sent to a scrobbling service.
    #[serde(skip)]
    pub library_id: Option<String>,
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub duration: Option<u32>,
    pub timestamp: Option<u64>,
}

#[async_trait]
pub trait ScrobblingClient: Send {
    async fn authenticate(&mut self, username: &str, password: &str) -> Result<()>;
    async fn update_now_playing(&self, track: &ScrobblingTrack) -> Result<Response>;
    async fn scrobble(&self, track: &ScrobblingTrack) -> Result<Response>;
    fn session_key(&self) -> Option<&str>;
}
