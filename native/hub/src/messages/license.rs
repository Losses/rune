use rinf::{DartSignal, RustSignal};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RegisterLicenseRequest {
    pub path: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RegisterLicenseResponse {
    pub license: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ValidateLicenseRequest {
    pub license: Option<String>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ValidateLicenseResponse {
    pub is_pro: bool,
    pub is_store_mode: bool,
    #[serde(default)]
    pub call_id: Option<u64>,
}
