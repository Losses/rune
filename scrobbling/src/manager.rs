use std::{collections::VecDeque, fmt, str::FromStr, sync::Arc, time::Duration};

use anyhow::Result;
use async_trait::async_trait;
use log::{error, info, warn};
use simple_channel::{SimpleChannel, SimpleReceiver, SimpleSender};
use tokio::{sync::Mutex, time::sleep};

use crate::{
    LastFmApiCredentialField, ScrobblingAuthFailure, ScrobblingClient, ScrobblingTrack,
    last_fm::LastFmClient, libre_fm::LibreFmClient, listen_brainz::ListenBrainzClient,
};
use anyhow::Error;

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
        write!(f, "{s}")
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
            .unwrap_or_else(|_| panic!("Invalid string for ScrobblingService: {s}"))
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
    pub library_id: Option<String>,
    pub service: ScrobblingService,
    pub action: ActionType,
    pub error: anyhow::Error,
}

#[derive(Debug)]
pub struct LoginStatus {
    pub service: ScrobblingService,
    pub is_available: bool,
    pub error_message: Option<String>,
}

const MIN_LISTEN_DURATION: Duration = Duration::from_secs(4 * 60);
const LAST_FM_MIN_TRACK_DURATION_SECS: f64 = 30.0;

/// The listening a service requires before it accepts a final submission. A track that lasts
/// longer than four minutes still needs only four minutes of listening; an unknown duration
/// falls back to the same four minutes.
fn required_listen(track: &ScrobblingTrack) -> Duration {
    track
        .duration
        .filter(|seconds| *seconds > 0.0)
        .map(|seconds| Duration::from_secs_f64(seconds.min(f64::from(u32::MAX))) / 2)
        .unwrap_or(MIN_LISTEN_DURATION)
        .min(MIN_LISTEN_DURATION)
}

/// Whether `listened` qualifies as a play for `service`. Last.fm also refuses tracks of 30
/// seconds or less; ListenBrainz and Libre.fm have no track-length rule.
fn listen_qualifies(
    track: &ScrobblingTrack,
    listened: Duration,
    service: ScrobblingService,
) -> bool {
    let long_enough = match service {
        ScrobblingService::LastFm => track
            .duration
            .filter(|seconds| *seconds > 0.0)
            .is_none_or(|seconds| seconds > LAST_FM_MIN_TRACK_DURATION_SECS),
        ScrobblingService::LibreFm | ScrobblingService::ListenBrainz => true,
    };
    long_enough && listened >= required_listen(track)
}

/// Submits listening history to the services the user has authenticated.
#[async_trait]
pub trait ScrobblingServiceManager: Send + Sync {
    /// Sends the current login state to every subscriber.
    async fn send_login_status(&self);
    /// Authenticates `service` and keeps its session for later submissions.
    async fn authenticate(
        &mut self,
        service: &ScrobblingService,
        username: &str,
        password: &str,
        api_key: Option<String>,
        api_secret: Option<String>,
        enable_retry: bool,
    ) -> Result<()> {
        self.authenticate_staged(
            service,
            username,
            password,
            api_key,
            api_secret,
            enable_retry,
        )
        .await?;
        self.commit_staged_authentication().await;
        Ok(())
    }
    /// Authenticates `service` and stages the client without activating it.
    ///
    /// The current login stays active until [`Self::commit_staged_authentication`]
    /// is called. A failed staged authentication leaves the current login active.
    async fn authenticate_staged(
        &mut self,
        service: &ScrobblingService,
        username: &str,
        password: &str,
        api_key: Option<String>,
        api_secret: Option<String>,
        enable_retry: bool,
    ) -> Result<()>;
    /// Activates the staged client, flushes queued submissions to it, and
    /// publishes the new login state.
    async fn commit_staged_authentication(&mut self);
    /// Drops the staged client without activating it or sending it queued
    /// submissions. The current login stays active.
    ///
    /// This is synchronous so a dropped login future can call it from `Drop`.
    fn discard_staged_authentication(&mut self);
    /// Reports that `track` started playing on `service`.
    async fn update_now_playing(&mut self, service: &ScrobblingService, track: ScrobblingTrack);
    /// Restores a saved session key for `service`.
    fn restore_session(&mut self, service: &ScrobblingService, session_key: String) -> Result<()>;
    /// Reports that `track` started playing on every authenticated service.
    fn update_now_playing_all(&mut self, track: ScrobblingTrack);
    /// Submits one final listen to `service` when `listened` qualifies.
    async fn scrobble(
        &mut self,
        service: ScrobblingService,
        track: ScrobblingTrack,
        listened: Duration,
    );
    /// Submits one final listen to every authenticated service whose rule `listened` meets.
    fn scrobble_all(&mut self, track: ScrobblingTrack, listened: Duration);
    /// Forgets the session and credentials of `service`.
    async fn logout(&mut self, service: ScrobblingService);
    /// Receives submission and authentication errors.
    fn subscribe_error(&self) -> SimpleReceiver<ScrobblingError>;
    /// Receives the login state after it changes.
    fn subscribe_login_status(&self) -> SimpleReceiver<Vec<LoginStatus>>;
    /// Returns the sender that carries submission and authentication errors.
    fn error_sender(&self) -> Arc<SimpleSender<ScrobblingError>>;
}

/// Submits listens to the services the user has authenticated.
pub struct ScrobblingManager {
    lastfm: Option<LastFmClient>,
    librefm: Option<LibreFmClient>,
    listenbrainz: Option<ListenBrainzClient>,

    lastfm_error: Option<String>,
    librefm_error: Option<String>,
    listenbrainz_error: Option<String>,

    max_retries: u32,
    retry_delay: Duration,
    error_sender: Arc<SimpleSender<ScrobblingError>>,
    login_status_sender: Arc<SimpleSender<Vec<LoginStatus>>>,

    is_authenticating: bool,
    staged: Option<StagedAuthentication>,
    now_playing_cache: VecDeque<ScrobblingTrack>,
    scrobble_cache: VecDeque<(ScrobblingTrack, Duration)>,
}

pub struct ScrobblingCredential {
    pub service: ScrobblingService,
    pub username: String,
    pub password: String,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
}

enum StagedAuthentication {
    LastFm(LastFmClient),
    LibreFm(LibreFmClient),
    ListenBrainz(ListenBrainzClient),
}

fn redact_authentication_secrets(
    mut message: String,
    password: &str,
    api_key: Option<&str>,
    api_secret: Option<&str>,
) -> String {
    for secret in [Some(password), api_key, api_secret].into_iter().flatten() {
        if !secret.is_empty() {
            message = message.replace(secret, "<redacted>");
        }
    }
    message
}

fn classify_authentication_error(
    error: anyhow::Error,
    password: &str,
    api_key: Option<&str>,
    api_secret: Option<&str>,
) -> ScrobblingAuthFailure {
    if let Some(failure) = error.downcast_ref::<ScrobblingAuthFailure>() {
        let redact = |detail: &str| {
            redact_authentication_secrets(detail.to_string(), password, api_key, api_secret)
        };
        return match failure {
            ScrobblingAuthFailure::CredentialsRejected { detail } => {
                ScrobblingAuthFailure::CredentialsRejected {
                    detail: redact(detail),
                }
            }
            ScrobblingAuthFailure::ServiceUnreachable { detail } => {
                ScrobblingAuthFailure::ServiceUnreachable {
                    detail: redact(detail),
                }
            }
            ScrobblingAuthFailure::LastFmApiCredentialsMissing(field) => {
                ScrobblingAuthFailure::LastFmApiCredentialsMissing(*field)
            }
            ScrobblingAuthFailure::LastFmApiCredentialsInvalid { detail } => {
                ScrobblingAuthFailure::LastFmApiCredentialsInvalid {
                    detail: redact(detail),
                }
            }
            ScrobblingAuthFailure::Other { detail } => ScrobblingAuthFailure::Other {
                detail: redact(detail),
            },
        };
    }

    if let Some(network_error) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<reqwest::Error>())
        && (network_error.is_timeout() || network_error.is_connect())
    {
        return ScrobblingAuthFailure::ServiceUnreachable {
            detail: redact_authentication_secrets(
                format!("Scrobbling service is unreachable: {error:#}"),
                password,
                api_key,
                api_secret,
            ),
        };
    }

    ScrobblingAuthFailure::Other {
        detail: redact_authentication_secrets(format!("{error:#}"), password, api_key, api_secret),
    }
}

impl ScrobblingManager {
    /// Creates a manager that retries a failed submission `max_retries` times, waiting
    /// `retry_delay` between attempts.
    pub fn new(max_retries: u32, retry_delay: Duration) -> Self {
        let (error_sender, _) = SimpleChannel::channel(32);
        let (login_status_sender, _) = SimpleChannel::channel(32);

        Self {
            lastfm: None,
            librefm: None,
            listenbrainz: None,

            lastfm_error: None,
            librefm_error: None,
            listenbrainz_error: None,

            max_retries,
            retry_delay,
            error_sender: Arc::new(error_sender),
            login_status_sender: Arc::new(login_status_sender),

            is_authenticating: false,
            staged: None,
            now_playing_cache: VecDeque::with_capacity(1),
            scrobble_cache: VecDeque::with_capacity(48),
        }
    }

    fn process_cache(&mut self) {
        if self.is_authenticating {
            return;
        }

        while let Some(track) = self.now_playing_cache.pop_front() {
            self.update_now_playing_all(track);
        }

        while let Some((track, listened)) = self.scrobble_cache.pop_front() {
            self.scrobble_all(track, listened);
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
                Ok(()) => return Ok(()),
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
                Ok(()) => return Ok(()),
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

    pub fn authenticate_all(
        manager: Arc<Mutex<dyn ScrobblingServiceManager>>,
        credentials_list: Vec<ScrobblingCredential>,
    ) {
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
                        true,
                    )
                    .await;

                if let Err(e) = result {
                    manager.error_sender().send(ScrobblingError {
                        service: credentials.service,
                        action: ActionType::Authenticate,
                        library_id: None,
                        error: e,
                    });
                }
            }
        });
    }
}

#[async_trait]
impl ScrobblingServiceManager for ScrobblingManager {
    async fn send_login_status(&self) {
        let statuses = vec![
            LoginStatus {
                service: ScrobblingService::LastFm,
                is_available: self.lastfm.is_some(),
                error_message: self.lastfm_error.clone(),
            },
            LoginStatus {
                service: ScrobblingService::LibreFm,
                is_available: self.librefm.is_some(),
                error_message: self.librefm_error.clone(),
            },
            LoginStatus {
                service: ScrobblingService::ListenBrainz,
                is_available: self.listenbrainz.is_some(),
                error_message: self.listenbrainz_error.clone(),
            },
        ];

        self.login_status_sender.send(statuses);
    }

    async fn authenticate_staged(
        &mut self,
        service: &ScrobblingService,
        username: &str,
        password: &str,
        api_key: Option<String>,
        api_secret: Option<String>,
        enable_retry: bool,
    ) -> Result<()> {
        self.staged = None;
        self.is_authenticating = true;
        let mut attempts = 0;

        loop {
            info!("Authenticating to {service} ({attempts})...");
            let result = match service {
                ScrobblingService::LastFm => {
                    match (
                        api_key.clone().filter(|value| !value.trim().is_empty()),
                        api_secret.clone().filter(|value| !value.trim().is_empty()),
                    ) {
                        (None, _) => Err(ScrobblingAuthFailure::LastFmApiCredentialsMissing(
                            LastFmApiCredentialField::ApiKey,
                        )
                        .into()),
                        (_, None) => Err(ScrobblingAuthFailure::LastFmApiCredentialsMissing(
                            LastFmApiCredentialField::ApiSecret,
                        )
                        .into()),
                        (Some(api_key), Some(api_secret)) => {
                            match LastFmClient::new(api_key, api_secret) {
                                Ok(mut client) => {
                                    client.authenticate(username, password).await.map(|_| {
                                        self.staged = Some(StagedAuthentication::LastFm(client));
                                    })
                                }
                                Err(error) => Err(error),
                            }
                        }
                    }
                }
                ScrobblingService::LibreFm => {
                    let mut client = LibreFmClient::new()?;
                    client.authenticate(username, password).await.map(|_| {
                        self.staged = Some(StagedAuthentication::LibreFm(client));
                    })
                }
                ScrobblingService::ListenBrainz => {
                    let mut client = ListenBrainzClient::new()?;
                    client.authenticate(username, password).await.map(|_| {
                        self.staged = Some(StagedAuthentication::ListenBrainz(client));
                    })
                }
            };

            match result {
                Ok(_) => {
                    info!("Authenticated to {}", { service });
                    break;
                }
                Err(e) => {
                    attempts += 1;
                    let failure = classify_authentication_error(
                        e,
                        password,
                        api_key.as_deref(),
                        api_secret.as_deref(),
                    );
                    let error_message = failure.to_string();
                    error!("Failed to authenticate to {service}: {error_message}");
                    match service {
                        ScrobblingService::LastFm => self.lastfm_error = Some(error_message),
                        ScrobblingService::LibreFm => self.librefm_error = Some(error_message),
                        ScrobblingService::ListenBrainz => {
                            self.listenbrainz_error = Some(error_message)
                        }
                    }

                    if attempts >= self.max_retries || !enable_retry {
                        self.is_authenticating = false;
                        self.send_login_status().await;
                        return Err(Error::new(failure));
                    }
                    sleep(self.retry_delay).await;
                }
            }
        }
        Ok(())
    }

    async fn commit_staged_authentication(&mut self) {
        let Some(staged) = self.staged.take() else {
            return;
        };
        match staged {
            StagedAuthentication::LastFm(client) => {
                self.lastfm = Some(client);
                self.lastfm_error = None;
            }
            StagedAuthentication::LibreFm(client) => {
                self.librefm = Some(client);
                self.librefm_error = None;
            }
            StagedAuthentication::ListenBrainz(client) => {
                self.listenbrainz = Some(client);
                self.listenbrainz_error = None;
            }
        }
        self.is_authenticating = false;
        self.process_cache();
        self.send_login_status().await;
    }

    fn discard_staged_authentication(&mut self) {
        self.staged = None;
        self.is_authenticating = false;
        self.process_cache();
    }

    async fn update_now_playing(&mut self, service: &ScrobblingService, track: ScrobblingTrack) {
        if self.is_authenticating {
            self.now_playing_cache.push_back(track);
            if self.now_playing_cache.len() > 1 {
                self.now_playing_cache.pop_front();
            }

            info!("Caching now playing update for {}", { service });

            return;
        }

        info!("Updating now playing for {}", { service });

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

        if let Some(client) = client
            && client.session_key().is_some()
        {
            let result = ScrobblingManager::retry_update_now_playing(
                client,
                &track,
                max_retries,
                retry_delay,
            )
            .await;

            if let Err(e) = result {
                error!("Failed to update now playing for {service}");

                self.error_sender.send(ScrobblingError {
                    service: *service,
                    action: ActionType::UpdateNowPlaying,
                    library_id: track.library_id.clone(),
                    error: e,
                });
            }
        }
    }

    fn restore_session(&mut self, service: &ScrobblingService, session_key: String) -> Result<()> {
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

    fn update_now_playing_all(&mut self, track: ScrobblingTrack) {
        if self.is_authenticating {
            self.now_playing_cache.push_back(track);
            if self.now_playing_cache.len() > 1 {
                self.now_playing_cache.pop_front();
            }

            info!("Caching now playing for all services");
            return;
        }

        info!("Updating now playing for all services");

        let lastfm = self.lastfm.clone();
        let librefm = self.librefm.clone();
        let listenbrainz = self.listenbrainz.clone();
        let error_sender = Arc::clone(&self.error_sender);

        tokio::spawn(async move {
            if let Some(client) = lastfm
                && client.session_key.is_some()
                && let Err(e) = client.update_now_playing(&track).await
            {
                error_sender.send(ScrobblingError {
                    service: ScrobblingService::LastFm,
                    action: ActionType::UpdateNowPlaying,
                    library_id: track.library_id.clone(),
                    error: e,
                });
            }

            if let Some(client) = librefm
                && client.session_key.is_some()
                && let Err(e) = client.update_now_playing(&track).await
            {
                error_sender.send(ScrobblingError {
                    service: ScrobblingService::LibreFm,
                    action: ActionType::UpdateNowPlaying,
                    library_id: track.library_id.clone(),
                    error: e,
                });
            }

            if let Some(client) = listenbrainz
                && client.session_key.is_some()
                && let Err(e) = client.update_now_playing(&track).await
            {
                error_sender.send(ScrobblingError {
                    service: ScrobblingService::ListenBrainz,
                    action: ActionType::UpdateNowPlaying,
                    library_id: track.library_id.clone(),
                    error: e,
                });
            }
        });
    }

    async fn scrobble(
        &mut self,
        service: ScrobblingService,
        track: ScrobblingTrack,
        listened: Duration,
    ) {
        if self.is_authenticating {
            self.scrobble_cache.push_back((track, listened));
            if self.scrobble_cache.len() > 48 {
                self.scrobble_cache.pop_front();
            }

            info!("Caching scrobble for {}", { service });

            return;
        }

        if !listen_qualifies(&track, listened, service) {
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
                    error!("Failed to scrobble to {service}");

                    self.error_sender.send(ScrobblingError {
                        service,
                        action: ActionType::Scrobbling,
                        library_id: track.library_id.clone(),
                        error: e,
                    });
                } else {
                    info!("Scrobbled to {}", { service });
                }
            } else {
                warn!("Not authenticated to {}", { service });
            }
        }
    }

    fn scrobble_all(&mut self, track: ScrobblingTrack, listened: Duration) {
        if self.is_authenticating {
            self.scrobble_cache.push_back((track, listened));
            if self.scrobble_cache.len() > 48 {
                self.scrobble_cache.pop_front();
            }

            return;
        }

        let lastfm = if listen_qualifies(&track, listened, ScrobblingService::LastFm) {
            self.lastfm.clone()
        } else {
            None
        };
        let librefm = if listen_qualifies(&track, listened, ScrobblingService::LibreFm) {
            self.librefm.clone()
        } else {
            None
        };
        let listenbrainz = if listen_qualifies(&track, listened, ScrobblingService::ListenBrainz) {
            self.listenbrainz.clone()
        } else {
            None
        };
        let max_retries = self.max_retries;
        let retry_delay = self.retry_delay;
        let error_sender = Arc::clone(&self.error_sender);

        tokio::spawn(async move {
            // Handle Last.fm
            if let Some(mut client) = lastfm
                && client.session_key.is_some()
            {
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
                        library_id: track.library_id.clone(),
                        error: e,
                    });
                }
            }

            // Handle Libre.fm
            if let Some(mut client) = librefm
                && client.session_key.is_some()
            {
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
                        library_id: track.library_id.clone(),
                        error: e,
                    });
                }
            }

            // Handle ListenBrainz
            if let Some(mut client) = listenbrainz
                && client.session_key.is_some()
            {
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
                        library_id: track.library_id.clone(),
                        error: e,
                    });
                }
            }
        });
    }

    async fn logout(&mut self, service: ScrobblingService) {
        match service {
            ScrobblingService::LastFm => {
                self.lastfm = None;
                self.lastfm_error = None;
            }
            ScrobblingService::LibreFm => {
                self.librefm = None;
                self.librefm_error = None;
            }
            ScrobblingService::ListenBrainz => {
                self.listenbrainz = None;
                self.listenbrainz_error = None;
            }
        }

        info!("Logged out from {}", { service });
        self.send_login_status().await;
    }

    fn subscribe_error(&self) -> SimpleReceiver<ScrobblingError> {
        self.error_sender.subscribe()
    }

    fn subscribe_login_status(&self) -> SimpleReceiver<Vec<LoginStatus>> {
        self.login_status_sender.subscribe()
    }

    fn error_sender(&self) -> Arc<SimpleSender<ScrobblingError>> {
        Arc::clone(&self.error_sender)
    }
}

/// A manager that accepts calls without sending anything.
pub struct MockScrobblingManager {
    error_sender: Arc<SimpleSender<ScrobblingError>>,
    login_status_sender: Arc<SimpleSender<Vec<LoginStatus>>>,
}

impl MockScrobblingManager {
    /// Creates a manager that accepts calls without sending anything.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for MockScrobblingManager {
    fn default() -> Self {
        let (error_sender, _) = SimpleChannel::channel(32);
        let (login_status_sender, _) = SimpleChannel::channel(32);

        Self {
            error_sender: Arc::new(error_sender),
            login_status_sender: Arc::new(login_status_sender),
        }
    }
}

#[async_trait]
impl ScrobblingServiceManager for MockScrobblingManager {
    async fn send_login_status(&self) {
        // Mock implementation: send empty login status list
        self.login_status_sender.send(Vec::new());
    }

    async fn authenticate_staged(
        &mut self,
        _service: &ScrobblingService,
        _username: &str,
        _password: &str,
        _api_key: Option<String>,
        _api_secret: Option<String>,
        _enable_retry: bool,
    ) -> Result<()> {
        // Mock implementation: always succeed
        Ok(())
    }

    async fn commit_staged_authentication(&mut self) {
        // Mock implementation: nothing to activate
    }

    fn discard_staged_authentication(&mut self) {
        // Mock implementation: nothing to discard
    }

    async fn update_now_playing(&mut self, _service: &ScrobblingService, _track: ScrobblingTrack) {
        // Mock implementation: do nothing
    }

    fn restore_session(
        &mut self,
        _service: &ScrobblingService,
        _session_key: String,
    ) -> Result<()> {
        // Mock implementation: always succeed
        Ok(())
    }

    fn update_now_playing_all(&mut self, _track: ScrobblingTrack) {
        // Mock implementation: do nothing
    }

    async fn scrobble(
        &mut self,
        _service: ScrobblingService,
        _track: ScrobblingTrack,
        _listened: Duration,
    ) {
        // Mock implementation: do nothing
    }

    fn scrobble_all(&mut self, _track: ScrobblingTrack, _listened: Duration) {
        // Mock implementation: do nothing
    }

    async fn logout(&mut self, _service: ScrobblingService) {
        // Mock implementation: do nothing
    }

    fn subscribe_error(&self) -> SimpleReceiver<ScrobblingError> {
        self.error_sender.subscribe()
    }

    fn subscribe_login_status(&self) -> SimpleReceiver<Vec<LoginStatus>> {
        self.login_status_sender.subscribe()
    }

    fn error_sender(&self) -> Arc<SimpleSender<ScrobblingError>> {
        Arc::clone(&self.error_sender)
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::atomic::Ordering, time::Duration};

    use anyhow::anyhow;
    use tokio::time::timeout;

    use super::{
        ActionType, ScrobblingManager, ScrobblingService, ScrobblingServiceManager,
        StagedAuthentication, classify_authentication_error, listen_qualifies,
        redact_authentication_secrets,
    };
    use crate::{
        ScrobblingAuthFailure, ScrobblingTrack,
        last_fm::LastFmClient,
        libre_fm::LibreFmClient,
        listen_brainz::ListenBrainzClient,
        test_support::{sample_track, spawn_persistent_server, spawn_sequence_server},
    };

    fn track_with_duration(seconds: f64) -> ScrobblingTrack {
        ScrobblingTrack {
            duration: Some(seconds),
            ..sample_track()
        }
    }

    fn lastfm_client(base_url: String, session_key: &str) -> LastFmClient {
        let mut client = LastFmClient::new("api-key".into(), "api-secret".into()).unwrap();
        client.base_url = base_url;
        client.session_key = Some(session_key.into());
        client
    }

    #[test]
    fn auth_error_redaction_preserves_non_secret_details() {
        let error = redact_authentication_secrets(
            "Authentication failed: invalid credentials for user, token-value".into(),
            "token-value",
            Some("api-key-value"),
            Some("api-secret-value"),
        );

        assert_eq!(
            error,
            "Authentication failed: invalid credentials for user, <redacted>"
        );
    }

    #[test]
    fn auth_failure_classifier_redacts_secrets_from_upstream_details() {
        let failure = classify_authentication_error(
            anyhow!("upstream response echoed password-value, api-key-value, api-secret-value"),
            "password-value",
            Some("api-key-value"),
            Some("api-secret-value"),
        );

        let ScrobblingAuthFailure::Other { detail } = failure else {
            panic!("unexpected auth failure kind");
        };
        assert_eq!(
            detail,
            "upstream response echoed <redacted>, <redacted>, <redacted>"
        );
    }

    #[tokio::test]
    async fn discard_staged_authentication_keeps_the_active_client() {
        let (old_url, old_count, old_server) = spawn_sequence_server(vec![(
            "200 OK",
            r#"{"scrobbles":{"@attr":{"accepted":1,"ignored":0}}}"#.to_string(),
        )])
        .await;
        let (new_url, new_count, new_server) = spawn_sequence_server(vec![(
            "200 OK",
            r#"{"scrobbles":{"@attr":{"accepted":1,"ignored":0}}}"#.to_string(),
        )])
        .await;

        let mut manager = ScrobblingManager::new(1, Duration::from_millis(1));
        manager.lastfm = Some(lastfm_client(old_url, "old-session"));
        manager.staged = Some(StagedAuthentication::LastFm(lastfm_client(
            new_url,
            "new-session",
        )));
        manager.is_authenticating = true;
        let status = manager.subscribe_login_status();
        manager.scrobble_all(sample_track(), Duration::from_secs(30));

        manager.discard_staged_authentication();

        timeout(Duration::from_secs(5), old_server)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(old_count.load(Ordering::SeqCst), 1);
        assert_eq!(new_count.load(Ordering::SeqCst), 0);
        assert!(
            timeout(Duration::from_millis(50), status.recv())
                .await
                .is_err(),
            "discarding a staged login must not publish a login change"
        );
        assert!(manager.staged.is_none());
        assert!(manager.lastfm.is_some());
        new_server.abort();
    }

    #[tokio::test]
    async fn commit_staged_authentication_activates_the_staged_client() {
        let (old_url, old_count, old_server) = spawn_sequence_server(vec![(
            "200 OK",
            r#"{"scrobbles":{"@attr":{"accepted":1,"ignored":0}}}"#.to_string(),
        )])
        .await;
        let (new_url, new_count, new_server) = spawn_sequence_server(vec![(
            "200 OK",
            r#"{"scrobbles":{"@attr":{"accepted":1,"ignored":0}}}"#.to_string(),
        )])
        .await;

        let mut manager = ScrobblingManager::new(1, Duration::from_millis(1));
        manager.lastfm = Some(lastfm_client(old_url, "old-session"));
        manager.staged = Some(StagedAuthentication::LastFm(lastfm_client(
            new_url,
            "new-session",
        )));
        manager.is_authenticating = true;
        let status = manager.subscribe_login_status();
        manager.scrobble_all(sample_track(), Duration::from_secs(30));

        manager.commit_staged_authentication().await;

        timeout(Duration::from_secs(5), new_server)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(new_count.load(Ordering::SeqCst), 1);
        assert_eq!(old_count.load(Ordering::SeqCst), 0);
        let statuses = timeout(Duration::from_secs(5), status.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            statuses.iter().any(|status| {
                status.service == ScrobblingService::LastFm && status.is_available
            })
        );
        assert!(manager.staged.is_none());
        assert!(manager.lastfm.is_some());
        old_server.abort();
    }

    #[tokio::test]
    async fn scrobble_all_retries_rejected_last_fm_report_until_success() {
        let (base_url, request_count, server) = spawn_sequence_server(vec![
            (
                "200 OK",
                r#"{"error":11,"message":"Service Offline"}"#.to_string(),
            ),
            (
                "200 OK",
                r#"{"scrobbles":{"@attr":{"accepted":1,"ignored":0}}}"#.to_string(),
            ),
        ])
        .await;

        let mut lastfm = LastFmClient::new("api-key".into(), "api-secret".into()).unwrap();
        lastfm.base_url = base_url;
        lastfm.session_key = Some("session-key".into());

        let mut manager = ScrobblingManager::new(2, Duration::from_millis(1));
        manager.lastfm = Some(lastfm);
        let error_rx = manager.subscribe_error();

        manager.scrobble_all(sample_track(), Duration::from_secs(30));
        timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(request_count.load(Ordering::SeqCst), 2);
        assert!(
            timeout(Duration::from_millis(50), error_rx.recv())
                .await
                .is_err(),
            "retried scrobble that succeeded on second attempt must not emit an error"
        );
    }

    #[tokio::test]
    async fn scrobble_all_emits_error_after_retries_exhausted_for_rejected_libre_fm_report() {
        let (base_url, request_count, server) = spawn_sequence_server(vec![
            ("500 Internal Server Error", "upstream error".to_string()),
            (
                "200 OK",
                r#"{"error":9,"message":"Invalid session key"}"#.to_string(),
            ),
        ])
        .await;

        let mut librefm = LibreFmClient::new().unwrap();
        librefm.base_url = base_url;
        librefm.session_key = Some("session-key".into());

        let mut manager = ScrobblingManager::new(2, Duration::from_millis(1));
        manager.librefm = Some(librefm);
        let error_rx = manager.subscribe_error();

        manager.scrobble_all(sample_track(), Duration::from_secs(30));
        timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();

        let emitted = timeout(Duration::from_secs(5), error_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(request_count.load(Ordering::SeqCst), 2);
        assert_eq!(emitted.service, ScrobblingService::LibreFm);
        assert_eq!(emitted.action, ActionType::Scrobbling);
        assert_eq!(emitted.library_id.as_deref(), Some("lib-1"));
    }

    #[tokio::test]
    async fn update_now_playing_all_emits_errors_for_rejected_reports_without_retrying() {
        let (lastfm_url, lastfm_count, lastfm_server) = spawn_sequence_server(vec![(
            "200 OK",
            r#"{"error":9,"message":"Invalid session key"}"#.to_string(),
        )])
        .await;
        let (librefm_url, librefm_count, librefm_server) =
            spawn_sequence_server(vec![("500 Internal Server Error", "down".to_string())]).await;

        let mut lastfm = LastFmClient::new("api-key".into(), "api-secret".into()).unwrap();
        lastfm.base_url = lastfm_url;
        lastfm.session_key = Some("session-key".into());

        let mut librefm = LibreFmClient::new().unwrap();
        librefm.base_url = librefm_url;
        librefm.session_key = Some("session-key".into());

        let mut manager = ScrobblingManager::new(3, Duration::from_millis(1));
        manager.lastfm = Some(lastfm);
        manager.librefm = Some(librefm);
        let error_rx = manager.subscribe_error();

        manager.update_now_playing_all(sample_track());
        timeout(Duration::from_secs(5), lastfm_server)
            .await
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(5), librefm_server)
            .await
            .unwrap()
            .unwrap();

        let first = timeout(Duration::from_secs(5), error_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let second = timeout(Duration::from_secs(5), error_rx.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(lastfm_count.load(Ordering::SeqCst), 1);
        assert_eq!(librefm_count.load(Ordering::SeqCst), 1);
        assert_eq!(first.service, ScrobblingService::LastFm);
        assert_eq!(first.action, ActionType::UpdateNowPlaying);
        assert_eq!(first.library_id.as_deref(), Some("lib-1"));
        assert_eq!(second.service, ScrobblingService::LibreFm);
        assert_eq!(second.action, ActionType::UpdateNowPlaying);
        assert_eq!(second.library_id.as_deref(), Some("lib-1"));
    }

    #[test]
    fn listen_qualification_follows_each_services_rule() {
        let check =
            |duration: Option<f64>, listened: Duration, last_fm: bool, listen_brainz: bool| {
                let track = ScrobblingTrack {
                    duration,
                    ..sample_track()
                };
                assert_eq!(
                    listen_qualifies(&track, listened, ScrobblingService::LastFm),
                    last_fm,
                    "Last.fm eligibility for duration {duration:?} and {listened:?} listened"
                );
                for service in [ScrobblingService::LibreFm, ScrobblingService::ListenBrainz] {
                    assert_eq!(
                        listen_qualifies(&track, listened, service),
                        listen_brainz,
                        "{service} eligibility for duration {duration:?} and {listened:?} listened"
                    );
                }
            };

        check(Some(180.0), Duration::from_secs(4), false, false);
        check(Some(180.0), Duration::from_secs(89), false, false);
        check(Some(180.0), Duration::from_secs(90), true, true);
        check(Some(20.0), Duration::from_secs(20), false, true);
        check(Some(30.0), Duration::from_secs(30), false, true);
        check(Some(30.5), Duration::from_millis(15_249), false, false);
        check(Some(30.5), Duration::from_millis(15_250), true, true);
        check(Some(31.0), Duration::from_millis(15_000), false, false);
        check(Some(31.0), Duration::from_millis(15_500), true, true);
        check(Some(600.0), Duration::from_secs(239), false, false);
        check(Some(600.0), Duration::from_secs(240), true, true);
        check(None, Duration::from_secs(239), false, false);
        check(None, Duration::from_secs(240), true, true);
        check(Some(0.0), Duration::from_secs(240), true, true);
    }

    fn last_fm(base_url: String) -> LastFmClient {
        let mut client = LastFmClient::new("api-key".into(), "api-secret".into()).unwrap();
        client.base_url = base_url;
        client.session_key = Some("session-key".into());
        client
    }

    fn listen_brainz(base_url: String) -> ListenBrainzClient {
        let mut client =
            ListenBrainzClient::with_base_url(base_url.trim_end_matches('/').to_string());
        client.session_key = Some("session-key".into());
        client
    }

    #[tokio::test]
    async fn scrobble_all_submits_a_qualifying_listen_once_to_each_service() {
        let (lastfm_url, lastfm_count, mut lastfm_requests, lastfm_server) =
            spawn_persistent_server((
                "200 OK",
                r#"{"scrobbles":{"@attr":{"accepted":1,"ignored":0}}}"#.to_string(),
            ))
            .await;
        let (listenbrainz_url, listenbrainz_count, mut listenbrainz_requests, listenbrainz_server) =
            spawn_persistent_server(("200 OK", r#"{"status":"ok"}"#.to_string())).await;

        let mut manager = ScrobblingManager::new(2, Duration::from_millis(1));
        manager.lastfm = Some(last_fm(lastfm_url));
        manager.listenbrainz = Some(listen_brainz(listenbrainz_url));

        manager.scrobble_all(track_with_duration(180.0), Duration::from_secs(180));

        timeout(Duration::from_secs(5), lastfm_requests.recv())
            .await
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(5), listenbrainz_requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            timeout(Duration::from_millis(500), lastfm_requests.recv())
                .await
                .is_err(),
            "Last.fm received a duplicate submission"
        );
        assert!(
            timeout(Duration::from_millis(500), listenbrainz_requests.recv())
                .await
                .is_err(),
            "ListenBrainz received a duplicate submission"
        );
        assert_eq!(lastfm_count.load(Ordering::SeqCst), 1);
        assert_eq!(listenbrainz_count.load(Ordering::SeqCst), 1);

        lastfm_server.abort();
        listenbrainz_server.abort();
    }

    #[tokio::test]
    async fn scrobble_all_submits_a_short_track_only_to_listen_brainz() {
        let (lastfm_url, lastfm_count, mut lastfm_requests, lastfm_server) =
            spawn_persistent_server((
                "200 OK",
                r#"{"scrobbles":{"@attr":{"accepted":1,"ignored":0}}}"#.to_string(),
            ))
            .await;
        let (listenbrainz_url, listenbrainz_count, mut listenbrainz_requests, listenbrainz_server) =
            spawn_persistent_server(("200 OK", r#"{"status":"ok"}"#.to_string())).await;

        let mut manager = ScrobblingManager::new(2, Duration::from_millis(1));
        manager.lastfm = Some(last_fm(lastfm_url));
        manager.listenbrainz = Some(listen_brainz(listenbrainz_url));

        manager.scrobble_all(track_with_duration(20.0), Duration::from_secs(20));

        timeout(Duration::from_secs(5), listenbrainz_requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(listenbrainz_count.load(Ordering::SeqCst), 1);
        assert!(
            timeout(Duration::from_millis(500), lastfm_requests.recv())
                .await
                .is_err(),
            "Last.fm must not receive a track of 30 seconds or less"
        );
        assert_eq!(lastfm_count.load(Ordering::SeqCst), 0);

        lastfm_server.abort();
        listenbrainz_server.abort();
    }

    #[tokio::test]
    async fn scrobble_all_does_not_submit_an_unqualified_listen() {
        let (lastfm_url, lastfm_count, mut lastfm_requests, lastfm_server) =
            spawn_persistent_server((
                "200 OK",
                r#"{"scrobbles":{"@attr":{"accepted":1,"ignored":0}}}"#.to_string(),
            ))
            .await;
        let (listenbrainz_url, listenbrainz_count, mut listenbrainz_requests, listenbrainz_server) =
            spawn_persistent_server(("200 OK", r#"{"status":"ok"}"#.to_string())).await;

        let mut manager = ScrobblingManager::new(2, Duration::from_millis(1));
        manager.lastfm = Some(last_fm(lastfm_url));
        manager.listenbrainz = Some(listen_brainz(listenbrainz_url));

        manager.scrobble_all(track_with_duration(180.0), Duration::from_secs(4));

        assert!(
            timeout(Duration::from_millis(500), lastfm_requests.recv())
                .await
                .is_err(),
            "Last.fm must not receive an unqualified listen"
        );
        assert!(
            timeout(Duration::from_millis(500), listenbrainz_requests.recv())
                .await
                .is_err(),
            "ListenBrainz must not receive an unqualified listen"
        );
        assert_eq!(lastfm_count.load(Ordering::SeqCst), 0);
        assert_eq!(listenbrainz_count.load(Ordering::SeqCst), 0);

        lastfm_server.abort();
        listenbrainz_server.abort();
    }
}
