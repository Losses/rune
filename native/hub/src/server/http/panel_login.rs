use std::{path::Path, sync::Arc};

use axum::{Extension, Json};
use bcrypt::verify;
use serde::Deserialize;

use crate::server::ServerManager;

use super::register::AppError;

#[derive(Deserialize)]
pub struct LoginRequest {
    password: String,
}

#[derive(serde::Serialize)]
pub struct LoginResponse {
    token: String,
}

pub async fn login_handler(
    Extension(server_manager): Extension<Arc<ServerManager>>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let config_path = Path::new(&*server_manager.global_params.config_path);
    let password_path = config_path.join("root_password.hash");

    if !password_path.exists() {
        return Err(AppError::Unauthorized(
            "Root password not initialized".into(),
        ));
    }

    let stored_hash = tokio::fs::read_to_string(&password_path)
        .await
        .map_err(|_| AppError::Internal("Failed to read password hash".into()))?;

    let valid = verify(&request.password, &stored_hash)
        .map_err(|_| AppError::Internal("Password verification failed".into()))?;

    if !valid {
        return Err(AppError::Unauthorized("Invalid password".into()));
    }

    let token = server_manager
        .generate_jwt_token(None)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(LoginResponse { token }))
}
