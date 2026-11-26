use std::sync::Arc;

use axum::{body::Body, http::Request, middleware::Next, response::Response};

use crate::server::{ServerManager, http::register::AppError};

pub async fn auth_middleware(request: Request<Body>, next: Next) -> Result<Response, AppError> {
    let token = request
        .headers()
        .get("Authorization")
        .ok_or_else(|| AppError::Unauthorized("Missing authorization header".into()))?
        .to_str()
        .ok()
        .and_then(|header| header.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::Unauthorized("Invalid token format".into()))?;

    let server_manager = request
        .extensions()
        .get::<Arc<ServerManager>>()
        .ok_or(AppError::Internal("Server manager not found".into()))?;

    if !server_manager.verify_jwt_token(token) {
        return Err(AppError::Unauthorized("Invalid token".into()));
    }

    Ok(next.run(request).await)
}
