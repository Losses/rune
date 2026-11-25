use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use discovery::server::UserStatus;

use crate::server::ServerState;

use super::register::AppError;

#[derive(Deserialize)]
pub struct StatusUpdate {
    status: UserStatus,
}

pub async fn update_user_status_handler(
    Path(fingerprint): Path<String>,
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<StatusUpdate>,
) -> Result<StatusCode, AppError> {
    state
        .permission_manager
        .write()
        .await
        .change_user_status(&fingerprint, payload.status)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
