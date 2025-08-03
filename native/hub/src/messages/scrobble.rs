use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct LoginRequestItem {
    pub service_id: String,
    pub username: String,
    pub password: String,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct AuthenticateSingleServiceRequest {
    pub request: Option<LoginRequestItem>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct AuthenticateSingleServiceResponse {
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct AuthenticateMultipleServiceRequest {
    pub requests: Vec<LoginRequestItem>,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct ScrobbleServiceStatus {
    pub service_id: String,
    pub is_available: bool,
    pub error: Option<String>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ScrobbleServiceStatusUpdated {
    pub services: Vec<ScrobbleServiceStatus>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct LogoutSingleServiceRequest {
    pub service_id: String,
}
