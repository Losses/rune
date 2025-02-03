use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{ConnectInfo, State},
    http::{Response, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use discovery::permission::PermissionError;

use crate::server::ServerState;

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    public_key: String,
    fingerprint: String,
    alias: String,
    device_model: String,
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
            ip,
        )
        .await?;

    Ok(StatusCode::CREATED)
}

#[derive(Debug)]
pub struct AppError(PermissionError);

impl From<PermissionError> for AppError {
    fn from(e: PermissionError) -> Self {
        AppError(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response<axum::body::Body> {
        let status = match self.0 {
            PermissionError::UserAlreadyExists => StatusCode::CONFLICT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.0.to_string()).into_response()
    }
}
