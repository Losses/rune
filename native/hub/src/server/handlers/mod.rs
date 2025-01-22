use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{routing::get, Router};
use discovery::{http_api::create_discovery_router, pin::PinConfig, DiscoveryParams};
use log::info;
use tokio::sync::RwLock;

use crate::server::{
    handlers::{file_handler::file_handler, websocket_handler::websocket_handler},
    AppState, ServerState, WebSocketService,
};

pub mod file_handler;
pub mod websocket_handler;

pub async fn serve_combined(
    app_state: Arc<AppState>,
    websocket_service: Arc<WebSocketService>,
    addr: SocketAddr,
    discovery_params: DiscoveryParams,
) {
    let server_state = Arc::new(ServerState {
        app_state: app_state.clone(),
        websocket_service,
        discovery_device_info: discovery_params.device_info,
        discovery_active_sessions: Arc::new(RwLock::new(HashMap::new())),
        discovery_pin_config: Arc::new(RwLock::new(PinConfig::new(discovery_params.pin))),
        discovery_file_provider: discovery_params.file_provider,
    });

    let app = Router::new()
        .route("/ws", get(websocket_handler))
        .route("/files/{*file_path}", get(file_handler))
        .nest("/api/discovery", create_discovery_router())
        .with_state(server_state);

    let lib_path = &app_state.lib_path;
    let cover_temp_dir = &app_state.cover_temp_dir;

    info!(
        "Libaray files path: {}",
        lib_path.to_string_lossy().to_string()
    );
    info!(
        "Temporary files path: {}",
        cover_temp_dir.to_string_lossy().to_string()
    );

    info!("Starting combined HTTP/WebSocket server on {}", addr);
    axum_server::bind(addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
