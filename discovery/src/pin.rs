use std::collections::HashMap;

use axum::{
    extract::{FromRequestParts, State},
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
};

use crate::http_api::AppState;

#[derive(Clone)]
pub struct PinConfig {
    enabled: bool,
    pin: String,
}

impl PinConfig {
    pub fn new(pin: Option<String>) -> Self {
        match pin {
            Some(pin) => Self { enabled: true, pin },
            None => Self {
                enabled: false,
                pin: String::new(),
            },
        }
    }
}

pub struct PinAuth {
    pub validated: bool,
}

impl<S> FromRequestParts<S> for PinAuth
where
    S: Send + Sync,
    AppState: axum::extract::FromRef<S>,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let State(app_state): State<AppState> = State::from_request_parts(parts, state)
            .await
            .map_err(|e| e.into_response())?;

        let pin_config = app_state.pin_config.read().await;

        if !pin_config.enabled {
            return Ok(Self { validated: true });
        }

        let query = parts.uri.query().unwrap_or("");
        let params: HashMap<String, String> = serde_urlencoded::from_str(query)
            .map_err(|_| StatusCode::BAD_REQUEST.into_response())?;

        match params.get("pin") {
            Some(provided_pin) if provided_pin == &pin_config.pin => Ok(Self { validated: true }),
            Some(_) => Err(StatusCode::UNAUTHORIZED.into_response()),
            None => Err((StatusCode::UNAUTHORIZED, "PIN required").into_response()),
        }
    }
}
