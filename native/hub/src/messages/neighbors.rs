use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub enum ClientStatus {
    Approved,
    Pending,
    Blocked,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct ClientSummary {
    pub alias: String,
    pub fingerprint: String,
    pub device_model: String,
    pub status: ClientStatus,
}

#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct TrustedServerCertificate {
    pub fingerprint: String,
    pub hosts: Vec<String>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct StartBroadcastRequest {
    pub duration_seconds: u32,
    pub alias: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct StopBroadcastRequest {}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct StartListeningRequest {
    pub alias: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct StopListeningRequest {}

#[derive(Deserialize, Serialize, SignalPiece, RustSignal)]
pub struct DiscoveredDeviceMessage {
    pub alias: String,
    pub device_model: String,
    pub device_type: String,
    pub fingerprint: String,
    pub last_seen_unix_epoch: i64,
    pub ips: Vec<String>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetDiscoveredDeviceRequest {}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetDiscoveredDeviceResponse {
    pub devices: Vec<DiscoveredDeviceMessage>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct StartServerRequest {
    pub interface: String,
    pub alias: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct StartServerResponse {
    pub success: bool,
    pub error: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct StopServerRequest {}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct StopServerResponse {
    pub success: bool,
    pub error: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ListClientsRequest {}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ListClientsResponse {
    pub success: bool,
    pub users: Vec<ClientSummary>,
    pub error: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct IncommingClientPermissionNotification {
    pub user: ClientSummary,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetSslCertificateFingerprintRequest {}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetSslCertificateFingerprintResponse {
    pub fingerprint: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RemoveTrustedClientRequest {
    pub fingerprint: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoveTrustedClientResponse {
    pub success: bool,
    pub error: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct UpdateClientStatusRequest {
    pub fingerprint: String,
    pub status: ClientStatus,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct UpdateClientStatusResponse {
    pub success: bool,
    pub error: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct EditHostsRequest {
    pub fingerprint: String,
    pub hosts: Vec<String>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct EditHostsResponse {
    pub success: bool,
    pub error: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct AddTrustedServerRequest {
    pub certificate: Option<TrustedServerCertificate>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct AddTrustedServerResponse {
    pub success: bool,
    pub error: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RemoveTrustedServerRequest {
    pub fingerprint: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoveTrustedServerResponse {
    pub success: bool,
    pub error: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ServerAvailabilityTestRequest {
    pub url: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ServerAvailabilityTestResponse {
    pub success: bool,
    pub error: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RegisterDeviceOnServerRequest {
    pub alias: String,
    pub hosts: Vec<String>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RegisterDeviceOnServerResponse {
    pub success: bool,
    pub error: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct CheckDeviceOnServerRequest {
    pub alias: String,
    pub hosts: Vec<String>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CheckDeviceOnServerResponse {
    pub success: bool,
    pub error: String,
    pub status: Option<ClientStatus>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct TrustListUpdated {
    pub certificates: Vec<TrustedServerCertificate>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ConnectRequest {
    pub hosts: Vec<String>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ConnectResponse {
    pub success: bool,
    pub error: String,
    pub connected_host: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchServerCertificateRequest {
    pub url: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchServerCertificateResponse {
    pub success: bool,
    pub fingerprint: String,
    pub error: String,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchRemoteFileRequest {
    pub url: String,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchRemoteFileResponse {
    pub success: bool,
    pub local_path: String,
    pub error: String,
}
