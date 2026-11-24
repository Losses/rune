use std::sync::Arc;

use axum::{Json, extract::Extension};

use super::register::AppError;
use crate::server::ServerManager;

#[derive(serde::Serialize)]
pub struct RefreshResponse {
    token: String,
}

pub async fn refresh_handler(
    Extension(server_manager): Extension<Arc<ServerManager>>,
) -> Result<Json<RefreshResponse>, AppError> {
    let token = server_manager
        .generate_jwt_token(None)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(RefreshResponse { token }))
}
