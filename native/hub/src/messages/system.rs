use rinf::{DartSignal, RustSignal};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, DartSignal)]
pub struct SystemInfoRequest {}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct SystemInfoResponse {
    pub build_date: String,
    pub build_sha: String,
    pub build_commit_timestamp: String,
    pub build_rustc_semver: String,
    pub system_name: String,
    pub system_kernel_version: String,
    pub system_os_version: String,
    pub system_host_name: String,
    pub users: Vec<String>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CrashResponse {
    pub detail: String,
}
