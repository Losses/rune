use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

pub trait PinValidationState: Send + Sync {
    fn pin_config(&self) -> &Arc<RwLock<PinConfig>>;
}

impl<T: PinValidationState> PinValidationState for Arc<T> {
    fn pin_config(&self) -> &Arc<RwLock<PinConfig>> {
        (**self).pin_config()
    }
}

#[derive(Clone)]
pub struct PinConfig {
    pub enabled: bool,
    pub pin: String,
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
    S: Send + Sync + PinValidationState,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let pin_config = state.pin_config().read().await;

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
