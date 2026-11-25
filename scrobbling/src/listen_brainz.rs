use anyhow::{Result, bail};
use async_trait::async_trait;
use reqwest::{Client, header::HeaderValue};
use serde_json::{Map, Value};
use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{ScrobblingAuthFailure, ScrobblingClient, ScrobblingTrack};

impl From<&ScrobblingTrack> for Value {
    fn from(track: &ScrobblingTrack) -> Self {
        let mut track_metadata: Map<String, Value> = Map::new();
        track_metadata.insert(
            "artist_name".to_string(),
            Value::String(track.artist.clone()),
        );
        track_metadata.insert("track_name".to_string(), Value::String(track.track.clone()));

        if let Some(album) = &track.album {
            track_metadata.insert("release_name".to_string(), Value::String(album.clone()));
        }

        let mut additional_info = Map::new();
        if let Some(duration) = track.duration {
            additional_info.insert(
                "duration".to_string(),
                Value::Number((duration.max(0.0).ceil() as u32).into()),
            );
        }

        additional_info.insert(
            "media_player".to_string(),
            Value::String("rune".to_string()),
        );

        track_metadata.insert(
            "additional_info".to_string(),
            Value::Object(additional_info),
        );

        Value::Object(track_metadata)
    }
}

#[derive(Clone)]
pub struct ListenBrainzClient {
    client: Client,
    base_url: String,
    pub session_key: Option<String>,
}

#[cfg(test)]
impl ListenBrainzClient {
    pub(crate) fn with_base_url(base_url: String) -> Self {
        let mut client = Self::new().expect("ListenBrainz client");
        client.base_url = base_url;
        client
    }
}

impl ListenBrainzClient {
    pub fn new() -> Result<Self> {
        let client = Client::builder().build()?;
        Ok(ListenBrainzClient {
            client,
            base_url: "https://api.listenbrainz.org".to_string(),
            session_key: None,
        })
    }

    async fn post_request(
        &self,
        endpoint: &str,
        body: &HashMap<&str, serde_json::Value>,
    ) -> Result<()> {
        if let Some(token) = &self.session_key {
            let response = self
                .client
                .post(format!("{}/{}", self.base_url, endpoint))
                .json(body)
                .header(
                    "Authorization",
                    HeaderValue::from_str(&format!("Token {token}"))?,
                )
                .send()
                .await?;

            if response.status().is_success() {
                Ok(())
            } else {
                bail!(
                    "ListenBrainz request failed with HTTP {}",
                    response.status()
                )
            }
        } else {
            bail!("Client is not authenticated.")
        }
    }
}

#[async_trait]
impl ScrobblingClient for ListenBrainzClient {
    async fn authenticate(&mut self, _username: &str, password: &str) -> Result<()> {
        let response = self
            .client
            .get(format!("{}/1/validate-token", self.base_url))
            .header(
                "Authorization",
                HeaderValue::from_str(&format!("Token {password}"))?,
            )
            .send()
            .await?;

        if response.status().is_success() {
            let json: Value = response.json().await?;
            if json["valid"].as_bool().unwrap_or(false) {
            } else {
                return Err(ScrobblingAuthFailure::CredentialsRejected {
                    detail: "ListenBrainz rejected the user token".into(),
                }
                .into());
            }
        } else {
            let status = response.status();
            if status.is_server_error() {
                return Err(ScrobblingAuthFailure::ServiceUnreachable {
                    detail: format!("ListenBrainz validation endpoint returned HTTP {status}"),
                }
                .into());
            }
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err(ScrobblingAuthFailure::CredentialsRejected {
                    detail: format!("ListenBrainz rejected the user token (HTTP {status})"),
                }
                .into());
            }
            bail!("ListenBrainz validation endpoint returned HTTP {status}")
        }

        self.session_key = Some(password.to_string());
        Ok(())
    }

    async fn update_now_playing(&self, track: &ScrobblingTrack) -> Result<()> {
        let track_metadata: Value = track.into();

        let mut payload = Map::new();
        payload.insert("track_metadata".to_string(), track_metadata);

        let mut body = HashMap::new();
        body.insert("listen_type", Value::String("playing_now".to_string()));
        body.insert("payload", Value::Array(vec![Value::Object(payload)]));

        self.post_request("1/submit-listens", &body).await
    }

    async fn scrobble(&self, track: &ScrobblingTrack) -> Result<()> {
        let track_metadata: Value = track.into();

        let mut payload = Map::new();
        let timestamp = track.timestamp.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
        });
        payload.insert("listened_at".to_string(), Value::Number(timestamp.into()));
        payload.insert("track_metadata".to_string(), track_metadata);

        let mut body = HashMap::new();
        body.insert("listen_type", Value::String("single".to_string()));
        body.insert("payload", Value::Array(vec![Value::Object(payload)]));

        self.post_request("1/submit-listens", &body).await
    }

    fn session_key(&self) -> Option<&str> {
        self.session_key.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::ListenBrainzClient;
    use crate::{
        ScrobblingClient,
        test_support::{sample_track, spawn_sequence_server},
    };

    #[tokio::test]
    async fn update_now_playing_and_scrobble_report_http_outcome() {
        let (base_url, request_count, server) = spawn_sequence_server(vec![
            ("200 OK", r#"{"status":"ok"}"#.to_string()),
            ("200 OK", r#"{"status":"ok"}"#.to_string()),
            ("500 Internal Server Error", "upstream error".to_string()),
            ("500 Internal Server Error", "upstream error".to_string()),
        ])
        .await;

        let mut client = ListenBrainzClient::new().unwrap();
        client.base_url = base_url.trim_end_matches('/').to_string();
        client.session_key = Some("synthetic-token".into());
        let track = sample_track();

        client.update_now_playing(&track).await.unwrap();
        client.scrobble(&track).await.unwrap();
        let now_err = client.update_now_playing(&track).await.unwrap_err();
        let scrobble_err = client.scrobble(&track).await.unwrap_err();
        server.await.unwrap();

        assert_eq!(request_count.load(Ordering::SeqCst), 4);
        assert!(format!("{now_err:#}").contains("500"));
        assert!(format!("{scrobble_err:#}").contains("500"));
    }
}
