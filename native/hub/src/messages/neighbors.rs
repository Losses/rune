use discovery::utils::DeviceType;
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SignalPiece)]
pub enum DeviceTypeMessage {
    Mobile,
    Desktop,
    Web,
    Headless,
    Server,
    Unknown,
}

impl From<DeviceType> for DeviceTypeMessage {
    fn from(device_type: DeviceType) -> Self {
        match device_type {
            DeviceType::Mobile => Self::Mobile,
            DeviceType::Desktop => Self::Desktop,
            DeviceType::Web => Self::Web,
            DeviceType::Headless => Self::Headless,
            DeviceType::Server => Self::Server,
            DeviceType::Unknown => Self::Unknown,
        }
    }
}

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
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct StopBroadcastRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct StartListeningRequest {
    pub alias: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct StopListeningRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, SignalPiece, RustSignal)]
pub struct DiscoveredDeviceMessage {
    pub alias: String,
    pub device_model: String,
    pub device_type: DeviceTypeMessage,
    pub fingerprint: String,
    pub last_seen_unix_epoch: i64,
    pub addresses: Vec<String>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetDiscoveredDeviceRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetDiscoveredDeviceResponse {
    pub devices: Vec<DiscoveredDeviceMessage>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct StartServerRequest {
    pub interface: String,
    pub alias: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct StartServerResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct StopServerRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct StopServerResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ListClientsRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ListClientsResponse {
    pub users: Vec<ClientSummary>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct TrustedClientsChanged {}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct IncommingClientPermissionNotification {
    pub user: ClientSummary,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct GetSslCertificateFingerprintRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct GetSslCertificateFingerprintResponse {
    pub fingerprint: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RemoveTrustedClientRequest {
    pub fingerprint: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoveTrustedClientResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct UpdateClientStatusRequest {
    pub fingerprint: String,
    pub status: ClientStatus,
    pub preset: Option<super::DevicePreset>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct UpdateClientStatusResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct EditHostsRequest {
    pub fingerprint: String,
    pub hosts: Vec<String>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct EditHostsResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct AddTrustedServerRequest {
    pub certificate: Option<TrustedServerCertificate>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct AddTrustedServerResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RemoveTrustedServerRequest {
    pub fingerprint: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RemoveTrustedServerResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ServerAvailabilityTestRequest {
    pub hosts: Vec<String>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerAvailabilityTestFailure {
    AddressMalformed,
    NoHostReachable,
    TlsFingerprintMismatch,
    TimedOut,
}

#[derive(Serialize, Deserialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerAvailabilityTestOutcome {
    Success,
    Failed(ServerAvailabilityTestFailure),
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ServerAvailabilityTestResponse {
    pub outcome: ServerAvailabilityTestOutcome,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct RegisterDeviceOnServerRequest {
    pub alias: String,
    pub hosts: Vec<String>,
    pub fingerprint: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct RegisterDeviceOnServerResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct CheckDeviceOnServerRequest {
    pub alias: String,
    pub hosts: Vec<String>,
    pub fingerprint: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct CheckDeviceOnServerResponse {
    pub status: ClientStatus,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct TrustListUpdated {
    pub certificates: Vec<TrustedServerCertificate>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct ConnectRequest {
    pub hosts: Vec<String>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct ConnectResponse {
    pub connected_host: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchServerCertificateRequest {
    pub url: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchServerCertificateResponse {
    pub fingerprint: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchRemoteFileRequest {
    pub url: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchRemoteFileResponse {
    pub local_path: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}
