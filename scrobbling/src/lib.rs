pub mod last_fm;
pub mod libre_fm;

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Response;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct AuthResponse {
    session: Option<SessionInfo>,
    error: Option<i32>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionInfo {
    key: String,
}

#[derive(Debug, Serialize)]
pub struct ScrobblingTrack {
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub duration: Option<u32>,
    pub timestamp: Option<u64>,
}

#[async_trait]
pub trait ScrobblingClient {
    async fn authenticate(&mut self, username: &str, password: &str) -> Result<()>;
    async fn update_now_playing(&self, track: &ScrobblingTrack) -> Result<Response>;
    async fn scrobble(&self, track: &ScrobblingTrack) -> Result<Response>;
}
