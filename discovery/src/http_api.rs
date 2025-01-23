use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use axum::{
    extract::{Query, State},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

use crate::{
    pin::{PinAuth, PinConfig, PinValidationState},
    utils::{DeviceInfo, FileMetadata},
};

#[async_trait]
pub trait DiscoveryState: Clone + Send + Sync + 'static {
    type FileProvider: FileProvider + ?Sized;

    fn device_info(&self) -> &DeviceInfo;
    fn active_sessions(&self) -> Arc<RwLock<HashMap<String, SessionInfo>>>;
    fn pin_config(&self) -> Arc<RwLock<PinConfig>>;
    fn file_provider(&self) -> Arc<Self::FileProvider>;
}

#[async_trait]
pub trait FileProvider: Send + Sync {
    async fn get_files(&self) -> Result<HashMap<String, FileMetadata>>;
}

#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub files: HashMap<String, FileMetadata>,
    pub tokens: HashMap<String, String>,
}

pub fn create_discovery_router<S: DiscoveryState + PinValidationState>() -> Router<Arc<S>> {
    Router::new()
        .route("/api/rune/v2/register", post(register::<S>))
        .route("/api/rune/v2/cancel", post(cancel::<S>))
        .route("/api/rune/v2/info", get(info::<S>))
        .layer(CorsLayer::permissive())
}

async fn register<S: DiscoveryState>(
    State(state): State<Arc<S>>,
    Json(_): Json<DeviceInfo>,
) -> impl IntoResponse {
    Json(state.device_info().clone())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareUploadRequest {
    pub info: DeviceInfo,
    pub files: HashMap<String, FileMetadata>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareUploadResponse {
    pub session_id: String,
    pub files: HashMap<String, String>, // file_id -> token
}

async fn cancel<S: DiscoveryState>(
    State(state): State<Arc<S>>,
    Query(params): Query<HashMap<String, String>>,
    _pin_auth: PinAuth,
) -> impl IntoResponse {
    let session_id = params.get("sessionId").ok_or(StatusCode::BAD_REQUEST)?;

    let active_sessions = state.active_sessions();
    let mut sessions = active_sessions.write().await;

    if sessions.remove(session_id).is_some() {
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn info<S: DiscoveryState>(State(state): State<Arc<S>>) -> impl IntoResponse {
    Json(state.device_info().clone())
}
