use std::collections::VecDeque;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use simple_channel::{SimpleChannel, SimpleReceiver, SimpleSender};
use tokio::sync::Mutex;
use tokio::time::sleep;

use crate::last_fm::LastFmClient;
use crate::libre_fm::LibreFmClient;
use crate::listen_brainz::ListenBrainzClient;
use crate::{ScrobblingClient, ScrobblingTrack};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScrobblingService {
    LastFm,
    LibreFm,
    ListenBrainz,
}

impl fmt::Display for ScrobblingService {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            ScrobblingService::LastFm => "LastFm",
            ScrobblingService::LibreFm => "LibreFm",
            ScrobblingService::ListenBrainz => "ListenBrainz",
        };
        write!(f, "{}", s)
    }
}

impl FromStr for ScrobblingService {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "LastFm" => Ok(ScrobblingService::LastFm),
            "LibreFm" => Ok(ScrobblingService::LibreFm),
            "ListenBrainz" => Ok(ScrobblingService::ListenBrainz),
            _ => Err(()),
        }
    }
}

// Optionally implement From<String> using FromStr
impl From<String> for ScrobblingService {
    fn from(s: String) -> Self {
        ScrobblingService::from_str(&s)
            .unwrap_or_else(|_| panic!("Invalid string for ScrobblingService: {}", s))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionType {
    Authenticate,
    Scrobbling,
    UpdateNowPlaying,
}

#[derive(Debug)]
pub struct ScrobblingError {
    pub service: ScrobblingService,
    pub action: ActionType,
    pub error: anyhow::Error,
}

#[derive(Debug)]
pub struct LoginStatus {
    pub service: ScrobblingService,
    pub is_available: bool,
}

pub struct ScrobblingManager {
    lastfm: Option<LastFmClient>,
    librefm: Option<LibreFmClient>,
    listenbrainz: Option<ListenBrainzClient>,
    max_retries: u32,
    retry_delay: Duration,
    error_sender: Arc<SimpleSender<ScrobblingError>>,
    login_status_sender: Arc<SimpleSender<Vec<LoginStatus>>>,

    is_authenticating: bool,
    now_playing_cache: VecDeque<ScrobblingTrack>,
    scrobble_cache: VecDeque<ScrobblingTrack>,
}

pub struct ScrobblingCredential {
    pub service: ScrobblingService,
    pub username: String,
    pub password: String,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
}

impl ScrobblingManager {
    pub fn new(max_retries: u32, retry_delay: Duration) -> Self {
        let (error_sender, _) = SimpleChannel::channel(32);
        let (login_status_sender, _) = SimpleChannel::channel(32);

        Self {
            lastfm: None,
            librefm: None,
            listenbrainz: None,
            max_retries,
            retry_delay,
            error_sender: Arc::new(error_sender),
            login_status_sender: Arc::new(login_status_sender),

            is_authenticating: false,
            now_playing_cache: VecDeque::with_capacity(1),
            scrobble_cache: VecDeque::with_capacity(48),
        }
    }

    pub async fn send_login_status(&self) {
        let statuses = vec![
            LoginStatus {
                service: ScrobblingService::LastFm,
                is_available: self.lastfm.is_some(),
            },
            LoginStatus {
                service: ScrobblingService::LibreFm,
                is_available: self.librefm.is_some(),
            },
            LoginStatus {
                service: ScrobblingService::ListenBrainz,
                is_available: self.listenbrainz.is_some(),
            },
        ];

        self.login_status_sender.send(statuses);
    }

    pub async fn authenticate(
        &mut self,
        service: &ScrobblingService,
        username: &str,
        password: &str,
        api_key: Option<String>,
        api_secret: Option<String>,
    ) -> Result<()> {
        self.is_authenticating = true;
        let mut attempts = 0;

        loop {
            let result = match service {
                ScrobblingService::LastFm => {
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
                ScrobblingService::LibreFm => {
                    let mut client = LibreFmClient::new()?;
                    client.authenticate(username, password).await.map(|_| {
                        self.librefm = Some(client);
                    })
                }
                ScrobblingService::ListenBrainz => {
                    let mut client = ListenBrainzClient::new()?;
                    client.authenticate(username, password).await.map(|_| {
                        self.listenbrainz = Some(client);
                    })
                }
            };

            match result {
                Ok(_) => {
                    self.is_authenticating = false;
                    self.process_cache().await;
                    self.send_login_status().await;
                    break;
                }
                Err(e) => {
                    attempts += 1;
                    if attempts >= self.max_retries {
                        self.is_authenticating = false;
                        self.send_login_status().await;
                        return Err(e);
                    }
                    sleep(self.retry_delay).await;
                }
            }
        }
        Ok(())
    }

    async fn process_cache(&mut self) {
        if self.is_authenticating {
            return;
        }

        while let Some(track) = self.now_playing_cache.pop_front() {
            self.update_now_playing_all(track);
        }

        while let Some(track) = self.scrobble_cache.pop_front() {
            self.scrobble_all(track);
        }
    }

    pub async fn update_now_playing(
        &mut self,
        service: &ScrobblingService,
        track: ScrobblingTrack,
    ) {
        if self.is_authenticating {
            self.now_playing_cache.push_back(track);
            if self.now_playing_cache.len() > 1 {
                self.now_playing_cache.pop_front();
            }

            return;
        }

        let max_retries = self.max_retries;
        let retry_delay = self.retry_delay;

        let client: Option<&mut dyn ScrobblingClient> = match service {
            ScrobblingService::LastFm => {
                self.lastfm.as_mut().map(|c| c as &mut dyn ScrobblingClient)
            }
            ScrobblingService::LibreFm => self
                .librefm
                .as_mut()
                .map(|c| c as &mut dyn ScrobblingClient),
            ScrobblingService::ListenBrainz => self
                .listenbrainz
                .as_mut()
                .map(|c| c as &mut dyn ScrobblingClient),
        };

        if let Some(client) = client {
            if client.session_key().is_some() {
                let result = ScrobblingManager::retry_update_now_playing(
                    client,
                    &track,
                    max_retries,
                    retry_delay,
                )
                .await;

                if let Err(e) = result {
                    self.error_sender.send(ScrobblingError {
                        service: *service,
                        action: ActionType::UpdateNowPlaying,
                        error: e,
                    });
                }
            }
        }
    }

    async fn retry_update_now_playing<T>(
        client: &mut T,
        track: &ScrobblingTrack,
        max_retries: u32,
        retry_delay: Duration,
    ) -> Result<()>
    where
        T: ScrobblingClient + ?Sized,
    {
        let mut attempts = 0;

        loop {
            match client.update_now_playing(track).await {
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

    pub fn authenticate_all(manager: Arc<Mutex<Self>>, credentials_list: Vec<ScrobblingCredential>) {
        tokio::spawn(async move {
            for credentials in credentials_list {
                let mut manager = manager.lock().await;
                let result = manager
                    .authenticate(
                        &credentials.service,
                        &credentials.username,
                        &credentials.password,
                        credentials.api_key.clone(),
                        credentials.api_secret.clone(),
                    )
                    .await;

                if let Err(e) = result {
                    manager.error_sender.send(ScrobblingError {
                        service: credentials.service,
                        action: ActionType::Authenticate,
                        error: e,
                    });
                }
            }
        });
    }

    pub fn restore_session(
        &mut self,
        service: &ScrobblingService,
        session_key: String,
    ) -> Result<()> {
        match service {
            ScrobblingService::LastFm => {
                if let Some(client) = &mut self.lastfm {
                    client.session_key = Some(session_key);
                } else {
                    return Err(anyhow::anyhow!("Last.fm client not initialized"));
                }
            }
            ScrobblingService::LibreFm => {
                if let Some(client) = &mut self.librefm {
                    client.session_key = Some(session_key);
                } else {
                    return Err(anyhow::anyhow!("Libre.fm client not initialized"));
                }
            }
            ScrobblingService::ListenBrainz => {
                if let Some(client) = &mut self.listenbrainz {
                    client.session_key = Some(session_key);
                } else {
                    return Err(anyhow::anyhow!("ListenBrainz client not initialized"));
                }
            }
        }
        Ok(())
    }

    pub fn update_now_playing_all(&mut self, track: ScrobblingTrack) {
        if self.is_authenticating {
            self.now_playing_cache.push_back(track);
            if self.now_playing_cache.len() > 1 {
                self.now_playing_cache.pop_front();
            }
            return;
        }

        let lastfm = self.lastfm.clone();
        let librefm = self.librefm.clone();
        let listenbrainz = self.listenbrainz.clone();
        let error_sender = Arc::clone(&self.error_sender);

        tokio::spawn(async move {
            if let Some(client) = lastfm {
                if client.session_key.is_some() {
                    if let Err(e) = client.update_now_playing(&track).await {
                        error_sender.send(ScrobblingError {
                            service: ScrobblingService::LastFm,
                            action: ActionType::UpdateNowPlaying,
                            error: e,
                        });
                    }
                }
            }

            if let Some(client) = librefm {
                if client.session_key.is_some() {
                    if let Err(e) = client.update_now_playing(&track).await {
                        error_sender.send(ScrobblingError {
                            service: ScrobblingService::LibreFm,
                            action: ActionType::UpdateNowPlaying,
                            error: e,
                        });
                    }
                }
            }

            if let Some(client) = listenbrainz {
                if client.session_key.is_some() {
                    if let Err(e) = client.update_now_playing(&track).await {
                        error_sender.send(ScrobblingError {
                            service: ScrobblingService::ListenBrainz,
                            action: ActionType::UpdateNowPlaying,
                            error: e,
                        });
                    }
                }
            }
        });
    }

    pub async fn scrobble(&mut self, service: ScrobblingService, track: ScrobblingTrack) {
        if self.is_authenticating {
            self.scrobble_cache.push_back(track);
            if self.scrobble_cache.len() > 48 {
                self.scrobble_cache.pop_front();
            }

            return;
        }

        let max_retries = self.max_retries;
        let retry_delay = self.retry_delay;

        let client: Option<&mut dyn ScrobblingClient> = match service {
            ScrobblingService::LastFm => {
                self.lastfm.as_mut().map(|c| c as &mut dyn ScrobblingClient)
            }
            ScrobblingService::LibreFm => self
                .librefm
                .as_mut()
                .map(|c| c as &mut dyn ScrobblingClient),
            ScrobblingService::ListenBrainz => self
                .listenbrainz
                .as_mut()
                .map(|c| c as &mut dyn ScrobblingClient),
        };

        if let Some(client) = client {
            if client.session_key().is_some() {
                let result =
                    ScrobblingManager::retry_scrobble(client, &track, max_retries, retry_delay)
                        .await;

                if let Err(e) = result {
                    self.error_sender.send(ScrobblingError {
                        service,
                        action: ActionType::Scrobbling,
                        error: e,
                    });
                }
            }
        }
    }

    pub fn scrobble_all(&mut self, track: ScrobblingTrack) {
        if self.is_authenticating {
            self.scrobble_cache.push_back(track);
            if self.scrobble_cache.len() > 48 {
                self.scrobble_cache.pop_front();
            }

            return;
        }

        let lastfm = self.lastfm.clone();
        let librefm = self.librefm.clone();
        let listenbrainz = self.listenbrainz.clone();
        let max_retries = self.max_retries;
        let retry_delay = self.retry_delay;
        let error_sender = Arc::clone(&self.error_sender);

        tokio::spawn(async move {
            // Handle Last.fm
            if let Some(mut client) = lastfm {
                if client.session_key.is_some() {
                    let result = ScrobblingManager::retry_scrobble(
                        &mut client,
                        &track,
                        max_retries,
                        retry_delay,
                    )
                    .await;

                    if let Err(e) = result {
                        error_sender.send(ScrobblingError {
                            service: ScrobblingService::LastFm,
                            action: ActionType::Scrobbling,
                            error: e,
                        });
                    }
                }
            }

            // Handle Libre.fm
            if let Some(mut client) = librefm {
                if client.session_key.is_some() {
                    let result = ScrobblingManager::retry_scrobble(
                        &mut client,
                        &track,
                        max_retries,
                        retry_delay,
                    )
                    .await;

                    if let Err(e) = result {
                        error_sender.send(ScrobblingError {
                            service: ScrobblingService::LibreFm,
                            action: ActionType::Scrobbling,
                            error: e,
                        });
                    }
                }
            }

            // Handle ListenBrainz
            if let Some(mut client) = listenbrainz {
                if client.session_key.is_some() {
                    let result = ScrobblingManager::retry_scrobble(
                        &mut client,
                        &track,
                        max_retries,
                        retry_delay,
                    )
                    .await;

                    if let Err(e) = result {
                        error_sender.send(ScrobblingError {
                            service: ScrobblingService::ListenBrainz,
                            action: ActionType::Scrobbling,
                            error: e,
                        });
                    }
                }
            }
        });
    }

    async fn retry_scrobble<T>(
        client: &mut T,
        track: &ScrobblingTrack,
        max_retries: u32,
        retry_delay: Duration,
    ) -> Result<()>
    where
        T: ScrobblingClient + ?Sized,
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

    pub fn subscribe_error(&self) -> SimpleReceiver<ScrobblingError> {
        self.error_sender.subscribe()
    }

    pub fn subscribe_login_status(&self) -> SimpleReceiver<Vec<LoginStatus>> {
        self.login_status_sender.subscribe()
    }
}
