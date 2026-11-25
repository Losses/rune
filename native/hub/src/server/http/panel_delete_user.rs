use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
};

use log::info;
use serde_json::json;

use crate::server::ServerState;
use crate::server::api::remove_trusted_device;

use super::register::AppError;

pub async fn delete_user_handler(
    Path(fingerprint): Path<String>,
    State(server_state): State<Arc<ServerState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    info!("Deleting user {fingerprint}");

    let node = server_state
        .node
        .upgrade()
        .ok_or_else(|| AppError::Internal("Node is shutting down".into()))?;
    if node
        .permission_manager
        .read()
        .await
        .verify_by_fingerprint(&fingerprint)
        .await
        .is_none()
    {
        return Err(AppError::NotFound(format!("User {fingerprint} not found")));
    }
    let permissions = node.permission_manager.read().await;
    let validator = node.cert_validator.read().await;
    remove_trusted_device(&permissions, &validator, &fingerprint)
        .await
        .map_err(|error| AppError::Internal(format!("{error:#}")))?;
    drop(validator);
    drop(permissions);
    node.sync_tokens.remove_peer(&fingerprint);
    node.revoke_outgoing_for(&fingerprint).await;

    Ok(Json(json!({
        "success": true,
        "message": format!("User {} deleted successfully", fingerprint)
    })))
}
