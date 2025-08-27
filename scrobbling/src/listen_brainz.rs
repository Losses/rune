use anyhow::{Result, bail};
use async_trait::async_trait;
use reqwest::{Client, Response, header::HeaderValue};
use serde_json::{Map, Value};
use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{ScrobblingClient, ScrobblingTrack};

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
            additional_info.insert("duration".to_string(), Value::Number(duration.into()));
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
    ) -> Result<Response> {
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
                Ok(response)
            } else {
                bail!("Request failed: {:?}", response.text().await?)
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
                bail!(
                    "Token invalid: {:?}",
                    json["message"].as_str().unwrap_or("Unknown error")
                )
            }
        } else {
            bail!("Failed to validate token: {:?}", response.text().await?)
        }

        self.session_key = Some(password.to_string());
        Ok(())
    }

    async fn update_now_playing(&self, track: &ScrobblingTrack) -> Result<Response> {
        let track_metadata: Value = track.into();

        let mut payload = Map::new();
        payload.insert("track_metadata".to_string(), track_metadata);

        let mut body = HashMap::new();
        body.insert(
            "listen_type",
            serde_json::Value::String("playing_now".to_string()),
        );
        body.insert("payload", Value::Array(vec![Value::Object(payload)]));

        self.post_request("1/submit-listens", &body).await
    }

    async fn scrobble(&self, track: &ScrobblingTrack) -> Result<Response> {
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
