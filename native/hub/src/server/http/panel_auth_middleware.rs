use std::sync::Arc;

use axum::{body::Body, http::Request, middleware::Next, response::Response};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};

use crate::server::{http::register::AppError, manager::JwtClaims, ServerManager};

pub async fn auth_middleware(request: Request<Body>, next: Next) -> Result<Response, AppError> {
    let auth_header = request
        .headers()
        .get("Authorization")
        .and_then(|h| h.to_str().ok());

    let token = if let Some(header) = auth_header {
        if header.starts_with("Bearer ") {
            header.trim_start_matches("Bearer ")
        } else {
            return Err(AppError::Unauthorized("Invalid token format".into()));
        }
    } else {
        return Err(AppError::Unauthorized(
            "Missing authorization header".into(),
        ));
    };

    let server_manager = request
        .extensions()
        .get::<Arc<ServerManager>>()
        .ok_or(AppError::Internal("Server manager not found".into()))?;

    let validation = Validation::new(Algorithm::HS256);
    decode::<JwtClaims>(
        token,
        &DecodingKey::from_secret(&server_manager.jwt_secret),
        &validation,
    )
    .map_err(|_| AppError::Unauthorized("Invalid token".into()))?;

    Ok(next.run(request).await)
}
