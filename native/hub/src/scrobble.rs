use std::sync::Arc;

use anyhow::Result;
use rinf::DartSignal;
use tokio::sync::Mutex;

use scrobbling::manager::{ScrobblingCredential, ScrobblingManager};

use crate::{
    AuthenticateMultipleServiceRequest, AuthenticateSingleServiceRequest,
    AuthenticateSingleServiceResponse, LogoutSingleServiceRequest,
};

pub async fn authenticate_single_service_request(
    scrobbler: Arc<Mutex<ScrobblingManager>>,
    dart_signal: DartSignal<AuthenticateSingleServiceRequest>,
) -> Result<Option<AuthenticateSingleServiceResponse>> {
    let request = dart_signal.message.request;

    if let Some(request) = request {
        let result = scrobbler
            .lock()
            .await
            .authenticate(
                &request.service_id.into(),
                &request.username,
                &request.password,
                request.api_key,
                request.api_secret,
                false,
            )
            .await;

        let response = match result {
            Ok(_) => AuthenticateSingleServiceResponse {
                success: true,
                error: None,
            },
            Err(e) => AuthenticateSingleServiceResponse {
                success: false,
                error: format!("{:#?}", e).into(),
            },
        };

        return Ok(Some(response));
    }

    Ok(None)
}

pub async fn authenticate_multiple_service_request(
    scrobbler: Arc<Mutex<ScrobblingManager>>,
    dart_signal: DartSignal<AuthenticateMultipleServiceRequest>,
) -> Result<()> {
    let requests = dart_signal.message.requests;

    ScrobblingManager::authenticate_all(
        scrobbler,
        requests
            .into_iter()
            .map(|x| ScrobblingCredential {
                service: x.service_id.into(),
                username: x.username,
                password: x.password,
                api_key: x.api_key,
                api_secret: x.api_secret,
            })
            .collect(),
    );

    Ok(())
}

pub async fn logout_single_service_request(
    scrobbler: Arc<Mutex<ScrobblingManager>>,
    dart_signal: DartSignal<LogoutSingleServiceRequest>,
) -> Result<()> {
    let service_id = dart_signal.message.service_id;

    scrobbler.lock().await.logout(service_id.into()).await;

    Ok(())
}
