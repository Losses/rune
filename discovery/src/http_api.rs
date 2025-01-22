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
    pin::{PinAuth, PinConfig},
    utils::{DeviceInfo, FileMetadata},
};

#[async_trait]
pub trait FileProvider: Send + Sync + 'static {
    async fn get_files(&self) -> anyhow::Result<HashMap<String, FileMetadata>>;
}

#[derive(Clone)]
pub struct AppState {
    pub device_info: DeviceInfo,
    pub active_sessions: Arc<RwLock<HashMap<String, SessionInfo>>>,
    pub pin_config: Arc<RwLock<PinConfig>>,
    pub file_provider: Arc<dyn FileProvider>,
}

#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub files: HashMap<String, FileMetadata>,
    pub tokens: HashMap<String, String>,
}

pub async fn serve(
    device_info: DeviceInfo,
    pin: Option<String>,
    file_provider: impl FileProvider,
) -> anyhow::Result<()> {
    let state = AppState {
        device_info,
        active_sessions: Arc::new(RwLock::new(HashMap::new())),
        pin_config: Arc::new(RwLock::new(PinConfig::new(pin))),
        file_provider: Arc::new(file_provider),
    };

    let app = Router::new()
        .route("/api/rune/v2/register", post(register))
        .route("/api/rune/v2/prepare-upload", post(prepare_upload))
        .route("/api/rune/v2/upload", post(upload))
        .route("/api/rune/v2/cancel", post(cancel))
        .route("/api/rune/v2/prepare-download", post(prepare_download))
        .route("/api/rune/v2/download", get(download))
        .route("/api/rune/v2/info", get(info))
        .layer(CorsLayer::permissive())
        .with_state(state);

    axum_server::bind("0.0.0.0:53317".parse()?)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

async fn register(State(state): State<AppState>, Json(_): Json<DeviceInfo>) -> impl IntoResponse {
    Json(state.device_info)
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

async fn prepare_upload(
    State(state): State<AppState>,
    Query(_): Query<HashMap<String, String>>,
    Json(request): Json<PrepareUploadRequest>,
) -> impl IntoResponse {
    // TODO: Implement PIN verification
    let session_id = uuid::Uuid::new_v4().to_string();
    let mut tokens = HashMap::new();

    for (file_id, _) in request.files.iter() {
        tokens.insert(file_id.clone(), uuid::Uuid::new_v4().to_string());
    }

    state.active_sessions.write().await.insert(
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

async fn cancel(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    _pin_auth: PinAuth,
) -> impl IntoResponse {
    let session_id = params.get("sessionId").ok_or(StatusCode::BAD_REQUEST)?;

    let mut sessions = state.active_sessions.write().await;

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

async fn prepare_download(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    _pin_auth: PinAuth,
) -> Result<Json<PrepareDownloadResponse>, StatusCode> {
    if let Some(session_id) = params.get("sessionId") {
        let sessions = state.active_sessions.read().await;
        if let Some(session) = sessions.get(session_id) {
            return Ok(Json(PrepareDownloadResponse {
                info: state.device_info.clone(),
                session_id: session_id.clone(),
                files: session.files.clone(),
            }));
        }
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    let files = state
        .file_provider
        .get_files()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut sessions = state.active_sessions.write().await;
    sessions.insert(
        session_id.clone(),
        SessionInfo {
            files: files.clone(),
            tokens: HashMap::new(),
        },
    );

    Ok(Json(PrepareDownloadResponse {
        info: state.device_info.clone(),
        session_id,
        files,
    }))
}

async fn info(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.device_info.clone())
}
