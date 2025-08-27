use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
};

use ::discovery::server::PermissionError;
use log::info;
use serde_json::json;

use crate::server::ServerState;

use super::register::AppError;

pub async fn delete_user_handler(
    Path(fingerprint): Path<String>,
    State(server_state): State<Arc<ServerState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    info!("Deleting user {fingerprint}");

    server_state
        .permission_manager
        .write()
        .await
        .remove_user(&fingerprint)
        .await
        .map_err(|e| match e {
            PermissionError::UserNotFound => {
                AppError::NotFound(format!("User {fingerprint} not found"))
            }
            _ => AppError::Internal(e.to_string()),
        })?;

    Ok(Json(json!({
        "success": true,
        "message": format!("User {} deleted successfully", fingerprint)
    })))
}
