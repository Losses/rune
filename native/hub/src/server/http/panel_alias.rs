use std::{path::Path, sync::Arc};

use anyhow::Context;
use axum::{
    Json,
    extract::{Extension, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::server::{ServerManager, ServerState, manager::update_alias};

use super::register::AppError;

#[derive(Deserialize)]
pub struct UpdateAliasRequest {
    alias: String,
}

pub async fn update_alias_handler(
    State(state): State<Arc<ServerState>>,
    Extension(server_manager): Extension<Arc<ServerManager>>,
    Json(payload): Json<UpdateAliasRequest>,
) -> Result<StatusCode, AppError> {
    let config_path = Path::new(&*server_manager.global_params.config_path);
    update_alias(config_path, &payload.alias)
        .await
        .context("Failed to update alias")
        .map_err(|e| AppError::Internal(e.to_string()))?;

    {
        let mut device_info = state.discovery_device_info.write().await;
        device_info.alias = payload.alias;
    }

    Ok(StatusCode::NO_CONTENT)
}
