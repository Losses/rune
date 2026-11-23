use std::sync::Arc;

use anyhow::{Context, Result};
use database::credentials::{
    Credential, CredentialRef, CredentialStore, CredentialStoreError, Secret,
};
use scrobbling::manager::{ScrobblingCredential, ScrobblingManager, ScrobblingServiceManager};
use scrobbling::{
    LastFmApiCredentialField as ManagerLastFmApiCredentialField, ScrobblingAuthFailure,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::request_error::ExpectedRequestError;
use crate::{
    Session, Signal,
    messages::*,
    node::NodeContext,
    utils::{GlobalParams, ParamsExtractor},
};

#[derive(Serialize, Deserialize)]
struct ScrobbleSecrets {
    password: String,
    api_credentials: ScrobbleApiCredentials,
}

struct PendingCredential {
    /// The reference already saved in settings, for a login that reuses it.
    existing: Option<CredentialRef>,
    /// The reference this new login replaces, when settings named one.
    replaces: Option<CredentialRef>,
    credential: Credential,
}

async fn credential_for_request(
    store: &CredentialStore,
    request: &LoginRequestItem,
) -> Result<PendingCredential> {
    match &request.secret_input {
        ScrobbleSecretInput::Saved { credential_ref } => {
            let reference = CredentialRef(
                Uuid::parse_str(credential_ref).context("Invalid credential reference")?,
            );
            let credential = store.get(reference).await?;
            Ok(PendingCredential {
                existing: Some(reference),
                replaces: None,
                credential,
            })
        }
        ScrobbleSecretInput::Password {
            password,
            api_credentials,
            replaces,
        } => {
            let secrets = ScrobbleSecrets {
                password: password.clone(),
                api_credentials: api_credentials.clone(),
            };
            let bytes = serde_json::to_vec(&secrets).context("Encode scrobble credentials")?;
            Ok(PendingCredential {
                existing: None,
                replaces: replaces
                    .as_deref()
                    .and_then(|reference| Uuid::parse_str(reference).ok())
                    .map(CredentialRef),
                credential: Credential {
                    username: request.username.clone(),
                    secret: Secret::new(bytes),
                },
            })
        }
    }
}

/// Writes a credential that has already authenticated. A replacement keeps the
/// reference settings already hold, so a lost response or a failed settings
/// write cannot leave that reference pointing at a removed credential.
async fn store_authenticated_credential(
    store: &CredentialStore,
    pending: PendingCredential,
) -> Result<CredentialRef> {
    if let Some(reference) = pending.existing {
        return Ok(reference);
    }
    if let Some(reference) = pending.replaces {
        match store.replace(reference, &pending.credential).await {
            Ok(()) => return Ok(reference),
            Err(CredentialStoreError::CredentialMissing(_)) => {
                // Settings held a reference the store no longer has; store a
                // fresh credential so the login can still be saved.
            }
            Err(_) => return Err(credentials_failure()),
        }
    }
    store
        .put(pending.credential)
        .await
        .map_err(|_| credentials_failure())
}

async fn authenticate_and_store<M>(
    store: &CredentialStore,
    scrobbler: &Mutex<M>,
    request: &LoginRequestItem,
) -> Result<CredentialRef>
where
    M: ScrobblingServiceManager + ?Sized,
{
    let pending = match credential_for_request(store, request).await {
        Ok(pending) => pending,
        Err(_) => return Err(credentials_failure()),
    };
    let secrets = match secrets_from_credential(&pending.credential) {
        Ok(secrets) => secrets,
        Err(_) => {
            if let Some(reference) = pending.existing {
                let _ = store.remove(reference).await;
            }
            return Err(credentials_failure());
        }
    };

    let (api_key, api_secret) = match &secrets.api_credentials {
        ScrobbleApiCredentials::None => (None, None),
        ScrobbleApiCredentials::LastFm {
            api_key,
            api_secret,
        } => (Some(api_key.clone()), Some(api_secret.clone())),
    };
    let result = scrobbler
        .lock()
        .await
        .authenticate(
            &request.service_id.clone().into(),
            &pending.credential.username,
            &secrets.password,
            api_key,
            api_secret,
            false,
        )
        .await;
    match result {
        Ok(()) => store_authenticated_credential(store, pending).await,
        Err(error) => Err(ExpectedRequestError::ScrobbleAuthentication {
            failure: authentication_failure(error, &secrets),
        }
        .into()),
    }
}

fn secrets_from_credential(credential: &Credential) -> Result<ScrobbleSecrets> {
    serde_json::from_slice(credential.secret.as_bytes()).context("Read scrobble credentials")
}

fn redact_secrets(mut message: String, secrets: &ScrobbleSecrets) -> String {
    let mut values = vec![secrets.password.as_str()];
    if let ScrobbleApiCredentials::LastFm {
        api_key,
        api_secret,
    } = &secrets.api_credentials
    {
        values.push(api_key);
        values.push(api_secret);
    }
    for secret in values {
        if !secret.is_empty() {
            message = message.replace(secret, "<redacted>");
        }
    }
    message
}

async fn remove_credential(store: &CredentialStore, credential_ref: &str) -> Result<()> {
    let reference =
        CredentialRef(Uuid::parse_str(credential_ref).context("Invalid credential reference")?);
    store.remove(reference).await?;
    Ok(())
}

fn authentication_failure(
    error: anyhow::Error,
    secrets: &ScrobbleSecrets,
) -> ScrobbleAuthenticationFailure {
    if let Some(failure) = error.downcast_ref::<ScrobblingAuthFailure>() {
        return match failure {
            ScrobblingAuthFailure::CredentialsRejected { .. } => {
                ScrobbleAuthenticationFailure::CredentialsRejected
            }
            ScrobblingAuthFailure::ServiceUnreachable { .. } => {
                ScrobbleAuthenticationFailure::ServiceUnreachable
            }
            ScrobblingAuthFailure::LastFmApiCredentialsMissing(field) => {
                ScrobbleAuthenticationFailure::LastFmApiCredentialsMissing {
                    field: match field {
                        ManagerLastFmApiCredentialField::ApiKey => LastFmApiCredentialField::ApiKey,
                        ManagerLastFmApiCredentialField::ApiSecret => {
                            LastFmApiCredentialField::ApiSecret
                        }
                    },
                }
            }
            ScrobblingAuthFailure::LastFmApiCredentialsInvalid { .. } => {
                ScrobbleAuthenticationFailure::LastFmApiCredentialsInvalid
            }
            ScrobblingAuthFailure::Other { detail } => ScrobbleAuthenticationFailure::Other {
                detail: redact_secrets(detail.clone(), secrets),
            },
        };
    }
    ScrobbleAuthenticationFailure::Other {
        detail: redact_secrets(format!("{error:#}"), secrets),
    }
}

fn credentials_failure() -> anyhow::Error {
    ExpectedRequestError::ScrobbleAuthentication {
        failure: ScrobbleAuthenticationFailure::Other {
            detail: "Could not read or store scrobble credentials".into(),
        },
    }
    .into()
}

impl ParamsExtractor for AuthenticateSingleServiceRequest {
    type Params = (Arc<NodeContext>, Arc<Mutex<dyn ScrobblingServiceManager>>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.node),
            Arc::clone(&all_params.scrobbler),
        )
    }
}

impl Signal for AuthenticateSingleServiceRequest {
    type Params = (Arc<NodeContext>, Arc<Mutex<dyn ScrobblingServiceManager>>);
    type Response = AuthenticateSingleServiceResponse;

    async fn handle(
        &self,
        (node, scrobbler): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let Some(request) = &dart_signal.request else {
            return Ok(None);
        };

        let reference = authenticate_and_store(&node.credentials, &scrobbler, request).await?;
        Ok(Some(AuthenticateSingleServiceResponse {
            call_id: None,
            credential_ref: reference.to_string(),
        }))
    }
}

impl ParamsExtractor for AuthenticateMultipleServiceRequest {
    type Params = (Arc<NodeContext>, Arc<Mutex<dyn ScrobblingServiceManager>>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.node),
            Arc::clone(&all_params.scrobbler),
        )
    }
}

impl Signal for AuthenticateMultipleServiceRequest {
    type Params = (Arc<NodeContext>, Arc<Mutex<dyn ScrobblingServiceManager>>);
    type Response = ();

    async fn handle(
        &self,
        (node, scrobbler): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let mut authentications = Vec::new();
        for request in &dart_signal.requests {
            let ScrobbleSecretInput::Saved { credential_ref } = &request.secret_input else {
                continue;
            };
            let reference = match Uuid::parse_str(credential_ref) {
                Ok(reference) => CredentialRef(reference),
                Err(_) => {
                    log::warn!("Could not read saved scrobble credentials");
                    continue;
                }
            };
            match node.credentials.get(reference).await {
                Ok(credential) => {
                    let secrets = match secrets_from_credential(&credential) {
                        Ok(secrets) => secrets,
                        Err(_) => {
                            log::warn!("Could not read saved scrobble credentials");
                            continue;
                        }
                    };
                    let (api_key, api_secret) = match secrets.api_credentials {
                        ScrobbleApiCredentials::None => (None, None),
                        ScrobbleApiCredentials::LastFm {
                            api_key,
                            api_secret,
                        } => (Some(api_key), Some(api_secret)),
                    };
                    authentications.push(ScrobblingCredential {
                        service: request.service_id.clone().into(),
                        username: credential.username.clone(),
                        password: secrets.password,
                        api_key,
                        api_secret,
                    });
                }
                Err(_) => log::warn!("Could not read saved scrobble credentials"),
            }
        }
        ScrobblingManager::authenticate_all(scrobbler, authentications);
        Ok(None)
    }
}

impl ParamsExtractor for LogoutSingleServiceRequest {
    type Params = (Arc<NodeContext>, Arc<Mutex<dyn ScrobblingServiceManager>>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.node),
            Arc::clone(&all_params.scrobbler),
        )
    }
}

impl Signal for LogoutSingleServiceRequest {
    type Params = (Arc<NodeContext>, Arc<Mutex<dyn ScrobblingServiceManager>>);
    type Response = ();

    async fn handle(
        &self,
        (node, scrobbler): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        scrobbler
            .lock()
            .await
            .logout(dart_signal.service_id.clone().into())
            .await;
        remove_credential(&node.credentials, &dart_signal.credential_ref).await?;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ScrobbleSecrets, authenticate_and_store, credential_for_request, redact_secrets,
        remove_credential, secrets_from_credential,
    };
    use crate::messages::{
        LoginRequestItem, RequestFailureCause, ScrobbleApiCredentials,
        ScrobbleAuthenticationFailure, ScrobbleSecretInput,
    };
    use crate::request_error::request_failure_cause;
    use anyhow::Result;
    use async_trait::async_trait;
    use database::credentials::{Credential, CredentialRef, CredentialStore, Secret};
    use scrobbling::ScrobblingTrack;
    use scrobbling::manager::{
        LoginStatus, MockScrobblingManager, ScrobblingError, ScrobblingService,
        ScrobblingServiceManager,
    };
    use simple_channel::{SimpleReceiver, SimpleSender};
    use std::fs;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    fn password_login() -> LoginRequestItem {
        LoginRequestItem {
            service_id: "LastFm".into(),
            username: "listener".into(),
            secret_input: ScrobbleSecretInput::Password {
                password: "unique-test-password".into(),
                api_credentials: ScrobbleApiCredentials::LastFm {
                    api_key: "unique-test-api-key".into(),
                    api_secret: "unique-test-api-secret".into(),
                },
                replaces: None,
            },
        }
    }

    fn credential(username: &str, password: &str) -> Credential {
        let secrets = ScrobbleSecrets {
            password: password.into(),
            api_credentials: ScrobbleApiCredentials::None,
        };
        Credential {
            username: username.into(),
            secret: Secret::new(serde_json::to_vec(&secrets).unwrap()),
        }
    }

    fn password_login_for(
        username: &str,
        password: &str,
        replaces: Option<CredentialRef>,
    ) -> LoginRequestItem {
        LoginRequestItem {
            service_id: "ListenBrainz".into(),
            username: username.into(),
            secret_input: ScrobbleSecretInput::Password {
                password: password.into(),
                api_credentials: ScrobbleApiCredentials::None,
                replaces: replaces.map(|reference| reference.to_string()),
            },
        }
    }

    struct TestScrobbler {
        inner: MockScrobblingManager,
        reject: bool,
        usernames: Vec<String>,
    }

    impl TestScrobbler {
        fn accepting() -> Self {
            Self {
                inner: MockScrobblingManager::new(),
                reject: false,
                usernames: Vec::new(),
            }
        }

        fn rejecting() -> Self {
            Self {
                inner: MockScrobblingManager::new(),
                reject: true,
                usernames: Vec::new(),
            }
        }
    }

    #[async_trait]
    impl ScrobblingServiceManager for TestScrobbler {
        async fn send_login_status(&self) {
            self.inner.send_login_status().await;
        }

        async fn authenticate(
            &mut self,
            service: &ScrobblingService,
            username: &str,
            password: &str,
            api_key: Option<String>,
            api_secret: Option<String>,
            enable_retry: bool,
        ) -> Result<()> {
            self.usernames.push(username.to_owned());
            if self.reject {
                return Err(scrobbling::ScrobblingAuthFailure::CredentialsRejected {
                    detail: "rejected".into(),
                }
                .into());
            }
            self.inner
                .authenticate(
                    service,
                    username,
                    password,
                    api_key,
                    api_secret,
                    enable_retry,
                )
                .await
        }

        async fn update_now_playing(
            &mut self,
            service: &ScrobblingService,
            track: ScrobblingTrack,
        ) {
            self.inner.update_now_playing(service, track).await;
        }

        fn restore_session(
            &mut self,
            service: &ScrobblingService,
            session_key: String,
        ) -> Result<()> {
            self.inner.restore_session(service, session_key)
        }

        fn update_now_playing_all(&mut self, track: ScrobblingTrack) {
            self.inner.update_now_playing_all(track);
        }

        async fn scrobble(
            &mut self,
            service: ScrobblingService,
            track: ScrobblingTrack,
            listened: Duration,
        ) {
            self.inner.scrobble(service, track, listened).await;
        }

        fn scrobble_all(&mut self, track: ScrobblingTrack, listened: Duration) {
            self.inner.scrobble_all(track, listened);
        }

        async fn logout(&mut self, service: ScrobblingService) {
            self.inner.logout(service).await;
        }

        fn subscribe_error(&self) -> SimpleReceiver<ScrobblingError> {
            self.inner.subscribe_error()
        }

        fn subscribe_login_status(&self) -> SimpleReceiver<Vec<LoginStatus>> {
            self.inner.subscribe_login_status()
        }

        fn error_sender(&self) -> Arc<SimpleSender<ScrobblingError>> {
            self.inner.error_sender()
        }
    }

    #[test]
    fn authentication_errors_keep_details_but_redact_submitted_secrets() {
        let secrets = ScrobbleSecrets {
            password: "password-value".into(),
            api_credentials: ScrobbleApiCredentials::LastFm {
                api_key: "api-key-value".into(),
                api_secret: "api-secret-value".into(),
            },
        };

        let message = redact_secrets(
            "Rejected password-value with api-key-value and api-secret-value".into(),
            &secrets,
        );

        assert_eq!(
            message,
            "Rejected <redacted> with <redacted> and <redacted>"
        );
    }

    #[tokio::test]
    async fn password_login_is_not_stored_before_authentication() {
        let directory = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(directory.path());

        let pending = credential_for_request(&store, &password_login())
            .await
            .unwrap();

        assert!(pending.existing.is_none());
        assert!(
            !directory.path().join("credentials.enc").exists(),
            "a password login must not be stored before authentication succeeds"
        );
    }

    #[tokio::test]
    async fn password_login_stores_secrets_and_returns_only_a_reference() {
        let directory = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(directory.path());
        let scrobbler = Mutex::new(TestScrobbler::accepting());

        let reference = authenticate_and_store(&store, &scrobbler, &password_login())
            .await
            .unwrap();
        let credential = store.get(reference).await.unwrap();
        let secrets = secrets_from_credential(&credential).unwrap();
        assert_eq!(secrets.password, "unique-test-password");
        assert!(matches!(
            secrets.api_credentials,
            ScrobbleApiCredentials::LastFm { api_key, api_secret }
                if api_key == "unique-test-api-key" && api_secret == "unique-test-api-secret"
        ));
        assert_eq!(credential.username, "listener");
        let encrypted = fs::read(directory.path().join("credentials.enc")).unwrap();
        assert!(
            !encrypted
                .windows(b"unique-test-password".len())
                .any(|window| window == b"unique-test-password")
        );
        assert!(
            !encrypted
                .windows(b"unique-test-api-key".len())
                .any(|window| window == b"unique-test-api-key")
        );
    }

    #[tokio::test]
    async fn stored_reference_restores_after_reopening_the_store() {
        let directory = tempfile::tempdir().unwrap();
        let reference = {
            let store = CredentialStore::open(directory.path());
            let scrobbler = Mutex::new(TestScrobbler::accepting());
            authenticate_and_store(&store, &scrobbler, &password_login())
                .await
                .unwrap()
        };
        let reopened = CredentialStore::open(directory.path());
        let request = LoginRequestItem {
            service_id: "ListenBrainz".into(),
            username: "listener".into(),
            secret_input: ScrobbleSecretInput::Saved {
                credential_ref: reference.to_string(),
            },
        };

        let pending = credential_for_request(&reopened, &request).await.unwrap();
        assert_eq!(pending.existing, Some(reference));
        assert_eq!(
            secrets_from_credential(&pending.credential)
                .unwrap()
                .password,
            "unique-test-password"
        );
    }

    #[tokio::test]
    async fn successful_replacement_persists_the_new_login_across_restart() {
        let directory = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(directory.path());
        let previous = store
            .put(credential("alice", "old-password"))
            .await
            .unwrap();
        let scrobbler = Mutex::new(TestScrobbler::accepting());
        let request = password_login_for("bob", "new-password", Some(previous));

        let reference = authenticate_and_store(&store, &scrobbler, &request)
            .await
            .unwrap();
        drop(store);

        let reopened = CredentialStore::open(directory.path());
        let saved = reopened.get(reference).await.unwrap();
        assert_eq!(saved.username, "bob");
        assert_eq!(
            secrets_from_credential(&saved).unwrap().password,
            "new-password"
        );
    }

    #[tokio::test]
    async fn interrupted_replacement_keeps_the_saved_reference_usable() {
        let directory = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(directory.path());
        let previous = store
            .put(credential("alice", "old-password"))
            .await
            .unwrap();
        let scrobbler = Mutex::new(TestScrobbler::accepting());
        let request = password_login_for("bob", "new-password", Some(previous));

        authenticate_and_store(&store, &scrobbler, &request)
            .await
            .unwrap();
        // The app stops before Dart writes the response to settings, so
        // settings still contain the reference from before the login.
        drop(store);

        let reopened = CredentialStore::open(directory.path());
        let saved = reopened
            .get(previous)
            .await
            .expect("the saved reference must stay usable when a replacement is interrupted");
        assert_eq!(saved.username, "bob");
        assert_eq!(
            secrets_from_credential(&saved).unwrap().password,
            "new-password"
        );
    }

    #[tokio::test]
    async fn failed_settings_save_leaves_the_saved_reference_valid() {
        let directory = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(directory.path());
        let previous = store
            .put(credential("alice", "old-password"))
            .await
            .unwrap();
        let scrobbler = Mutex::new(TestScrobbler::accepting());
        let request = password_login_for("bob", "new-password", Some(previous));

        let reference = authenticate_and_store(&store, &scrobbler, &request)
            .await
            .unwrap();

        assert_eq!(
            reference, previous,
            "a failed settings write must leave the already saved reference valid"
        );
        let saved = store.get(previous).await.unwrap();
        assert_eq!(saved.username, "bob");
        assert_eq!(
            secrets_from_credential(&saved).unwrap().password,
            "new-password"
        );
    }

    #[tokio::test]
    async fn rejected_replacement_preserves_the_saved_login() {
        let directory = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(directory.path());
        let previous = store
            .put(credential("alice", "old-password"))
            .await
            .unwrap();
        let store_len = fs::metadata(directory.path().join("credentials.enc"))
            .unwrap()
            .len();
        let scrobbler = Mutex::new(TestScrobbler::rejecting());
        let request = password_login_for("bob", "new-password", Some(previous));

        let error = authenticate_and_store(&store, &scrobbler, &request)
            .await
            .unwrap_err();

        assert_eq!(
            request_failure_cause(&error),
            Some(RequestFailureCause::ScrobbleAuthentication {
                failure: ScrobbleAuthenticationFailure::CredentialsRejected,
            })
        );
        let saved = store.get(previous).await.unwrap();
        assert_eq!(saved.username, "alice");
        assert_eq!(
            secrets_from_credential(&saved).unwrap().password,
            "old-password"
        );
        assert_eq!(
            fs::metadata(directory.path().join("credentials.enc"))
                .unwrap()
                .len(),
            store_len,
            "a rejected candidate must not stay in the store"
        );
    }

    #[tokio::test]
    async fn replacement_with_a_stale_saved_reference_stores_a_fresh_credential() {
        let directory = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(directory.path());
        let stale = CredentialRef(Uuid::new_v4());
        let scrobbler = Mutex::new(TestScrobbler::accepting());
        let request = password_login_for("bob", "new-password", Some(stale));

        let reference = authenticate_and_store(&store, &scrobbler, &request)
            .await
            .unwrap();

        assert_ne!(reference, stale);
        let saved = store.get(reference).await.unwrap();
        assert_eq!(saved.username, "bob");
        assert_eq!(
            secrets_from_credential(&saved).unwrap().password,
            "new-password"
        );
    }

    #[tokio::test]
    async fn saved_login_authenticates_with_the_stored_username() {
        let directory = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(directory.path());
        let reference = store
            .put(credential("new-listener", "new-password"))
            .await
            .unwrap();
        let request = LoginRequestItem {
            service_id: "ListenBrainz".into(),
            username: "old-listener".into(),
            secret_input: ScrobbleSecretInput::Saved {
                credential_ref: reference.to_string(),
            },
        };
        let scrobbler = Mutex::new(TestScrobbler::accepting());

        let returned = authenticate_and_store(&store, &scrobbler, &request)
            .await
            .unwrap();

        assert_eq!(returned, reference);
        assert_eq!(
            scrobbler.lock().await.usernames.as_slice(),
            ["new-listener".to_owned()]
        );
    }

    #[tokio::test]
    async fn logout_removes_the_referenced_credential() {
        let directory = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(directory.path());
        let scrobbler = Mutex::new(TestScrobbler::accepting());
        let reference = authenticate_and_store(&store, &scrobbler, &password_login())
            .await
            .unwrap();

        remove_credential(&store, &reference.to_string())
            .await
            .unwrap();
        assert!(store.get(reference).await.is_err());
    }
}
