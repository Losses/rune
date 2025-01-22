use std::{collections::HashMap, sync::Arc};

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
    file::{download, upload},
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
    async fn get_files(&self) -> anyhow::Result<HashMap<String, FileMetadata>>;
}

#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub files: HashMap<String, FileMetadata>,
    pub tokens: HashMap<String, String>,
}

pub fn create_discovery_router<S: DiscoveryState + PinValidationState>() -> Router<Arc<S>> {
    Router::new()
        .route("/api/rune/v2/register", post(register::<S>))
        .route("/api/rune/v2/prepare-upload", post(prepare_upload::<S>))
        .route("/api/rune/v2/upload", post(upload::<S>))
        .route("/api/rune/v2/cancel", post(cancel::<S>))
        .route("/api/rune/v2/prepare-download", post(prepare_download::<S>))
        .route("/api/rune/v2/download", get(download::<S>))
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

async fn prepare_upload<S: DiscoveryState>(
    State(state): State<Arc<S>>,
    Query(_): Query<HashMap<String, String>>,
    Json(request): Json<PrepareUploadRequest>,
) -> impl IntoResponse {
    let session_id = uuid::Uuid::new_v4().to_string();
    let mut tokens = HashMap::new();

    for (file_id, _) in request.files.iter() {
        tokens.insert(file_id.clone(), uuid::Uuid::new_v4().to_string());
    }

    state.active_sessions().write().await.insert(
        session_id.clone(),
        SessionInfo {
            files: request.files,
            tokens: tokens.clone(),
        },
    );

    Json(PrepareUploadResponse {
        session_id,
        files: tokens,
    })
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

#[derive(Debug, Serialize)]
struct PrepareDownloadResponse {
    info: DeviceInfo,
    session_id: String,
    files: HashMap<String, FileMetadata>,
}

async fn prepare_download<S: DiscoveryState>(
    State(state): State<Arc<S>>,
    Query(params): Query<HashMap<String, String>>,
    _pin_auth: PinAuth,
) -> Result<Json<PrepareDownloadResponse>, StatusCode> {
    if let Some(session_id) = params.get("sessionId") {
        let active_sessions = state.active_sessions();
        let sessions = active_sessions.read().await;
        if let Some(session) = sessions.get(session_id) {
            return Ok(Json(PrepareDownloadResponse {
                info: state.device_info().clone(),
                session_id: session_id.clone(),
                files: session.files.clone(),
            }));
        }
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    let files = state
        .file_provider()
        .get_files()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let active_sessions = state.active_sessions();
    let mut sessions = active_sessions.write().await;
    sessions.insert(
        session_id.clone(),
        SessionInfo {
            files: files.clone(),
            tokens: HashMap::new(),
        },
    );

    Ok(Json(PrepareDownloadResponse {
        info: state.device_info().clone(),
        session_id,
        files,
    }))
}

async fn info<S: DiscoveryState>(State(state): State<Arc<S>>) -> impl IntoResponse {
    Json(state.device_info().clone())
}
