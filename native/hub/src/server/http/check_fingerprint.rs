use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use log::debug;
use serde::{Deserialize, Serialize};

use ::discovery::server::UserStatus;

use crate::server::ServerState;

#[derive(Debug, Deserialize)]
pub struct CheckFingerprintQuery {
    fingerprint: String,
}

#[derive(Debug, Serialize)]
pub struct CheckFingerprintResponse {
    is_trusted: bool,
    status: String,
    message: String,
}

/// API to check if a fingerprint is trusted
/// To test this API, use:
/// curl -v "http://localhost:7863/check-fingerprint?fingerprint=01:23:45:67:89:AB:CD:EF"
pub async fn check_fingerprint_handler(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<CheckFingerprintQuery>,
) -> impl IntoResponse {
    let permission_manager = state.permission_manager.read().await;
    let user = permission_manager
        .verify_by_fingerprint(&query.fingerprint)
        .await;

    match user {
        Some(user) => {
            let (is_trusted, status, message) = match user.status {
                UserStatus::Pending => (
                    false,
                    "PENDING".to_string(),
                    "Device is pending approval".to_string(),
                ),
                UserStatus::Blocked => (
                    false,
                    "BLOCKED".to_string(),
                    "Device is blocked".to_string(),
                ),
                UserStatus::Approved => (
                    true,
                    "APPROVED".to_string(),
                    "Device is trusted".to_string(),
                ),
            };

            debug!("Checking fingerprint: {}({})", &query.fingerprint, status);

            let response = CheckFingerprintResponse {
                is_trusted,
                status,
                message,
            };

            (StatusCode::OK, Json(response))
        }
        None => {
            let response = CheckFingerprintResponse {
                is_trusted: false,
                status: "UNKNOWN".to_string(),
                message: "Device not found".to_string(),
            };

            (StatusCode::OK, Json(response))
        }
    }
}
