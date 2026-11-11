use std::time::UNIX_EPOCH;
use std::{collections::HashMap, time::SystemTime};

use anyhow::{Result, bail};
use async_trait::async_trait;
use md5;
use reqwest::{Client, Response};

use crate::{AuthResponse, ScrobblingAuthFailure, ScrobblingClient, ScrobblingTrack};

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

            if let Some(code) = auth_response.error {
                match code {
                    4 => {
                        return Err(ScrobblingAuthFailure::CredentialsRejected {
                            detail: "Last.fm rejected the account credentials (API error 4)".into(),
                        }
                        .into());
                    }
                    10 | 13 | 26 => {
                        return Err(ScrobblingAuthFailure::LastFmApiCredentialsInvalid {
                            detail: format!(
                                "Last.fm rejected the API credentials (API error {code})"
                            ),
                        }
                        .into());
                    }
                    _ => bail!("Last.fm authentication failed (API error {code})"),
                }
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
                    detail: format!("Last.fm authentication endpoint returned HTTP {status}"),
                }
                .into());
            }
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err(ScrobblingAuthFailure::CredentialsRejected {
                    detail: format!("Last.fm rejected the account credentials (HTTP {status})"),
                }
                .into());
            }
            bail!("Last.fm authentication endpoint returned HTTP {status}");
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

#[cfg(test)]
mod tests {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::LastFmClient;
    use crate::{ScrobblingAuthFailure, ScrobblingClient};

    #[tokio::test]
    async fn authentication_response_body_cannot_echo_credentials_into_errors() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let password = "password-secret-value";
        let api_secret = "api-secret-value";
        let response_body =
            format!(r#"{{"error":4,"message":"echoed {password} and {api_secret}"}}"#);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 2048];
            let _ = stream.read(&mut request).await;
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let mut client = LastFmClient::new("api-key-value".into(), api_secret.into()).unwrap();
        client.base_url = format!("http://{address}/");
        let error = client.authenticate("listener", password).await.unwrap_err();
        server.await.unwrap();

        assert!(matches!(
            error.downcast_ref::<ScrobblingAuthFailure>(),
            Some(ScrobblingAuthFailure::CredentialsRejected { .. })
        ));
        assert!(!format!("{error:#}").contains(password));
        assert!(!format!("{error:#}").contains(api_secret));
        assert!(!format!("{error:#}").contains("api-key-value"));
    }
}
