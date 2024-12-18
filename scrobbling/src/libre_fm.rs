use std::time::UNIX_EPOCH;
use std::{collections::HashMap, time::SystemTime};

use anyhow::{bail, Result};
use async_trait::async_trait;
use reqwest::{Client, Response};

use crate::{AuthResponse, ScrobblingClient, ScrobblingTrack};

pub struct LibreFmClient {
    session_key: Option<String>,
    client: Client,
    base_url: String,
}

impl LibreFmClient {
    pub fn new() -> Result<Self> {
        let client = Client::builder().build()?;
        Ok(LibreFmClient {
            session_key: None,
            client,
            base_url: "https://libre.fm/2.0/".to_string(),
        })
    }
}

#[async_trait]
impl ScrobblingClient for LibreFmClient {
    async fn authenticate(&mut self, username: &str, password: &str) -> Result<()> {
        let mut params = HashMap::new();
        params.insert("method".to_string(), "auth.getMobileSession".to_string());
        params.insert("username".to_string(), username.to_string());
        params.insert("password".to_string(), password.to_string());
        params.insert("api_key".to_string(), "0".repeat(32));

        let response = self
            .client
            .post(&self.base_url)
            .form(&params)
            .query(&[("format", "json")])
            .send()
            .await?;

        if response.status().is_success() {
            let auth_response: AuthResponse = response.json().await?;
            self.session_key = Some(auth_response.session.key);
            Ok(())
        } else {
            bail!("Authentication failed")
        }
    }

    async fn update_now_playing(&self, track: &ScrobblingTrack) -> Result<Response> {
        if self.session_key.is_none() {
            bail!("Not authenticated");
        }

        let mut params = HashMap::new();
        params.insert("method".to_string(), "track.updateNowPlaying".to_string());
        params.insert("artist".to_string(), track.artist.clone());
        params.insert("track".to_string(), track.track.clone());
        params.insert("api_key".to_string(), "0".repeat(32));
        params.insert("sk".to_string(), self.session_key.clone().unwrap());

        if let Some(album) = &track.album {
            params.insert("album".to_string(), album.clone());
        }
        if let Some(album_artist) = &track.album_artist {
            params.insert("albumArtist".to_string(), album_artist.clone());
        }
        if let Some(duration) = track.duration {
            params.insert("duration".to_string(), duration.to_string());
        }

        let response = self
            .client
            .post(&self.base_url)
            .form(&params)
            .query(&[("format", "json")])
            .send()
            .await?;

        Ok(response)
    }

    async fn scrobble(&self, track: &ScrobblingTrack) -> Result<Response> {
        if self.session_key.is_none() {
            bail!("Not authenticated");
        }

        let mut params = HashMap::new();
        params.insert("method".to_string(), "track.scrobble".to_string());
        params.insert("artist".to_string(), track.artist.clone());
        params.insert("track".to_string(), track.track.clone());
        params.insert("api_key".to_string(), "0".repeat(32));
        params.insert("sk".to_string(), self.session_key.clone().unwrap());

        let timestamp = track.timestamp.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
        });
        params.insert("timestamp".to_string(), timestamp.to_string());

        if let Some(album) = &track.album {
            params.insert("album".to_string(), album.clone());
        }
        if let Some(album_artist) = &track.album_artist {
            params.insert("albumArtist".to_string(), album_artist.clone());
        }

        let response = self
            .client
            .post(&self.base_url)
            .form(&params)
            .query(&[("format", "json")])
            .send()
            .await?;

        Ok(response)
    }
}
