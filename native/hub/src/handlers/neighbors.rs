use std::{fs, future::Future, path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow};
use log::info;
use tokio::sync::RwLock;
use url::Url;

use ::database::actions::cover_art::COVER_TEMP_DIR;
use ::discovery::{
    DiscoveryParams,
    client::{CertValidator, fetch_server_certificate, select_best_host, try_connect},
    protocol::DiscoveryService,
    server::{PermissionManager, UserStatus},
    url::decode_rnsrv_url,
    utils::{DeviceInfo, DeviceType},
};
use ::http_request::{BodyExt, Bytes, Empty, Request, Uri, create_https_client, send_http_request};

use crate::server::{
    ServerManager,
    api::{check_fingerprint, register_device},
    generate_or_load_certificates, get_or_generate_alias,
};
use crate::utils::{GlobalParams, ParamsExtractor};
use crate::{Session, Signal, messages::*};

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
                    api_port: 7863,
                    protocol: "http".to_owned(),
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
            .map(|x| DiscoveredDeviceMessage {
                alias: x.alias,
                fingerprint: x.fingerprint,
                device_model: x.device_model,
                device_type: x.device_type.to_string(),
                last_seen_unix_epoch: x.last_seen.timestamp(),
                ips: x.ips.into_iter().map(|ip| ip.to_string()).collect(),
            })
            .collect();

        Ok(Some(GetDiscoveredDeviceResponse {
            devices: devices_message,
        }))
    }
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
                anyhow::anyhow!("Invalid interface address '{}': {}", request.interface, e)
            })?;
            let addr = std::net::SocketAddr::new(ip, 7863);

            let device_info = DeviceInfo {
                alias: request.alias.clone(),
                device_model: Some("RuneAudio".to_string()),
                version: "Technical Preview".to_owned(),
                device_type: Some(discovery::utils::DeviceType::Desktop),
                fingerprint: generate_or_load_certificates(
                    Path::new(&*config_path),
                    &request.alias.clone(),
                )
                .await?
                .0,
                api_port: 7863,
                protocol: "http".to_owned(),
            };

            let discovery_params = DiscoveryParams { device_info };

            match server_manager.start(addr, discovery_params).await {
                Ok(_) => Ok(Some(StartServerResponse {
                    success: true,
                    error: String::new(),
                })),
                Err(e) => Ok(Some(StartServerResponse {
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
        match server_manager.stop().await {
            Ok(_) => Ok(Some(StopServerResponse {
                success: true,
                error: "".into(),
            })),
            Err(e) => Ok(Some(StopServerResponse {
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

        Ok(Some(GetSslCertificateFingerprintResponse { fingerprint }))
    }
}

impl ParamsExtractor for RemoveTrustedClientRequest {
    type Params = Arc<RwLock<PermissionManager>>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        Arc::clone(&all_params.permission_manager)
    }
}

impl Signal for RemoveTrustedClientRequest {
    type Params = Arc<RwLock<PermissionManager>>;
    type Response = RemoveTrustedClientResponse;

    async fn handle(
        &self,
        validator: Self::Params,
        _session: Option<Session>,
        req: &Self,
    ) -> Result<Option<Self::Response>> {
        let result = validator.write().await.remove_user(&req.fingerprint).await;
        match result {
            Ok(_) => Ok(Some(RemoveTrustedClientResponse {
                success: true,
                error: String::new(),
            })),
            Err(e) => Ok(Some(RemoveTrustedClientResponse {
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

        let hosts = match decode_rnsrv_url(&req.url) {
            Ok(x) => x,
            Err(e) => {
                return Ok(Some(ServerAvailabilityTestResponse {
                    success: false,
                    error: format!("{e:#?}"),
                }));
            }
        };

        let client_config = Arc::new(validator.clone().into_client_config());
        let host = select_best_host(hosts, client_config).await;

        Ok(Some(match host {
            Ok(_) => ServerAvailabilityTestResponse {
                success: true,
                error: String::new(),
            },
            Err(e) => ServerAvailabilityTestResponse {
                success: false,
                error: format!("{e:#?}"),
            },
        }))
    }
}

impl ParamsExtractor for UpdateClientStatusRequest {
    type Params = Arc<RwLock<PermissionManager>>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        Arc::clone(&all_params.permission_manager)
    }
}

impl Signal for UpdateClientStatusRequest {
    type Params = Arc<RwLock<PermissionManager>>;
    type Response = UpdateClientStatusResponse;

    async fn handle(
        &self,
        permission_manager: Self::Params,
        _session: Option<Session>,
        message: &Self,
    ) -> Result<Option<Self::Response>> {
        match permission_manager
            .write()
            .await
            .change_user_status(
                &message.fingerprint,
                match message.status {
                    ClientStatus::Approved => UserStatus::Approved,
                    ClientStatus::Pending => UserStatus::Pending,
                    ClientStatus::Blocked => UserStatus::Blocked,
                },
            )
            .await
        {
            Ok(_) => Ok(Some(UpdateClientStatusResponse {
                success: true,
                error: String::new(),
            })),
            Err(e) => Ok(Some(UpdateClientStatusResponse {
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
        let result = validator
            .write()
            .await
            .replace_hosts_for_fingerprint(&req.fingerprint, req.hosts.clone())
            .await;
        match result {
            Ok(_) => Ok(Some(EditHostsResponse {
                success: true,
                error: String::new(),
            })),
            Err(e) => Ok(Some(EditHostsResponse {
                success: false,
                error: format!("{e:#?}"),
            })),
        }
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
        let certificate = match &req.certificate {
            Some(x) => x,
            None => {
                return Ok(Some(AddTrustedServerResponse {
                    success: true,
                    error: String::new(),
                }));
            }
        };

        info!(
            "Adding trusted host {:?} with fingerprint {:?}",
            certificate.hosts, certificate.fingerprint
        );

        let result = validator
            .write()
            .await
            .add_trusted_domains(&certificate.hosts, certificate.fingerprint.clone())
            .await;

        match result {
            Ok(_) => Ok(Some(AddTrustedServerResponse {
                success: true,
                error: String::new(),
            })),
            Err(e) => Ok(Some(AddTrustedServerResponse {
                success: false,
                error: format!("{e:#?}"),
            })),
        }
    }
}

impl ParamsExtractor for RemoveTrustedServerRequest {
    type Params = Arc<RwLock<CertValidator>>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        Arc::clone(&all_params.cert_validator)
    }
}

impl Signal for RemoveTrustedServerRequest {
    type Params = Arc<RwLock<CertValidator>>;
    type Response = RemoveTrustedServerResponse;

    async fn handle(
        &self,
        validator: Self::Params,
        _session: Option<Session>,
        req: &Self,
    ) -> Result<Option<Self::Response>> {
        let result = validator
            .write()
            .await
            .remove_fingerprint(&req.fingerprint)
            .await;
        match result {
            Ok(_) => Ok(Some(RemoveTrustedServerResponse {
                success: true,
                error: String::new(),
            })),
            Err(e) => Ok(Some(RemoveTrustedServerResponse {
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
        let validator = Arc::new(validator.read().await.clone());
        let client_config = Arc::new(validator.clone().into_client_config());
        let config_path = config_path.to_string();

        let certificate_id = req.alias.clone();
        let (fingerprint, cert, _) =
            generate_or_load_certificates(config_path, &certificate_id).await?;

        let host = match select_best_host(req.hosts.clone(), client_config.clone()).await {
            Ok(x) => x,
            Err(e) => {
                return Ok(Some(RegisterDeviceOnServerResponse {
                    success: false,
                    error: format!("{e:#?}"),
                }));
            }
        };

        match register_device(
            &host,
            client_config.clone(),
            cert,
            fingerprint,
            req.alias.clone(),
            "RuneAudio".to_string(),
            "Desktop".to_string(),
        )
        .await
        {
            Ok(_) => Ok(Some(RegisterDeviceOnServerResponse {
                success: true,
                error: String::new(),
            })),
            Err(e) => Ok(Some(RegisterDeviceOnServerResponse {
                success: false,
                error: format!("{e:#?}"),
            })),
        }
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
        let client_config = Arc::new(Arc::new(validator).into_client_config());

        let (fingerprint, _, _) =
            generate_or_load_certificates(Path::new(&*config_path), &req.alias).await?;

        let host = match select_best_host(req.hosts.clone(), client_config.clone()).await {
            Ok(host) => host,
            Err(e) => {
                return Ok(Some(CheckDeviceOnServerResponse {
                    success: false,
                    error: format!("{e:#?}"),
                    status: None,
                }));
            }
        };

        match check_fingerprint(&host, client_config, &fingerprint).await {
            Ok(response) => {
                let status = match response.status.as_str() {
                    "APPROVED" => ClientStatus::Approved,
                    "PENDING" => ClientStatus::Pending,
                    "BLOCKED" => ClientStatus::Blocked,
                    _ => ClientStatus::Pending,
                };

                Ok(Some(CheckDeviceOnServerResponse {
                    success: true,
                    error: String::new(),
                    status: Some(status),
                }))
            }
            Err(e) => Ok(Some(CheckDeviceOnServerResponse {
                success: false,
                error: format!("{e:#}"),
                status: Some(ClientStatus::Blocked),
            })),
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
                success: true,
                fingerprint: cert.fingerprint,
                error: String::new(),
            })),
            Err(e) => Ok(Some(FetchServerCertificateResponse {
                success: false,
                fingerprint: String::new(),
                error: format!("{e:#?}"),
            })),
        }
    }
}

impl ParamsExtractor for FetchRemoteFileRequest {
    type Params = Arc<RwLock<CertValidator>>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        Arc::clone(&all_params.cert_validator)
    }
}

impl Signal for FetchRemoteFileRequest {
    type Params = Arc<RwLock<CertValidator>>;
    type Response = FetchRemoteFileResponse;

    async fn handle(
        &self,
        validator: Self::Params,
        _session: Option<Session>,
        req: &Self,
    ) -> Result<Option<Self::Response>> {
        let validator = Arc::new(validator.read().await.clone());

        // Parse the URL to extract host, port, and path
        let url = match Url::parse(&req.url) {
            Ok(url) => url,
            Err(e) => {
                return Ok(Some(FetchRemoteFileResponse {
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
                    success: false,
                    local_path: String::new(),
                    error: "Missing host in URL".to_string(),
                }));
            }
        };

        let port = url.port().unwrap_or(7863);

        // Extract the file name from the URL path
        let path = url.path();
        let file_name = Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();

        if file_name.is_empty() {
            return Ok(Some(FetchRemoteFileResponse {
                success: false,
                local_path: String::new(),
                error: "Invalid file path in URL".to_string(),
            }));
        }

        // Ensure the cover arts directory exists
        if let Err(e) = fs::create_dir_all(COVER_TEMP_DIR.clone()) {
            return Ok(Some(FetchRemoteFileResponse {
                success: false,
                local_path: String::new(),
                error: format!("Failed to create temp directory: {e}"),
            }));
        }

        // Create the local file path
        let local_path = COVER_TEMP_DIR.clone().join(file_name);

        // Check if the file already exists locally
        if local_path.exists() {
            return Ok(Some(FetchRemoteFileResponse {
                success: true,
                local_path: local_path.to_str().unwrap_or_default().to_string(),
                error: String::new(),
            }));
        }

        // Create an HTTPS client with the custom TLS configuration
        let mut sender = match create_https_client(
            host.clone(),
            port,
            validator.clone().into_client_config().into(),
        )
        .await
        {
            Ok(sender) => sender,
            Err(e) => {
                return Ok(Some(FetchRemoteFileResponse {
                    success: false,
                    local_path: String::new(),
                    error: format!("Failed to create HTTPS client: {e}"),
                }));
            }
        };

        // Build the request URI
        let uri = match Uri::builder()
            .scheme("https")
            .authority(format!("{host}:{port}"))
            .path_and_query(path)
            .build()
        {
            Ok(uri) => uri,
            Err(e) => {
                return Ok(Some(FetchRemoteFileResponse {
                    success: false,
                    local_path: String::new(),
                    error: format!("Failed to build request URI: {e}"),
                }));
            }
        };

        // Build the HTTP request
        let req = match Request::builder().uri(uri).body(Empty::<Bytes>::new()) {
            Ok(req) => req,
            Err(e) => {
                return Ok(Some(FetchRemoteFileResponse {
                    success: false,
                    local_path: String::new(),
                    error: format!("Failed to build HTTP request: {e}"),
                }));
            }
        };

        // Send the request
        let res = match send_http_request(&mut sender, req).await {
            Ok(res) => res,
            Err(e) => {
                return Ok(Some(FetchRemoteFileResponse {
                    success: false,
                    local_path: String::new(),
                    error: format!("Failed to send HTTP request: {e}"),
                }));
            }
        };

        // Check the response status
        if !res.status().is_success() {
            return Ok(Some(FetchRemoteFileResponse {
                success: false,
                local_path: String::new(),
                error: format!("HTTP request failed with status: {}", res.status()),
            }));
        }

        // Read the response body
        let body = match res.into_body().collect().await {
            Ok(body) => body.to_bytes(),
            Err(e) => {
                return Ok(Some(FetchRemoteFileResponse {
                    success: false,
                    local_path: String::new(),
                    error: format!("Failed to read response body: {e}"),
                }));
            }
        };

        // Write the file to disk
        if let Err(e) = fs::write(&local_path, &body) {
            return Ok(Some(FetchRemoteFileResponse {
                success: false,
                local_path: String::new(),
                error: format!("Failed to write file to disk: {e}"),
            }));
        }

        // Return the successful response with the local file path
        Ok(Some(FetchRemoteFileResponse {
            success: true,
            local_path: local_path.to_str().unwrap_or_default().to_string(),
            error: String::new(),
        }))
    }
}
