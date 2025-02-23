use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct PingResponse {
    message: &'static str,
}

pub async fn ping_handler() -> Json<PingResponse> {
    Json(PingResponse { message: "pong" })
}
