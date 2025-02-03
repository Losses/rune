use std::{net::SocketAddr, str::FromStr, sync::Arc};

use axum::{
    extract::{ConnectInfo, State},
    http::{Response, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use discovery::{permission::PermissionError, utils::DeviceType};

use crate::server::ServerState;

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    public_key: String,
    fingerprint: String,
    alias: String,
    device_model: String,
    device_type: String,
}

pub async fn register_handler(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<ServerState>>,
    Json(request): Json<RegisterRequest>,
) -> Result<impl IntoResponse, AppError> {
    let ip = addr.ip().to_string();
    state
        .permission_manager
        .add_user(
            request.public_key,
            request.fingerprint,
            request.alias,
            request.device_model,
            DeviceType::from_str(&request.device_type)?,
            ip,
        )
        .await?;

    Ok(StatusCode::CREATED)
}

#[derive(Debug)]
pub enum AppError {
    Permission(PermissionError),
    ParseDevice(discovery::utils::ParseDeviceTypeError),
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

impl IntoResponse for AppError {
    fn into_response(self) -> Response<axum::body::Body> {
        let (status, message) = match self {
            AppError::Permission(e) => match e {
                PermissionError::UserAlreadyExists => (StatusCode::CONFLICT, e.to_string()),
                _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            },
            AppError::ParseDevice(e) => (StatusCode::BAD_REQUEST, e.to_string()),
        };
        (status, message).into_response()
    }
}
