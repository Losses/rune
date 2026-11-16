use std::sync::Arc;

use anyhow::{Context, Result};
use database::credentials::{Credential, CredentialRef, CredentialStore, Secret};
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

async fn credential_for_request(
    store: &CredentialStore,
    request: &LoginRequestItem,
) -> Result<(CredentialRef, Credential, Option<String>)> {
    let (secrets, replaces) = match &request.secret_input {
        ScrobbleSecretInput::Saved { credential_ref } => {
            let reference = CredentialRef(
                Uuid::parse_str(credential_ref).context("Invalid credential reference")?,
            );
            let credential = store.get(reference).await?;
            return Ok((reference, credential, None));
        }
        ScrobbleSecretInput::Password {
            password,
            api_credentials,
            replaces,
        } => (
            ScrobbleSecrets {
                password: password.clone(),
                api_credentials: api_credentials.clone(),
            },
            replaces.clone(),
        ),
    };
    let bytes = serde_json::to_vec(&secrets).context("Encode scrobble credentials")?;
    let reference = store
        .put(Credential {
            username: request.username.clone(),
            secret: Secret::new(bytes),
        })
        .await?;
    let credential = store.get(reference).await?;
    Ok((reference, credential, replaces))
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

        let (reference, credential, replaces) =
            match credential_for_request(&node.credentials, request).await {
                Ok(credential) => credential,
                Err(_) => return Err(credentials_failure()),
            };
        let secrets = match secrets_from_credential(&credential) {
            Ok(secrets) => secrets,
            Err(_) => {
                let _ = node.credentials.remove(reference).await;
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
                &request.username,
                &secrets.password,
                api_key,
                api_secret,
                false,
            )
            .await;
        match result {
            Ok(()) => {
                if let Some(previous_reference) = replaces
                    && let Ok(id) = Uuid::parse_str(&previous_reference)
                {
                    let _ = node.credentials.remove(CredentialRef(id)).await;
                }
                Ok(Some(AuthenticateSingleServiceResponse {
                    call_id: None,
                    credential_ref: reference.to_string(),
                }))
            }
            Err(error) => {
                if matches!(request.secret_input, ScrobbleSecretInput::Password { .. }) {
                    let _ = node.credentials.remove(reference).await;
                }
                Err(ExpectedRequestError::ScrobbleAuthentication {
                    failure: authentication_failure(error, &secrets),
                }
                .into())
            }
        }
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
        ScrobbleSecrets, credential_for_request, redact_secrets, remove_credential,
        secrets_from_credential,
    };
    use crate::messages::{LoginRequestItem, ScrobbleApiCredentials, ScrobbleSecretInput};
    use database::credentials::CredentialStore;

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
    async fn password_login_stores_secrets_and_returns_only_a_reference() {
        let directory = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(directory.path());
        let request = password_login();

        let (reference, credential, _) = credential_for_request(&store, &request).await.unwrap();
        let secrets = secrets_from_credential(&credential).unwrap();
        assert_eq!(secrets.password, "unique-test-password");
        assert!(matches!(
            secrets.api_credentials,
            ScrobbleApiCredentials::LastFm { api_key, api_secret }
                if api_key == "unique-test-api-key" && api_secret == "unique-test-api-secret"
        ));
        assert_eq!(credential.username, "listener");
        assert!(matches!(
            request.secret_input,
            ScrobbleSecretInput::Password { .. }
        ));
        assert!(store.get(reference).await.is_ok());
        let encrypted = std::fs::read(directory.path().join("credentials.enc")).unwrap();
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
            credential_for_request(&store, &password_login())
                .await
                .unwrap()
                .0
        };
        let reopened = CredentialStore::open(directory.path());
        let request = LoginRequestItem {
            service_id: "ListenBrainz".into(),
            username: "listener".into(),
            secret_input: ScrobbleSecretInput::Saved {
                credential_ref: reference.to_string(),
            },
        };

        let (restored_reference, credential, _) =
            credential_for_request(&reopened, &request).await.unwrap();
        assert_eq!(restored_reference, reference);
        assert_eq!(
            secrets_from_credential(&credential).unwrap().password,
            "unique-test-password"
        );
    }

    #[tokio::test]
    async fn logout_removes_the_referenced_credential() {
        let directory = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(directory.path());
        let (reference, _, _) = credential_for_request(&store, &password_login())
            .await
            .unwrap();

        remove_credential(&store, &reference.to_string())
            .await
            .unwrap();
        assert!(store.get(reference).await.is_err());
    }
}
