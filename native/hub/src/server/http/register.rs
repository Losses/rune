use std::{net::SocketAddr, str::FromStr, sync::Arc};

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{Response, StatusCode},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use discovery::{
    server::{PermissionError, UserStatus},
    utils::DeviceType,
};

use crate::server::ServerState;

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    public_key: String,
    fingerprint: String,
    alias: String,
    device_model: String,
    device_type: String,
}

/// To test this API, use:
/// curl -v http://localhost:7863/register \
///  -H "Content-Type: application/json" \
///  -d '{
///    "public_key": "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA",
///    "fingerprint": "01:23:45:67:89:AB:CD:EF",
///    "alias": "Test Device",
///    "device_model": "NixOS Device",
///    "device_type": "Desktop"
///  }'
pub async fn register_handler(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<ServerState>>,
    Json(request): Json<RegisterRequest>,
) -> Result<impl IntoResponse, AppError> {
    let ip = addr.ip().to_string();
    {
        let permission_manager = state.permission_manager.read().await;
        let user = permission_manager
            .verify_by_fingerprint(&request.fingerprint)
            .await;

        if let Some(user) = user
            && user.status == UserStatus::Blocked
        {
            return Ok(StatusCode::FORBIDDEN);
        }
    }

    {
        state
            .permission_manager
            .write()
            .await
            .add_user(
                request.public_key,
                request.fingerprint,
                request.alias,
                request.device_model,
                DeviceType::from_str(&request.device_type)?,
                ip,
            )
            .await?;
    }

    Ok(StatusCode::CREATED)
}

#[derive(Debug)]
pub enum AppError {
    Permission(PermissionError),
    ParseDevice(discovery::utils::ParseDeviceTypeError),
    Internal(String),
    NotFound(String),
    Unauthorized(String),
}

impl From<PermissionError> for AppError {
    fn from(e: PermissionError) -> Self {
        AppError::Permission(e)
    }
}

impl From<discovery::utils::ParseDeviceTypeError> for AppError {
    fn from(e: discovery::utils::ParseDeviceTypeError) -> Self {
        AppError::ParseDevice(e)
    }
}

#[derive(Serialize)]
struct ErrorResponse {
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response<axum::body::Body> {
        let (status, message) = match self {
            AppError::Permission(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            AppError::ParseDevice(e) => (StatusCode::BAD_REQUEST, e.to_string()),
            AppError::Internal(e) => (StatusCode::INTERNAL_SERVER_ERROR, e),
            AppError::NotFound(e) => (StatusCode::NOT_FOUND, e),
            AppError::Unauthorized(e) => (StatusCode::UNAUTHORIZED, e),
        };

        let body = Json(ErrorResponse { message });

        (status, body).into_response()
    }
}
