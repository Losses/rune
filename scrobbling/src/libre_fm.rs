use std::time::UNIX_EPOCH;
use std::{collections::HashMap, time::SystemTime};

use anyhow::{Result, bail};
use async_trait::async_trait;
use reqwest::Client;

use crate::{
    AuthResponse, ScrobblingAuthFailure, ScrobblingClient, ScrobblingTrack,
    send_audioscrobbler_request,
};

#[derive(Clone)]
pub struct LibreFmClient {
    pub session_key: Option<String>,
    client: Client,
    pub(crate) base_url: String,
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
        // Compute MD5 of the password
        let password_hash = format!("{:x}", md5::compute(password));

        // Compute the authentication token
        let authtoken = format!(
            "{:x}",
            md5::compute(format!("{}{}", username.to_lowercase(), password_hash))
        );

        let mut params = HashMap::new();
        params.insert("method".to_string(), "auth.getMobileSession".to_string());
        params.insert("username".to_string(), username.to_string());
        params.insert("password".to_string(), password.to_string());
        params.insert("authtoken".to_string(), authtoken);
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

            if let Some(code) = auth_response.error {
                if code == 4 {
                    return Err(ScrobblingAuthFailure::CredentialsRejected {
                        detail: "Libre.fm rejected the account credentials (API error 4)".into(),
                    }
                    .into());
                }
                bail!("Libre.fm authentication failed (API error {code})");
            }

            if let Some(session) = auth_response.session {
                self.session_key = Some(session.key);
                Ok(())
            } else {
                bail!("Authentication failed: No session key returned");
            }
        } else {
            let status = response.status();
            if status.is_server_error() {
                return Err(ScrobblingAuthFailure::ServiceUnreachable {
                    detail: format!("Libre.fm authentication endpoint returned HTTP {status}"),
                }
                .into());
            }
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err(ScrobblingAuthFailure::CredentialsRejected {
                    detail: format!("Libre.fm rejected the account credentials (HTTP {status})"),
                }
                .into());
            }
            bail!("Libre.fm authentication endpoint returned HTTP {status}")
        }
    }

    async fn update_now_playing(&self, track: &ScrobblingTrack) -> Result<()> {
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
            params.insert(
                "duration".to_string(),
                (duration.max(0.0).ceil() as u32).to_string(),
            );
        }

        send_audioscrobbler_request(
            &self.client,
            &self.base_url,
            &params,
            "Libre.fm",
            "now playing",
        )
        .await
    }

    async fn scrobble(&self, track: &ScrobblingTrack) -> Result<()> {
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

        send_audioscrobbler_request(
            &self.client,
            &self.base_url,
            &params,
            "Libre.fm",
            "scrobble",
        )
        .await
    }

    fn session_key(&self) -> Option<&str> {
        self.session_key.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::LibreFmClient;
    use crate::{
        ScrobblingClient,
        test_support::{sample_track, spawn_sequence_server},
    };

    #[tokio::test]
    async fn update_now_playing_and_scrobble_reject_http_500_without_leaking_body() {
        let body = "upstream unavailable leaked-session-token".to_string();
        let (base_url, _, server) = spawn_sequence_server(vec![
            ("500 Internal Server Error", body.clone()),
            ("500 Internal Server Error", body),
        ])
        .await;

        let mut client = LibreFmClient::new().unwrap();
        client.base_url = base_url;
        client.session_key = Some("synthetic-session".into());
        let track = sample_track();

        let now_err = client.update_now_playing(&track).await.unwrap_err();
        let scrobble_err = client.scrobble(&track).await.unwrap_err();
        server.await.unwrap();

        assert!(!format!("{now_err:#}").contains("leaked-session-token"));
        assert!(!format!("{scrobble_err:#}").contains("leaked-session-token"));
        assert!(format!("{now_err:#}").contains("500"));
        assert!(format!("{scrobble_err:#}").contains("500"));
    }

    #[tokio::test]
    async fn update_now_playing_and_scrobble_reject_http_200_api_error_without_leaking_message() {
        let body = r#"{"error":9,"message":"Invalid session key secret-echo"}"#.to_string();
        let (base_url, _, server) =
            spawn_sequence_server(vec![("200 OK", body.clone()), ("200 OK", body)]).await;

        let mut client = LibreFmClient::new().unwrap();
        client.base_url = base_url;
        client.session_key = Some("synthetic-session".into());
        let track = sample_track();

        let now_err = client.update_now_playing(&track).await.unwrap_err();
        let scrobble_err = client.scrobble(&track).await.unwrap_err();
        server.await.unwrap();

        assert!(!format!("{now_err:#}").contains("secret-echo"));
        assert!(!format!("{scrobble_err:#}").contains("secret-echo"));
        assert!(format!("{now_err:#}").contains('9'));
        assert!(format!("{scrobble_err:#}").contains('9'));
    }

    #[tokio::test]
    async fn update_now_playing_and_scrobble_accept_valid_json_responses() {
        let now_body = r##"{"nowplaying":{"track":{"corrected":"0","#text":"Synthetic track"}}}"##;
        let scrobble_body = r#"{"scrobbles":{"@attr":{"accepted":1,"ignored":0}}}"#;
        let (base_url, request_count, server) = spawn_sequence_server(vec![
            ("200 OK", now_body.to_string()),
            ("200 OK", scrobble_body.to_string()),
        ])
        .await;

        let mut client = LibreFmClient::new().unwrap();
        client.base_url = base_url;
        client.session_key = Some("synthetic-session".into());
        let track = sample_track();

        client.update_now_playing(&track).await.unwrap();
        client.scrobble(&track).await.unwrap();
        server.await.unwrap();

        assert_eq!(request_count.load(Ordering::SeqCst), 2);
    }
}
