use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;

use ::scrobbling::manager::{ScrobblingCredential, ScrobblingManager};

use crate::{
    messages::*,
    utils::{GlobalParams, ParamsExtractor},
    Signal,
};

impl ParamsExtractor for AuthenticateSingleServiceRequest {
    type Params = (Arc<Mutex<ScrobblingManager>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.scrobbler),)
    }
}

impl Signal for AuthenticateSingleServiceRequest {
    type Params = (Arc<Mutex<ScrobblingManager>>,);
    type Response = AuthenticateSingleServiceResponse;

    async fn handle(
        &self,
        (scrobbler,): Self::Params,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = &dart_signal.request;

        if let Some(request) = request {
            let result = scrobbler
                .lock()
                .await
                .authenticate(
                    &request.service_id.clone().into(),
                    &request.username,
                    &request.password,
                    request.api_key.clone(),
                    request.api_secret.clone(),
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
}

impl ParamsExtractor for AuthenticateMultipleServiceRequest {
    type Params = (Arc<Mutex<ScrobblingManager>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.scrobbler),)
    }
}

impl Signal for AuthenticateMultipleServiceRequest {
    type Params = (Arc<Mutex<ScrobblingManager>>,);
    type Response = ();

    async fn handle(
        &self,
        (scrobbler,): Self::Params,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let requests = &dart_signal.requests;

        ScrobblingManager::authenticate_all(
            scrobbler,
            requests
                .iter()
                .map(|x| ScrobblingCredential {
                    service: x.service_id.clone().into(),
                    username: x.username.clone(),
                    password: x.password.clone(),
                    api_key: x.api_key.clone(),
                    api_secret: x.api_secret.clone(),
                })
                .collect(),
        );

        Ok(None)
    }
}

impl ParamsExtractor for LogoutSingleServiceRequest {
    type Params = (Arc<Mutex<ScrobblingManager>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.scrobbler),)
    }
}

impl Signal for LogoutSingleServiceRequest {
    type Params = (Arc<Mutex<ScrobblingManager>>,);
    type Response = ();

    async fn handle(
        &self,
        (scrobbler,): Self::Params,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        scrobbler
            .lock()
            .await
            .logout(dart_signal.service_id.clone().into())
            .await;

        Ok(None)
    }
}
