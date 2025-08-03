use rinf::{DartSignal, RustSignal};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RegisterLicenseRequest {
    pub path: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RegisterLicenseResponse {
    pub valid: bool,
    pub license: Option<String>,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ValidateLicenseRequest {
    pub license: Option<String>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ValidateLicenseResponse {
    pub is_pro: bool,
    pub is_store_mode: bool,
}
