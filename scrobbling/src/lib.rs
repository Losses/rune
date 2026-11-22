pub mod last_fm;
pub mod libre_fm;
pub mod listen_brainz;
pub mod manager;

use std::collections::HashMap;

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::error;
use std::fmt;

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

impl fmt::Display for ScrobblingAuthFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

impl error::Error for ScrobblingAuthFailure {}

#[derive(Debug, Deserialize)]
struct AuthResponse {
    session: Option<SessionInfo>,
    error: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct SessionInfo {
    key: String,
}

#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    error: Option<Value>,
}

pub(crate) async fn send_audioscrobbler_request(
    client: &Client,
    base_url: &str,
    params: &HashMap<String, String>,
    service: &str,
    action: &str,
) -> Result<()> {
    let response = client
        .post(base_url)
        .form(params)
        .query(&[("format", "json")])
        .send()
        .await?;
    validate_audioscrobbler_response(response, service, action).await
}

pub(crate) async fn validate_audioscrobbler_response(
    response: Response,
    service: &str,
    action: &str,
) -> Result<()> {
    let status = response.status();
    if !status.is_success() {
        bail!("{service} {action} endpoint returned HTTP {status}");
    }

    let body = response.bytes().await?;

    let envelope: ApiErrorResponse = serde_json::from_slice(&body)
        .map_err(|_| anyhow!("{service} {action} returned an invalid JSON response"))?;
    if let Some(error) = envelope
        .error
        .filter(|value| !value.is_null() && *value != Value::Bool(false))
    {
        match error {
            Value::Number(code) => {
                bail!("{service} {action} failed (API error {code})");
            }
            Value::String(code) if !code.is_empty() && code.chars().all(|c| c.is_ascii_digit()) => {
                bail!("{service} {action} failed (API error {code})");
            }
            _ => {
                bail!("{service} {action} failed (API error)");
            }
        }
    }

    Ok(())
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
    /// Track length in seconds. Fractional values keep the 30-second boundary exact.
    pub duration: Option<f64>,
    pub timestamp: Option<u64>,
}

#[async_trait]
pub trait ScrobblingClient: Send {
    async fn authenticate(&mut self, username: &str, password: &str) -> Result<()>;
    async fn update_now_playing(&self, track: &ScrobblingTrack) -> Result<()>;
    async fn scrobble(&self, track: &ScrobblingTrack) -> Result<()>;
    fn session_key(&self) -> Option<&str>;
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::mpsc,
        task::JoinHandle,
    };

    use crate::ScrobblingTrack;

    pub fn sample_track() -> ScrobblingTrack {
        ScrobblingTrack {
            library_id: Some("lib-1".to_string()),
            artist: "Synthetic artist".to_string(),
            track: "Synthetic track".to_string(),
            album: None,
            album_artist: None,
            duration: Some(60.0),
            timestamp: Some(1_700_000_000),
        }
    }

    async fn read_http_request(stream: &mut TcpStream) {
        let mut buf = Vec::with_capacity(2048);
        let mut chunk = [0u8; 1024];
        let mut header_end = None;
        let mut content_length = 0usize;

        loop {
            let n = stream.read(&mut chunk).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if header_end.is_none()
                && let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n")
            {
                let end = pos + 4;
                header_end = Some(end);
                let headers = String::from_utf8_lossy(&buf[..pos]);
                for line in headers.lines() {
                    if let Some((name, value)) = line.split_once(':')
                        && name.trim().eq_ignore_ascii_case("content-length")
                    {
                        content_length = value.trim().parse().unwrap_or(0);
                    }
                }
            }
            if let Some(end) = header_end
                && buf.len() >= end + content_length
            {
                break;
            }
        }
    }

    pub async fn spawn_sequence_server(
        responses: Vec<(&'static str, String)>,
    ) -> (String, Arc<AtomicUsize>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let request_count = Arc::new(AtomicUsize::new(0));
        let count_for_task = Arc::clone(&request_count);

        let handle = tokio::spawn(async move {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                read_http_request(&mut stream).await;
                count_for_task.fetch_add(1, Ordering::SeqCst);

                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
                stream.shutdown().await.unwrap();
            }
        });

        (format!("http://{address}/"), request_count, handle)
    }

    /// A server that answers every request with the same response, so a repeated submission is
    /// counted rather than failing against a listener that already stopped.
    pub async fn spawn_persistent_server(
        response: (&'static str, String),
    ) -> (
        String,
        Arc<AtomicUsize>,
        mpsc::UnboundedReceiver<()>,
        JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let request_count = Arc::new(AtomicUsize::new(0));
        let count_for_task = Arc::clone(&request_count);
        let (request_tx, request_rx) = mpsc::unbounded_channel();

        let handle = tokio::spawn(async move {
            let (status, body) = response;
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                read_http_request(&mut stream).await;
                count_for_task.fetch_add(1, Ordering::SeqCst);
                let _ = request_tx.send(());

                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });

        (
            format!("http://{address}/"),
            request_count,
            request_rx,
            handle,
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        ScrobblingClient,
        last_fm::LastFmClient,
        libre_fm::LibreFmClient,
        test_support::{sample_track, spawn_sequence_server},
    };

    #[tokio::test]
    async fn reports_reject_http_200_non_json_and_http_403_without_leaking_body() {
        let (lastfm_url, _, lastfm_server) = spawn_sequence_server(vec![
            (
                "200 OK",
                "<html>captive portal secret-echo</html>".to_string(),
            ),
            ("403 Forbidden", "forbidden secret-echo".to_string()),
        ])
        .await;
        let (librefm_url, _, librefm_server) = spawn_sequence_server(vec![
            (
                "200 OK",
                "<html>captive portal secret-echo</html>".to_string(),
            ),
            ("403 Forbidden", "forbidden secret-echo".to_string()),
        ])
        .await;

        let mut lastfm =
            LastFmClient::new("synthetic-api-key".into(), "synthetic-api-secret".into()).unwrap();
        lastfm.base_url = lastfm_url;
        lastfm.session_key = Some("synthetic-session".into());

        let mut librefm = LibreFmClient::new().unwrap();
        librefm.base_url = librefm_url;
        librefm.session_key = Some("synthetic-session".into());

        let track = sample_track();
        let lastfm_now_err = lastfm.update_now_playing(&track).await.unwrap_err();
        let lastfm_scrobble_err = lastfm.scrobble(&track).await.unwrap_err();
        let librefm_now_err = librefm.update_now_playing(&track).await.unwrap_err();
        let librefm_scrobble_err = librefm.scrobble(&track).await.unwrap_err();

        lastfm_server.await.unwrap();
        librefm_server.await.unwrap();

        for err in [
            lastfm_now_err,
            lastfm_scrobble_err,
            librefm_now_err,
            librefm_scrobble_err,
        ] {
            assert!(!format!("{err:#}").contains("secret-echo"));
        }
    }
}
