use std::time::UNIX_EPOCH;
use std::{collections::HashMap, time::SystemTime};

use anyhow::{bail, Result};
use async_trait::async_trait;
use md5;
use reqwest::{Client, Response};

use crate::{AuthResponse, ScrobblingClient, ScrobblingTrack};

#[derive(Clone)]
pub struct LastFmClient {
    api_key: String,
    api_secret: String,
    pub session_key: Option<String>,
    client: Client,
    base_url: String,
}

impl LastFmClient {
    pub fn new(api_key: String, api_secret: String) -> Result<Self> {
        let client = Client::builder().build()?;
        Ok(LastFmClient {
            api_key,
            api_secret,
            session_key: None,
            client,
            base_url: "https://ws.audioscrobbler.com/2.0/".to_string(),
        })
    }

    fn generate_signature(&self, params: &mut HashMap<String, String>) -> String {
        let mut sig_str = String::new();
        let mut sorted_keys: Vec<_> = params.keys().collect();
        sorted_keys.sort();

        for key in sorted_keys {
            sig_str.push_str(key);
            sig_str.push_str(&params[key]);
        }
        sig_str.push_str(&self.api_secret);

        format!("{:x}", md5::compute(sig_str.as_bytes()))
    }
}

#[async_trait]
impl ScrobblingClient for LastFmClient {
    async fn authenticate(&mut self, username: &str, password: &str) -> Result<()> {
        let mut params = HashMap::new();
        params.insert("method".to_string(), "auth.getMobileSession".to_string());
        params.insert("username".to_string(), username.to_string());
        params.insert("password".to_string(), password.to_string());
        params.insert("api_key".to_string(), self.api_key.clone());

        let api_sig = self.generate_signature(&mut params);
        params.insert("api_sig".to_string(), api_sig);

        let response = self
            .client
            .post(&self.base_url)
            .form(&params)
            .query(&[("format", "json")])
            .send()
            .await?;

        if response.status().is_success() {
            let auth_response: AuthResponse = response.json().await?;

            if auth_response.error.is_some() {
                bail!(
                    "Authentication failed: {}",
                    auth_response
                        .message
                        .unwrap_or_else(|| "Unknown error".to_string())
                );
            }

            if let Some(session) = auth_response.session {
                self.session_key = Some(session.key);
                Ok(())
            } else {
                bail!("Authentication failed: No session key returned");
            }
        } else {
            let auth_response = response.text().await?;
            bail!("Authentication failed: {}", auth_response);
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
        params.insert("api_key".to_string(), self.api_key.clone());
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

        let api_sig = self.generate_signature(&mut params);
        params.insert("api_sig".to_string(), api_sig);

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
        params.insert("api_key".to_string(), self.api_key.clone());
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

        let api_sig = self.generate_signature(&mut params);
        params.insert("api_sig".to_string(), api_sig);

        let response = self
            .client
            .post(&self.base_url)
            .form(&params)
            .query(&[("format", "json")])
            .send()
            .await?;

        Ok(response)
    }

    fn session_key(&self) -> Option<&str> {
        self.session_key.as_deref()
    }
}
