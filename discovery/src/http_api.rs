use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use axum::{extract::State, response::IntoResponse, routing::get, Json, Router};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

use crate::utils::{DeviceInfo, FileMetadata};

#[async_trait]
pub trait DiscoveryState: Clone + Send + Sync + 'static {
    fn device_info(&self) -> &DeviceInfo;
    fn active_sessions(&self) -> Arc<RwLock<HashMap<String, SessionInfo>>>;
}

#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub files: HashMap<String, FileMetadata>,
    pub tokens: HashMap<String, String>,
}

pub fn create_discovery_router<S: DiscoveryState>() -> Router<Arc<S>> {
    Router::new()
        .route("/api/rune/v2/info", get(info::<S>))
        .layer(CorsLayer::permissive())
}

async fn info<S: DiscoveryState>(State(state): State<Arc<S>>) -> impl IntoResponse {
    Json(state.device_info().clone())
}
