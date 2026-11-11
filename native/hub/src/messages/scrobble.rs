use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub enum ScrobbleApiCredentials {
    None,
    LastFm { api_key: String, api_secret: String },
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub enum ScrobbleSecretInput {
    Password {
        password: String,
        api_credentials: ScrobbleApiCredentials,
        replaces: Option<String>,
    },
    Saved {
        credential_ref: String,
    },
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub enum LastFmApiCredentialField {
    ApiKey,
    ApiSecret,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub enum ScrobbleAuthenticationFailure {
    CredentialsRejected,
    ServiceUnreachable,
    LastFmApiCredentialsMissing { field: LastFmApiCredentialField },
    LastFmApiCredentialsInvalid,
    Other { detail: String },
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub enum AuthenticateSingleServiceResult {
    Success {
        credential_ref: String,
    },
    Failure {
        failure: ScrobbleAuthenticationFailure,
    },
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct LoginRequestItem {
    pub service_id: String,
    pub username: String,
    pub secret_input: ScrobbleSecretInput,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct AuthenticateSingleServiceRequest {
    pub request: Option<LoginRequestItem>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct AuthenticateSingleServiceResponse {
    pub result: AuthenticateSingleServiceResult,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct AuthenticateMultipleServiceRequest {
    pub requests: Vec<LoginRequestItem>,
    #[serde(default)]
    pub call_id: Option<u64>,
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
    pub credential_ref: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}
