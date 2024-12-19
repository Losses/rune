use anyhow::Result;
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::sleep;

use crate::last_fm::LastFmClient;
use crate::libre_fm::LibreFmClient;
use crate::listen_brainz::ListenBrainzClient;
use crate::{ScrobblingClient, ScrobblingTrack};

pub struct ScrobblingManager {
    lastfm: Option<LastFmClient>,
    librefm: Option<LibreFmClient>,
    listenbrainz: Option<ListenBrainzClient>,
    max_retries: u32,
    retry_delay: Duration,
}

impl ScrobblingManager {
    pub fn new(max_retries: u32, retry_delay: Duration) -> Self {
        Self {
            lastfm: None,
            librefm: None,
            listenbrainz: None,
            max_retries,
            retry_delay,
        }
    }

    pub async fn authenticate(
        &mut self,
        service: &str,
        username: &str,
        password: &str,
        api_key: Option<String>,
        api_secret: Option<String>,
    ) -> Result<()> {
        let mut attempts = 0;

        loop {
            let result = match service {
                "lastfm" => {
                    let api_key = api_key
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("Last.fm requires API key"))?;
                    let api_secret = api_secret
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("Last.fm requires API secret"))?;
                    let mut client = LastFmClient::new(api_key, api_secret)?;
                    client.authenticate(username, password).await.map(|_| {
                        self.lastfm = Some(client);
                    })
                }
                "librefm" => {
                    let mut client = LibreFmClient::new()?;
                    client.authenticate(username, password).await.map(|_| {
                        self.librefm = Some(client);
                    })
                }
                "listenbrainz" => {
                    let mut client = ListenBrainzClient::new()?;
                    client.authenticate(username, password).await.map(|_| {
                        self.listenbrainz = Some(client);
                    })
                }
                _ => Err(anyhow::anyhow!("Unsupported service: {}", service)),
            };

            match result {
                Ok(_) => break,
                Err(e) => {
                    attempts += 1;
                    if attempts >= self.max_retries {
                        return Err(e);
                    }
                    sleep(self.retry_delay).await;
                }
            }
        }
        Ok(())
    }

    pub fn restore_session(&mut self, service: &str, session_key: String) -> Result<()> {
        match service {
            "lastfm" => {
                if let Some(client) = &mut self.lastfm {
                    client.session_key = Some(session_key);
                } else {
                    return Err(anyhow::anyhow!("Last.fm client not initialized"));
                }
            }
            "librefm" => {
                if let Some(client) = &mut self.librefm {
                    client.session_key = Some(session_key);
                } else {
                    return Err(anyhow::anyhow!("Libre.fm client not initialized"));
                }
            }
            "listenbrainz" => {
                if let Some(client) = &mut self.listenbrainz {
                    client.session_key = Some(session_key);
                } else {
                    return Err(anyhow::anyhow!("ListenBrainz client not initialized"));
                }
            }
            _ => return Err(anyhow::anyhow!("Unsupported service: {}", service)),
        }
        Ok(())
    }

    pub async fn scrobble_all(&mut self, track: &ScrobblingTrack) -> HashMap<String, Result<()>> {
        let mut results = HashMap::new();

        // Handle Last.fm
        if let Some(client) = &mut self.lastfm {
            if client.session_key.is_some() {
                let result =
                    Self::retry_scrobble(client, track, self.max_retries, self.retry_delay).await;
                results.insert("lastfm".to_string(), result);
            }
        }

        // Handle Libre.fm
        if let Some(client) = &mut self.librefm {
            if client.session_key.is_some() {
                let result =
                    Self::retry_scrobble(client, track, self.max_retries, self.retry_delay).await;
                results.insert("librefm".to_string(), result);
            }
        }

        // Handle ListenBrainz
        if let Some(client) = &mut self.listenbrainz {
            if client.session_key.is_some() {
                let result =
                    Self::retry_scrobble(client, track, self.max_retries, self.retry_delay).await;
                results.insert("listenbrainz".to_string(), result);
            }
        }

        results
    }

    async fn retry_scrobble<T>(
        client: &mut T,
        track: &ScrobblingTrack,
        max_retries: u32,
        retry_delay: Duration,
    ) -> Result<()>
    where
        T: ScrobblingClient,
    {
        let mut attempts = 0;

        loop {
            match client.scrobble(track).await {
                Ok(_) => return Ok(()),
                Err(e) => {
                    attempts += 1;
                    if attempts >= max_retries {
                        return Err(e);
                    }
                    sleep(retry_delay).await;
                }
            }
        }
    }
}
