use std::{net::SocketAddr, sync::Arc};

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{Response, StatusCode},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use discovery::{
    client::parse_certificate,
    server::{PermissionError, UserStatus},
    utils::DeviceType,
};

use crate::server::ServerState;

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    public_key: String,
    fingerprint: String,
    alias: String,
    device_model: String,
    device_type: DeviceType,
}

/// To test this API, use:
/// curl -v http://localhost:7863/register \
///  -H "Content-Type: application/json" \
///  -d '{
///    "public_key": "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA",
///    "fingerprint": "01:23:45:67:89:AB:CD:EF",
///    "alias": "Test Device",
///    "device_model": "NixOS Device",
///    "device_type": "desktop"
///  }'
pub async fn register_handler(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<ServerState>>,
    Json(request): Json<RegisterRequest>,
) -> Result<impl IntoResponse, AppError> {
    verify_registration_key(&request.public_key, &request.fingerprint)?;
    let ip = addr.ip().to_string();
    {
        let permission_manager = state.permission_manager.read().await;
        let user = permission_manager
            .verify_by_fingerprint(&request.fingerprint)
            .await;

        if let Some(user) = user
            && user.status == UserStatus::Blocked
        {
            return Ok(StatusCode::FORBIDDEN);
        }
    }

    {
        state
            .permission_manager
            .write()
            .await
            .add_user(
                request.public_key,
                request.fingerprint,
                request.alias,
                request.device_model,
                request.device_type,
                ip,
            )
            .await?;
    }

    Ok(StatusCode::CREATED)
}

fn verify_registration_key(public_key: &str, fingerprint: &str) -> Result<(), AppError> {
    let (_, derived_fingerprint) = parse_certificate(public_key)
        .map_err(|error| AppError::ParseCertificate(error.to_string()))?;
    if derived_fingerprint != fingerprint {
        return Err(AppError::FingerprintMismatch);
    }
    Ok(())
}

#[derive(Debug)]
pub enum AppError {
    Permission(PermissionError),
    Internal(String),
    NotFound(String),
    Unauthorized(String),
    ParseCertificate(String),
    FingerprintMismatch,
}

impl From<PermissionError> for AppError {
    fn from(e: PermissionError) -> Self {
        AppError::Permission(e)
    }
}

#[derive(Serialize)]
struct ErrorResponse {
    code: &'static str,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response<axum::body::Body> {
        let (status, code, message) = match self {
            AppError::Permission(PermissionError::FingerprintConflict) => (
                StatusCode::CONFLICT,
                "fingerprint_conflict",
                "Fingerprint is already registered with a different key".to_owned(),
            ),
            AppError::Permission(e) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
            }
            AppError::Internal(e) => (StatusCode::INTERNAL_SERVER_ERROR, "internal", e),
            AppError::NotFound(e) => (StatusCode::NOT_FOUND, "not_found", e),
            AppError::Unauthorized(e) => (StatusCode::UNAUTHORIZED, "unauthorized", e),
            AppError::ParseCertificate(e) => (StatusCode::BAD_REQUEST, "invalid_public_key", e),
            AppError::FingerprintMismatch => (
                StatusCode::BAD_REQUEST,
                "public_key_mismatch",
                "Public key does not match fingerprint".to_owned(),
            ),
        };

        let body = Json(ErrorResponse { code, message });

        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::{AppError, IntoResponse, verify_registration_key};
    use axum::http::StatusCode;
    use discovery::server::PermissionError;
    use discovery::ssl::generate_self_signed_cert;

    #[test]
    fn registration_rejects_a_key_with_another_fingerprint() {
        let cert = generate_self_signed_cert("device", "rune", "CA", 1).unwrap();
        assert!(verify_registration_key(&cert.certificate, &cert.public_key_fingerprint).is_ok());
        assert!(matches!(
            verify_registration_key(&cert.certificate, "unrelated-fingerprint"),
            Err(AppError::FingerprintMismatch)
        ));
    }

    #[test]
    fn registration_key_mismatch_and_conflict_have_distinct_statuses() {
        assert_eq!(
            AppError::FingerprintMismatch.into_response().status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::Permission(PermissionError::FingerprintConflict)
                .into_response()
                .status(),
            StatusCode::CONFLICT
        );
    }
}
