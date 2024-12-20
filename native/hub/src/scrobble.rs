use std::sync::Arc;

use anyhow::Result;
use rinf::DartSignal;
use tokio::sync::Mutex;

use scrobbling::manager::{ScrobblingCredential, ScrobblingManager};

use crate::{
    AuthenticateMultipleServiceRequest, AuthenticateSingleServiceRequest,
    AuthenticateSingleServiceResponse,
};

pub async fn authenticate_single_service_request(
    scrobbler: Arc<Mutex<ScrobblingManager>>,
    dart_signal: DartSignal<AuthenticateSingleServiceRequest>,
) -> Result<()> {
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

        match result {
            Ok(_) => AuthenticateSingleServiceResponse {
                success: true,
                error: None,
            }
            .send_signal_to_dart(),
            Err(e) => AuthenticateSingleServiceResponse {
                success: false,
                error: format!("{:#?}", e).into(),
            }
            .send_signal_to_dart(),
        }
    }

    Ok(())
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
