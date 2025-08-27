use std::sync::Arc;

use axum::{Json, extract::State};

use super::register::AppError;
use crate::server::ServerState;
use discovery::server::UserSummary;

#[axum::debug_handler]
pub async fn list_users_handler(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<Vec<UserSummary>>, AppError> {
    let users = state.permission_manager.read().await.list_users().await;
    Ok(Json(users))
}
