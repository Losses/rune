use crate::node::NodeContext;
use crate::request_error::ExpectedRequestError;
use crate::server::api::remove_trusted_device;
use crate::server::{
    ServerManager,
    api::{add_trusted_server, check_fingerprint, edit_trusted_hosts, register_device},
    generate_or_load_certificates, get_or_generate_alias,
    sync_worker::{
        SyncTokenStore, obtain_or_reuse_token, peer_base, tls_config, trusted_fingerprint,
    },
    utils::device::load_device_info,
};
use std::net::{IpAddr, SocketAddr};
use std::{
    fs,
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use log::info;
use reqwest::redirect::Policy;
use rustls::pki_types::CertificateDer;
use tokio::{sync::RwLock, time::timeout};
use url::Url;
use uuid::Uuid;

use ::database::actions::cover_art::COVER_TEMP_DIR;
use ::discovery::{
    DiscoveryParams,
    client::{
        CertValidator, CertValidatorError, fetch_server_certificate, select_best_host, try_connect,
    },
    endpoint::{DEFAULT_DEVICE_PORT, DeviceAddress},
    protocol::DiscoveryService,
    server::{PermissionManager, UserStatus},
    utils::{DeviceInfo, DeviceType},
};

use crate::utils::{GlobalParams, ParamsExtractor};
use crate::{Session, Signal, messages::*};

pub(crate) async fn select_pinned_pair_host(
    hosts: Vec<String>,
    validator: &CertValidator,
    fingerprint: &str,
) -> Result<String> {
    let addresses = hosts
        .iter()
        .map(|host| {
            host.parse::<DeviceAddress>()
                .map_err(|_| ExpectedRequestError::invalid_input("Device address is malformed"))
                .map_err(anyhow::Error::from)
        })
        .collect::<Result<Vec<_>>>()?;
    let urls = addresses
        .iter()
        .map(|address| address.url("https", "/").map(|url| url.to_string()))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    match select_best_host(
        urls.clone(),
        Arc::new(tls_config(validator.clone(), fingerprint)),
    )
    .await
    {
        Ok(url) => {
            let parsed = Url::parse(&url).context("Invalid selected device URL")?;
            let host = parsed
                .host_str()
                .context("Selected device URL has no host")?;
            let port = parsed.port().unwrap_or(DEFAULT_DEVICE_PORT);
            Ok(DeviceAddress::new(host, port)?.to_string())
        }
        Err(_) => {
            for url in &urls {
                if let Ok(certificate) = fetch_server_certificate(url).await
                    && certificate.fingerprint != fingerprint
                {
                    return Err(ExpectedRequestError::CertificateMismatch.into());
                }
            }
            Err(ExpectedRequestError::Unavailable {
                what: "paired server",
            }
            .into())
        }
    }
}

impl ParamsExtractor for StartBroadcastRequest {
    type Params = (Arc<DiscoveryService>, Arc<String>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.device_scanner),
            Arc::clone(&all_params.config_path),
        )
    }
}

impl Signal for StartBroadcastRequest {
    type Params = (Arc<DiscoveryService>, Arc<String>);
    type Response = ();

    async fn handle(
        &self,
        (scanner, config_path): Self::Params,
        _session: Option<Session>,
        request: &Self,
    ) -> anyhow::Result<Option<Self::Response>> {
        let certificate_id = request.alias.clone();
        let (fingerprint, _, _) =
            generate_or_load_certificates(Path::new(&*config_path), &certificate_id).await?;

        info!(
            "Start broadcasting the device: {}({})",
            request.alias, fingerprint
        );

        scanner
            .start_announcements(
                DeviceInfo {
                    alias: request.alias.clone(),
                    device_model: Some("RuneAudio".to_string()),
                    version: "Technical Preview".to_owned(),
                    device_type: Some(DeviceType::Desktop),
                    fingerprint: fingerprint.clone(),
                    api_port: DEFAULT_DEVICE_PORT,
                    protocol: "http".to_owned(),
                    protocol_version: Some(discovery::protocol_version::PROTOCOL_VERSION),
                    capabilities: None,
                },
                Duration::from_secs(request.duration_seconds.into()),
                None,
            )
            .await?;
        Ok(None)
    }
}

impl ParamsExtractor for StopBroadcastRequest {
    type Params = (Arc<DiscoveryService>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.device_scanner),)
    }
}

impl Signal for StopBroadcastRequest {
    type Params = (Arc<DiscoveryService>,);
    type Response = ();

    async fn handle(
        &self,
        (scanner,): Self::Params,
        _session: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        scanner.stop_announcements().await;
        Ok(None)
    }
}

impl ParamsExtractor for StartListeningRequest {
    type Params = (Arc<DiscoveryService>, Arc<String>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.device_scanner),
            Arc::clone(&all_params.config_path),
        )
    }
}

impl Signal for StartListeningRequest {
    type Params = (Arc<DiscoveryService>, Arc<String>);
    type Response = ();

    async fn handle(
        &self,
        (scanner, config_path): Self::Params,
        _session: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        info!("Start listening for devices with alias: {}", request.alias);
        let certificate_id = request.alias.clone();
        let (fingerprint, _, _) =
            generate_or_load_certificates(Path::new(&*config_path), &certificate_id).await?;

        scanner.start_listening(Some(fingerprint)).await?;

        Ok(None)
    }
}

impl ParamsExtractor for StopListeningRequest {
    type Params = (Arc<DiscoveryService>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.device_scanner),)
    }
}

impl Signal for StopListeningRequest {
    type Params = (Arc<DiscoveryService>,);
    type Response = ();

    async fn handle(
        &self,
        (scanner,): Self::Params,
        _session: Option<Session>,
        _: &Self,
    ) -> anyhow::Result<Option<Self::Response>> {
        scanner.stop_listening().await;
        Ok(None)
    }
}

impl ParamsExtractor for GetDiscoveredDeviceRequest {
    type Params = Arc<DiscoveryService>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        Arc::clone(&all_params.device_scanner)
    }
}

impl Signal for GetDiscoveredDeviceRequest {
    type Params = Arc<DiscoveryService>;
    type Response = GetDiscoveredDeviceResponse;

    async fn handle(
        &self,
        scanner: Self::Params,
        _session: Option<Session>,
        _request: &Self,
    ) -> Result<Option<Self::Response>> {
        let devices = scanner.get_all_devices();

        let devices_message = devices
            .into_iter()
            .map(|x| {
                let addresses = announced_device_addresses(&x.ips, x.api_port)?;
                Ok(DiscoveredDeviceMessage {
                    alias: x.alias,
                    fingerprint: x.fingerprint,
                    device_model: x.device_model,
                    device_type: x.device_type.to_string(),
                    last_seen_unix_epoch: x.last_seen.timestamp(),
                    addresses,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Some(GetDiscoveredDeviceResponse {
            call_id: None,
            devices: devices_message,
        }))
    }
}

fn announced_device_addresses(ips: &[IpAddr], api_port: Option<u16>) -> Result<Vec<String>> {
    let port = api_port.unwrap_or(DEFAULT_DEVICE_PORT);
    ips.iter()
        .map(|ip| Ok(DeviceAddress::new(ip.to_string(), port)?.to_string()))
        .collect()
}

impl ParamsExtractor for StartServerRequest {
    type Params = (Arc<String>, Arc<ServerManager>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        let server_manager = all_params
            .server_manager
            .get()
            .expect("ServerManager must be initialized before use")
            .clone();

        (
            Arc::clone(&all_params.config_path),
            Arc::clone(&server_manager),
        )
    }
}

impl Signal for StartServerRequest {
    type Params = (Arc<String>, Arc<ServerManager>);
    type Response = StartServerResponse;

    #[allow(clippy::manual_async_fn)]
    fn handle(
        &self,
        (config_path, server_manager): Self::Params,
        _session: Option<Session>,
        request: &Self,
    ) -> impl Future<Output = Result<Option<Self::Response>>> + Send {
        async move {
            let ip: std::net::IpAddr = request.interface.parse().map_err(|e| {
                ExpectedRequestError::invalid_input(format!(
                    "Invalid interface address '{}': {}",
                    request.interface, e
                ))
            })?;
            let addr = SocketAddr::new(ip, DEFAULT_DEVICE_PORT);

            let device_info = DeviceInfo {
                alias: request.alias.clone(),
                device_model: Some("RuneAudio".to_string()),
                version: "Technical Preview".to_owned(),
                device_type: Some(DeviceType::Desktop),
                fingerprint: generate_or_load_certificates(
                    Path::new(&*config_path),
                    &request.alias.clone(),
                )
                .await?
                .0,
                api_port: DEFAULT_DEVICE_PORT,
                protocol: "http".to_owned(),
                protocol_version: Some(discovery::protocol_version::PROTOCOL_VERSION),
                capabilities: None,
            };

            let discovery_params = DiscoveryParams { device_info };

            match server_manager.start(addr, discovery_params).await {
                Ok(_) => Ok(Some(StartServerResponse {
                    call_id: None,
                    success: true,
                    error: String::new(),
                })),
                Err(e) => Ok(Some(StartServerResponse {
                    call_id: None,
                    success: false,
                    error: format!("{e:#?}"),
                })),
            }
        }
    }
}

impl ParamsExtractor for StopServerRequest {
    type Params = Arc<ServerManager>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        let server_manager = all_params
            .server_manager
            .get()
            .expect("ServerManager must be initialized before use")
            .clone();

        Arc::clone(&server_manager)
    }
}

impl Signal for StopServerRequest {
    type Params = Arc<ServerManager>;
    type Response = StopServerResponse;

    async fn handle(
        &self,
        server_manager: Self::Params,
        _session: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        let address = server_manager.get_address().await;
        let result = async {
            server_manager.stop().await?;
            if server_manager.node()?.sync_settings.read().await.listen
                && let Some(address) = address
            {
                let info =
                    load_device_info(Path::new(&*server_manager.node()?.config_path)).await?;
                server_manager
                    .clone()
                    .start_sync(address, DiscoveryParams { device_info: info })
                    .await?;
            }
            anyhow::Ok(())
        }
        .await;
        match result {
            Ok(_) => Ok(Some(StopServerResponse {
                call_id: None,
                success: true,
                error: "".into(),
            })),
            Err(e) => Ok(Some(StopServerResponse {
                call_id: None,
                success: false,
                error: format!("{e:#?}"),
            })),
        }
    }
}

impl ParamsExtractor for ListClientsRequest {
    type Params = Arc<RwLock<PermissionManager>>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        Arc::clone(&all_params.permission_manager)
    }
}

impl Signal for ListClientsRequest {
    type Params = Arc<RwLock<PermissionManager>>;
    type Response = ListClientsResponse;

    async fn handle(
        &self,
        permission_manager: Self::Params,
        _session: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        let users = permission_manager.read().await.list_users().await;

        let converted_users = users
            .into_iter()
            .map(|u| ClientSummary {
                alias: u.alias,
                fingerprint: u.fingerprint,
                device_model: u.device_model,
                status: match u.status {
                    UserStatus::Approved => ClientStatus::Approved,
                    UserStatus::Pending => ClientStatus::Pending,
                    UserStatus::Blocked => ClientStatus::Blocked,
                },
            })
            .collect();

        Ok(Some(ListClientsResponse {
            call_id: None,
            success: true,
            users: converted_users,
            error: String::new(),
        }))
    }
}

impl ParamsExtractor for GetSslCertificateFingerprintRequest {
    type Params = Arc<String>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        Arc::clone(&all_params.config_path)
    }
}

impl Signal for GetSslCertificateFingerprintRequest {
    type Params = Arc<String>;
    type Response = GetSslCertificateFingerprintResponse;

    async fn handle(
        &self,
        config_path: Self::Params,
        _session: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        let path = Path::new(&**config_path);
        let certificate_id = get_or_generate_alias(path).await?;

        let (fingerprint, _certificate, _private_key) =
            generate_or_load_certificates(path, &certificate_id)
                .await
                .context("Failed to initialize certificates")?;

        Ok(Some(GetSslCertificateFingerprintResponse {
            call_id: None,
            fingerprint,
        }))
    }
}

impl ParamsExtractor for RemoveTrustedClientRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for RemoveTrustedClientRequest {
    type Params = Arc<NodeContext>;
    type Response = RemoveTrustedClientResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        req: &Self,
    ) -> Result<Option<Self::Response>> {
        let permissions = node.permission_manager.read().await;
        let validator = node.cert_validator.read().await;
        let result = remove_trusted_device(&permissions, &validator, &req.fingerprint).await;
        drop(validator);
        drop(permissions);
        if result.is_ok() {
            node.sync_tokens.remove_peer(&req.fingerprint);
            node.revoke_outgoing_for(&req.fingerprint).await;
        }
        match result {
            Ok(_) => Ok(Some(RemoveTrustedClientResponse {
                call_id: None,
                success: true,
                error: String::new(),
            })),
            Err(e) => Ok(Some(RemoveTrustedClientResponse {
                call_id: None,
                success: false,
                error: format!("{e:#?}"),
            })),
        }
    }
}

impl ParamsExtractor for ServerAvailabilityTestRequest {
    type Params = Arc<RwLock<CertValidator>>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        Arc::clone(&all_params.cert_validator)
    }
}

impl Signal for ServerAvailabilityTestRequest {
    type Params = Arc<RwLock<CertValidator>>;
    type Response = ServerAvailabilityTestResponse;

    async fn handle(
        &self,
        validator: Self::Params,
        _session: Option<Session>,
        req: &Self,
    ) -> Result<Option<Self::Response>> {
        let validator = Arc::new(validator.read().await.clone());

        let hosts = &req.hosts;
        if hosts.is_empty()
            || hosts
                .iter()
                .any(|host| host.parse::<DeviceAddress>().is_err())
        {
            return Ok(Some(ServerAvailabilityTestResponse {
                call_id: None,
                outcome: ServerAvailabilityTestOutcome::Failed(
                    ServerAvailabilityTestFailure::AddressMalformed,
                ),
            }));
        }

        let client_config = Arc::new(validator.clone().into_client_config());
        let device_urls = match hosts
            .iter()
            .map(|host| {
                host.parse::<DeviceAddress>()
                    .and_then(|address| address.url("https", "/"))
                    .map(|url| url.to_string())
            })
            .collect::<std::result::Result<Vec<_>, _>>()
        {
            Ok(urls) => urls,
            Err(_) => {
                return Ok(Some(ServerAvailabilityTestResponse {
                    call_id: None,
                    outcome: ServerAvailabilityTestOutcome::Failed(
                        ServerAvailabilityTestFailure::AddressMalformed,
                    ),
                }));
            }
        };
        let outcome = match timeout(
            Duration::from_secs(5),
            select_best_host(device_urls, client_config),
        )
        .await
        {
            Err(_) => availability_outcome(Err(ServerAvailabilityTestFailure::TimedOut)),
            Ok(Ok(_)) => availability_outcome(Ok(())),
            Ok(Err(_)) => {
                let failure = if has_fingerprint_mismatch(hosts, &validator).await {
                    ServerAvailabilityTestFailure::TlsFingerprintMismatch
                } else {
                    ServerAvailabilityTestFailure::NoHostReachable
                };
                availability_outcome(Err(failure))
            }
        };

        Ok(Some(ServerAvailabilityTestResponse {
            call_id: None,
            outcome,
        }))
    }
}

fn availability_outcome(
    result: Result<(), ServerAvailabilityTestFailure>,
) -> ServerAvailabilityTestOutcome {
    match result {
        Ok(()) => ServerAvailabilityTestOutcome::Success,
        Err(failure) => ServerAvailabilityTestOutcome::Failed(failure),
    }
}

async fn has_fingerprint_mismatch(hosts: &[String], validator: &CertValidator) -> bool {
    for host in hosts {
        let Ok(address) = host.parse::<DeviceAddress>() else {
            continue;
        };
        let Ok(url) = address.url("https", "/ping") else {
            continue;
        };
        let Ok(Ok(server_certificate)) = timeout(
            Duration::from_secs(5),
            fetch_server_certificate(url.as_str()),
        )
        .await
        else {
            continue;
        };
        let Ok(certificate_pem) = pem::parse(server_certificate.cert) else {
            continue;
        };
        let certificate = CertificateDer::from(certificate_pem.into_contents());
        if matches!(
            validator.verify_certificate(&certificate, host).await,
            Err(CertValidatorError::FingerprintMismatch)
        ) {
            return true;
        }
    }
    false
}

impl ParamsExtractor for UpdateClientStatusRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for UpdateClientStatusRequest {
    type Params = Arc<NodeContext>;
    type Response = UpdateClientStatusResponse;

    async fn handle(
        &self,
        permission_manager: Self::Params,
        _session: Option<Session>,
        message: &Self,
    ) -> Result<Option<Self::Response>> {
        let result = async {
            let permissions = permission_manager.permission_manager.write().await;
            // Install a chosen preset before approval becomes visible to transports.
            if matches!(message.status, ClientStatus::Approved)
                && let Some(preset) = message.preset
            {
                permissions
                    .verify_by_fingerprint(&message.fingerprint)
                    .await
                    .context("Device not found")?;
                permission_manager
                    .access_policy
                    .set(&message.fingerprint, preset.grants())?;
            }
            permissions
                .change_user_status(
                    &message.fingerprint,
                    match message.status {
                        ClientStatus::Approved => UserStatus::Approved,
                        ClientStatus::Pending => UserStatus::Pending,
                        ClientStatus::Blocked => UserStatus::Blocked,
                    },
                )
                .await?;
            Ok::<(), anyhow::Error>(())
        }
        .await;
        match result {
            Ok(_) => Ok(Some(UpdateClientStatusResponse {
                call_id: None,
                success: true,
                error: String::new(),
            })),
            Err(e) => Ok(Some(UpdateClientStatusResponse {
                call_id: None,
                success: false,
                error: format!("{e:#?}"),
            })),
        }
    }
}

impl ParamsExtractor for EditHostsRequest {
    type Params = Arc<RwLock<CertValidator>>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        Arc::clone(&all_params.cert_validator)
    }
}

impl Signal for EditHostsRequest {
    type Params = Arc<RwLock<CertValidator>>;
    type Response = EditHostsResponse;

    async fn handle(
        &self,
        validator: Self::Params,
        _session: Option<Session>,
        req: &Self,
    ) -> Result<Option<Self::Response>> {
        edit_trusted_hosts(
            &validator.read().await.clone(),
            &req.fingerprint,
            req.hosts.clone(),
        )
        .await?;
        Ok(Some(EditHostsResponse {
            call_id: None,
            success: true,
            error: String::new(),
        }))
    }
}

impl ParamsExtractor for AddTrustedServerRequest {
    type Params = Arc<RwLock<CertValidator>>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        Arc::clone(&all_params.cert_validator)
    }
}

impl Signal for AddTrustedServerRequest {
    type Params = Arc<RwLock<CertValidator>>;
    type Response = AddTrustedServerResponse;

    async fn handle(
        &self,
        validator: Self::Params,
        _session: Option<Session>,
        req: &Self,
    ) -> Result<Option<Self::Response>> {
        add_trusted_server(&validator.read().await.clone(), req.certificate.as_ref()).await?;

        if let Some(certificate) = &req.certificate {
            info!(
                "Adding trusted host {:?} with fingerprint {:?}",
                certificate.hosts, certificate.fingerprint
            );
        }

        Ok(Some(AddTrustedServerResponse {
            call_id: None,
            success: true,
            error: String::new(),
        }))
    }
}

impl ParamsExtractor for RemoveTrustedServerRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.node.clone()
    }
}

impl Signal for RemoveTrustedServerRequest {
    type Params = Arc<NodeContext>;
    type Response = RemoveTrustedServerResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _session: Option<Session>,
        req: &Self,
    ) -> Result<Option<Self::Response>> {
        let permissions = node.permission_manager.read().await;
        let validator = node.cert_validator.read().await;
        let result = remove_trusted_device(&permissions, &validator, &req.fingerprint).await;
        drop(validator);
        drop(permissions);
        if result.is_ok() {
            node.sync_tokens.remove_peer(&req.fingerprint);
            node.revoke_outgoing_for(&req.fingerprint).await;
        }
        match result {
            Ok(_) => Ok(Some(RemoveTrustedServerResponse {
                call_id: None,
                success: true,
                error: String::new(),
            })),
            Err(e) => Ok(Some(RemoveTrustedServerResponse {
                call_id: None,
                success: false,
                error: format!("{e:#?}"),
            })),
        }
    }
}

impl ParamsExtractor for RegisterDeviceOnServerRequest {
    type Params = (Arc<String>, Arc<RwLock<CertValidator>>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.config_path),
            Arc::clone(&all_params.cert_validator),
        )
    }
}

impl Signal for RegisterDeviceOnServerRequest {
    type Params = (Arc<String>, Arc<RwLock<CertValidator>>);
    type Response = RegisterDeviceOnServerResponse;

    async fn handle(
        &self,
        config: Self::Params,
        _session: Option<Session>,
        req: &Self,
    ) -> Result<Option<Self::Response>> {
        let (config_path, validator) = config;
        let validator = validator.read().await.clone();
        let config_path = config_path.to_string();

        let certificate_id = req.alias.clone();
        let (fingerprint, cert, _) =
            generate_or_load_certificates(config_path, &certificate_id).await?;

        let host = select_pinned_pair_host(req.hosts.clone(), &validator, &req.fingerprint).await?;
        let client_config = Arc::new(tls_config(validator, &req.fingerprint));

        register_device(
            &host,
            client_config,
            cert,
            fingerprint,
            req.alias.clone(),
            "RuneAudio".to_string(),
            "Desktop".to_string(),
        )
        .await?;
        Ok(Some(RegisterDeviceOnServerResponse {
            call_id: None,
            success: true,
            error: String::new(),
        }))
    }
}

impl ParamsExtractor for CheckDeviceOnServerRequest {
    type Params = (Arc<RwLock<CertValidator>>, Arc<String>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.cert_validator),
            Arc::clone(&all_params.config_path),
        )
    }
}

impl Signal for CheckDeviceOnServerRequest {
    type Params = (Arc<RwLock<CertValidator>>, Arc<String>);
    type Response = CheckDeviceOnServerResponse;

    async fn handle(
        &self,
        (validator, config_path): Self::Params,
        _session: Option<Session>,
        req: &Self,
    ) -> Result<Option<Self::Response>> {
        let validator = validator.read().await.clone();

        let (fingerprint, _, _) =
            generate_or_load_certificates(Path::new(&*config_path), &req.alias).await?;

        let host = select_pinned_pair_host(req.hosts.clone(), &validator, &req.fingerprint).await?;
        let client_config = Arc::new(tls_config(validator, &req.fingerprint));

        match check_fingerprint(&host, client_config, &fingerprint).await {
            Ok(response) => {
                let status = match response.status.as_str() {
                    "APPROVED" => ClientStatus::Approved,
                    "PENDING" => ClientStatus::Pending,
                    "BLOCKED" => ClientStatus::Blocked,
                    _ => ClientStatus::Pending,
                };

                Ok(Some(CheckDeviceOnServerResponse {
                    call_id: None,
                    success: true,
                    error: String::new(),
                    status: Some(status),
                }))
            }
            Err(_) => Err(ExpectedRequestError::CertificateMismatch.into()),
        }
    }
}

impl ParamsExtractor for ConnectRequest {
    type Params = Arc<RwLock<CertValidator>>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        Arc::clone(&all_params.cert_validator)
    }
}

impl Signal for ConnectRequest {
    type Params = Arc<RwLock<CertValidator>>;
    type Response = ConnectResponse;

    async fn handle(
        &self,
        validator: Self::Params,
        _session: Option<Session>,
        req: &Self,
    ) -> Result<Option<Self::Response>> {
        let tasks = req.hosts.iter().map(|host| {
            let validator = Arc::clone(&validator);
            let host = host.clone();
            tokio::spawn(async move {
                try_connect(
                    &host,
                    Arc::new(validator.read().await.clone()).into_client_config(),
                )
                .await
            })
        });

        let mut last_err = None;
        for task in tasks {
            match task.await {
                Ok(Ok(host)) => {
                    return Ok(Some(ConnectResponse {
                        call_id: None,
                        success: true,
                        connected_host: host,
                        error: String::new(),
                    }));
                }
                Ok(Err(e)) => last_err = Some(e),
                Err(e) => last_err = Some(anyhow!(e)),
            }
        }

        Ok(Some(ConnectResponse {
            call_id: None,
            success: false,
            connected_host: String::new(),
            error: last_err.map(|e| e.to_string()).unwrap_or_default(),
        }))
    }
}

impl ParamsExtractor for FetchServerCertificateRequest {
    type Params = ();

    fn extract_params(&self, _all_params: &GlobalParams) -> Self::Params {}
}

impl Signal for FetchServerCertificateRequest {
    type Params = ();
    type Response = FetchServerCertificateResponse;

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        req: &Self,
    ) -> Result<Option<Self::Response>> {
        match fetch_server_certificate(&req.url).await {
            Ok(cert) => Ok(Some(FetchServerCertificateResponse {
                call_id: None,
                success: true,
                fingerprint: cert.fingerprint,
                error: String::new(),
            })),
            Err(e) => Ok(Some(FetchServerCertificateResponse {
                call_id: None,
                success: false,
                fingerprint: String::new(),
                error: format!("{e:#?}"),
            })),
        }
    }
}

pub struct FetchRemoteFileParams {
    validator: Arc<RwLock<CertValidator>>,
    config_path: Arc<String>,
    sync_tokens: SyncTokenStore,
}

impl ParamsExtractor for FetchRemoteFileRequest {
    type Params = FetchRemoteFileParams;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        FetchRemoteFileParams {
            validator: Arc::clone(&all_params.cert_validator),
            config_path: Arc::clone(&all_params.config_path),
            sync_tokens: all_params.node.sync_tokens.clone(),
        }
    }
}

/// Fetches `url` from a trusted peer with a proof-of-key bearer token.
pub(crate) async fn authorized_get(
    config_path: &str,
    validator: CertValidator,
    sync_tokens: &SyncTokenStore,
    host: &str,
    port: u16,
    url: Url,
) -> Result<reqwest::Response> {
    let server_fingerprint = trusted_fingerprint(&validator, host, port).await?;
    let config_path = Path::new(config_path);
    let alias = get_or_generate_alias(config_path).await?;
    let (fingerprint, _, private_key) = generate_or_load_certificates(config_path, &alias).await?;
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(tls_config(validator, &server_fingerprint))
        .redirect(Policy::none())
        .build()?;
    let base = peer_base(host, port)?;
    let token = obtain_or_reuse_token(
        sync_tokens,
        &client,
        &base,
        &fingerprint,
        &private_key,
        &server_fingerprint,
    )
    .await?;
    let response = client.get(url.clone()).bearer_auth(&token).send().await?;
    if response.status() != reqwest::StatusCode::UNAUTHORIZED {
        return Ok(response);
    }
    sync_tokens.invalidate(&server_fingerprint, &base);
    let refreshed = obtain_or_reuse_token(
        sync_tokens,
        &client,
        &base,
        &fingerprint,
        &private_key,
        &server_fingerprint,
    )
    .await?;
    Ok(client.get(url).bearer_auth(refreshed).send().await?)
}

fn remote_file_cache_path(
    cache_root: &Path,
    fingerprint: &str,
    library_id: Uuid,
    file_name: &str,
) -> PathBuf {
    cache_root
        .join("remote_files")
        .join(fingerprint)
        .join(library_id.to_string())
        .join(file_name)
}

fn publish_verified_remote_file(cache_dir: &Path, file_name: &str, body: &[u8]) -> Result<PathBuf> {
    let digest = blake3::hash(body).to_hex().to_string();
    if digest != file_name {
        return Err(anyhow!(
            "Downloaded file does not match its BLAKE3 digest name"
        ));
    }
    let local_path = cache_dir.join(file_name);
    let temporary_path = cache_dir.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    if let Err(error) =
        fs::write(&temporary_path, body).and_then(|()| fs::rename(&temporary_path, &local_path))
    {
        let _ = fs::remove_file(&temporary_path);
        return Err(error.into());
    }
    Ok(local_path)
}

impl Signal for FetchRemoteFileRequest {
    type Params = FetchRemoteFileParams;
    type Response = FetchRemoteFileResponse;

    async fn handle(
        &self,
        params: Self::Params,
        _session: Option<Session>,
        req: &Self,
    ) -> Result<Option<Self::Response>> {
        let validator = params.validator.read().await.clone();

        // Parse the URL to extract host, port, and path
        let url = match Url::parse(&req.url) {
            Ok(url) => url,
            Err(e) => {
                return Ok(Some(FetchRemoteFileResponse {
                    call_id: None,
                    success: false,
                    local_path: String::new(),
                    error: format!("Invalid URL: {e}"),
                }));
            }
        };

        // Extract hostname and port
        let host = match url.host_str() {
            Some(host) => host.to_string(),
            None => {
                return Ok(Some(FetchRemoteFileResponse {
                    call_id: None,
                    success: false,
                    local_path: String::new(),
                    error: "Missing host in URL".to_string(),
                }));
            }
        };

        let address = (url.scheme() == "https")
            .then(|| {
                url.host_str().and_then(|host| {
                    DeviceAddress::new(host, url.port().unwrap_or(DEFAULT_DEVICE_PORT)).ok()
                })
            })
            .flatten();
        let Some(address) = address else {
            return Ok(Some(FetchRemoteFileResponse {
                call_id: None,
                success: false,
                local_path: String::new(),
                error: "Remote file URL must be an HTTPS URL with a valid port".to_string(),
            }));
        };
        let port = address.port();

        let library_id = url
            .query_pairs()
            .find(|(name, _)| name == "library_id")
            .and_then(|(_, value)| Uuid::parse_str(&value).ok());
        let Some(library_id) = library_id else {
            return Ok(Some(FetchRemoteFileResponse {
                call_id: None,
                success: false,
                local_path: String::new(),
                error: "Remote file URL must include a valid library_id".to_string(),
            }));
        };

        // Extract the file name from the URL path
        let path = url.path();
        let file_name = Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();

        if file_name.is_empty() {
            return Ok(Some(FetchRemoteFileResponse {
                call_id: None,
                success: false,
                local_path: String::new(),
                error: "Invalid file path in URL".to_string(),
            }));
        }

        if file_name.len() != 64 || !file_name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Ok(Some(FetchRemoteFileResponse {
                call_id: None,
                success: false,
                local_path: String::new(),
                error: "Remote cover file name must be a BLAKE3 digest".to_string(),
            }));
        }

        // Authenticate the peer before a cached response can be returned.
        let server_fingerprint = match trusted_fingerprint(&validator, &host, port).await {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                return Ok(Some(FetchRemoteFileResponse {
                    call_id: None,
                    success: false,
                    local_path: String::new(),
                    error: format!("Failed to authorize remote host: {error:#}"),
                }));
            }
        };

        let local_path =
            remote_file_cache_path(&COVER_TEMP_DIR, &server_fingerprint, library_id, &file_name);
        let cache_dir = local_path
            .parent()
            .context("remote file cache path has no parent")?;
        if let Err(e) = fs::create_dir_all(cache_dir) {
            return Ok(Some(FetchRemoteFileResponse {
                call_id: None,
                success: false,
                local_path: String::new(),
                error: format!("Failed to create temp directory: {e}"),
            }));
        }

        // The host is a trusted peer (checked above) and the name is content-addressed
        // within that peer and library, so a cached copy needs no new download.
        if local_path.exists() {
            return Ok(Some(FetchRemoteFileResponse {
                call_id: None,
                success: true,
                local_path: local_path.to_string_lossy().into_owned(),
                error: String::new(),
            }));
        }

        let res = match authorized_get(
            &params.config_path,
            validator,
            &params.sync_tokens,
            &host,
            port,
            url,
        )
        .await
        {
            Ok(res) => res,
            Err(e) => {
                return Ok(Some(FetchRemoteFileResponse {
                    call_id: None,
                    success: false,
                    local_path: String::new(),
                    error: format!("Failed to fetch remote file: {e:#}"),
                }));
            }
        };

        if !res.status().is_success() {
            return Ok(Some(FetchRemoteFileResponse {
                call_id: None,
                success: false,
                local_path: String::new(),
                error: format!("HTTP request failed with status: {}", res.status()),
            }));
        }

        // Read the response body
        let body = match res.bytes().await {
            Ok(body) => body,
            Err(e) => {
                return Ok(Some(FetchRemoteFileResponse {
                    call_id: None,
                    success: false,
                    local_path: String::new(),
                    error: format!("Failed to read response body: {e}"),
                }));
            }
        };

        let published_path = match publish_verified_remote_file(cache_dir, &file_name, &body) {
            Ok(path) => path,
            Err(error) => {
                return Ok(Some(FetchRemoteFileResponse {
                    call_id: None,
                    success: false,
                    local_path: String::new(),
                    error: format!("Failed to verify or publish remote file: {error:#}"),
                }));
            }
        };

        // Return the successful response with the local file path
        Ok(Some(FetchRemoteFileResponse {
            call_id: None,
            success: true,
            local_path: published_path.to_string_lossy().into_owned(),
            error: String::new(),
        }))
    }
}

#[cfg(test)]
mod discovered_device_address_tests {
    use super::announced_device_addresses;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn announced_api_port_is_kept_in_discovered_addresses() {
        let ips = [IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))];
        assert_eq!(
            announced_device_addresses(&ips, Some(9999)).unwrap(),
            ["192.0.2.1:9999"],
        );
    }
}

#[cfg(test)]
mod remote_file_cache_tests {
    use super::{publish_verified_remote_file, remote_file_cache_path};
    use anyhow::{Context, Result};
    use std::fs;
    use tempfile::TempDir;
    use uuid::Uuid;

    #[test]
    fn remote_file_cache_is_scoped_to_peer_and_library() -> Result<()> {
        let temp = TempDir::new()?;
        let library_id = Uuid::new_v4();
        let file_name = blake3::hash(b"shared basename").to_hex().to_string();
        let peer_x = remote_file_cache_path(temp.path(), "peer-x", library_id, &file_name);
        let peer_y = remote_file_cache_path(temp.path(), "peer-y", library_id, &file_name);

        fs::create_dir_all(peer_x.parent().context("peer X cache directory")?)?;
        fs::write(&peer_x, b"peer X bytes")?;

        assert_ne!(peer_x, peer_y);
        assert!(peer_x.exists());
        assert!(!peer_y.exists());
        assert!(peer_x.ends_with(format!("peer-x/{library_id}/{file_name}")));
        assert!(peer_y.ends_with(format!("peer-y/{library_id}/{file_name}")));
        Ok(())
    }

    #[test]
    fn mismatched_digest_is_not_published() -> Result<()> {
        let temp = TempDir::new()?;
        let file_name = blake3::hash(b"expected").to_hex().to_string();
        let result = publish_verified_remote_file(temp.path(), &file_name, b"different");
        assert!(result.is_err());
        assert!(!temp.path().join(file_name).exists());
        Ok(())
    }
}

#[cfg(test)]
mod server_availability_test_tests {
    use super::{Signal, availability_outcome};
    use crate::{
        Session,
        messages::{
            ServerAvailabilityTestFailure, ServerAvailabilityTestOutcome,
            ServerAvailabilityTestRequest,
        },
    };
    use discovery::client::CertValidator;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    async fn request_outcome(hosts: Vec<String>) -> ServerAvailabilityTestOutcome {
        let directory = tempfile::tempdir().expect("temporary certificate directory");
        let validator = Arc::new(RwLock::new(
            CertValidator::new(directory.path())
                .await
                .expect("certificate validator"),
        ));
        let request = ServerAvailabilityTestRequest {
            hosts,
            call_id: None,
        };
        request
            .handle(validator, None::<Session>, &request)
            .await
            .expect("availability request")
            .expect("availability response")
            .outcome
    }

    #[tokio::test]
    async fn empty_host_list_is_typed_malformed_address() {
        assert_eq!(
            request_outcome(Vec::new()).await,
            ServerAvailabilityTestOutcome::Failed(ServerAvailabilityTestFailure::AddressMalformed),
        );
    }

    #[tokio::test]
    async fn malformed_host_is_typed_malformed_address() {
        assert_eq!(
            request_outcome(vec!["bad host:7863".to_owned()]).await,
            ServerAvailabilityTestOutcome::Failed(ServerAvailabilityTestFailure::AddressMalformed),
        );
    }

    #[test]
    fn malformed_address_is_reported_as_typed_failure() {
        assert_eq!(
            availability_outcome(Err(ServerAvailabilityTestFailure::AddressMalformed)),
            ServerAvailabilityTestOutcome::Failed(ServerAvailabilityTestFailure::AddressMalformed,),
        );
    }

    #[test]
    fn unreachable_hosts_are_reported_as_typed_failure() {
        assert_eq!(
            availability_outcome(Err(ServerAvailabilityTestFailure::NoHostReachable)),
            ServerAvailabilityTestOutcome::Failed(ServerAvailabilityTestFailure::NoHostReachable,),
        );
    }

    #[test]
    fn tls_fingerprint_mismatch_is_reported_as_typed_failure() {
        assert_eq!(
            availability_outcome(Err(ServerAvailabilityTestFailure::TlsFingerprintMismatch,)),
            ServerAvailabilityTestOutcome::Failed(
                ServerAvailabilityTestFailure::TlsFingerprintMismatch,
            ),
        );
    }

    #[test]
    fn timed_out_probe_is_reported_as_typed_failure() {
        assert_eq!(
            availability_outcome(Err(ServerAvailabilityTestFailure::TimedOut)),
            ServerAvailabilityTestOutcome::Failed(ServerAvailabilityTestFailure::TimedOut),
        );
    }

    #[test]
    fn successful_probe_has_success_outcome() {
        assert_eq!(
            availability_outcome(Ok(())),
            ServerAvailabilityTestOutcome::Success,
        );
    }
}
