use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};

use discovery::permission::{PermissionManager, UserStatus};
use discovery::utils::{DeviceInfo, DeviceType};
use discovery::verifier::{fetch_server_certificate, try_connect, CertValidator};
use discovery::DiscoveryParams;
use tokio::sync::RwLock;

use crate::server::{generate_or_load_certificates, get_or_generate_certificate_id, ServerManager};
use crate::utils::device_scanner::DeviceScanner;
use crate::utils::{GlobalParams, ParamsExtractor};
use crate::{messages::*, Signal};

impl ParamsExtractor for StartBroadcastRequest {
    type Params = (Arc<DeviceScanner>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.device_scanner),)
    }
}

impl Signal for StartBroadcastRequest {
    type Params = (Arc<DeviceScanner>,);
    type Response = ();

    async fn handle(
        &self,
        (scanner,): Self::Params,
        request: &Self,
    ) -> anyhow::Result<Option<Self::Response>> {
        scanner
            .start_broadcast(
                &DeviceInfo {
                    alias: request.alias.clone(),
                    device_model: Some("RuneAudio".to_string()),
                    version: "Technical Preview".to_owned(),
                    device_type: Some(DeviceType::Desktop),
                    fingerprint: request.fingerprint.clone(),
                    api_port: 7863,
                    protocol: "http".to_owned(),
                },
                request.duration_seconds,
            )
            .await;
        Ok(None)
    }
}

impl ParamsExtractor for StopBroadcastRequest {
    type Params = (Arc<DeviceScanner>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.device_scanner),)
    }
}

impl Signal for StopBroadcastRequest {
    type Params = (Arc<DeviceScanner>,);
    type Response = ();

    async fn handle(&self, (scanner,): Self::Params, _: &Self) -> Result<Option<Self::Response>> {
        scanner.stop_broadcast().await;
        Ok(None)
    }
}

impl ParamsExtractor for StartListeningRequest {
    type Params = (Arc<DeviceScanner>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.device_scanner),)
    }
}

impl Signal for StartListeningRequest {
    type Params = (Arc<DeviceScanner>,);
    type Response = ();

    async fn handle(
        &self,
        (scanner,): Self::Params,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        scanner.start_listening(Some(request.fingerprint.clone())).await;
        Ok(None)
    }
}

impl ParamsExtractor for StopListeningRequest {
    type Params = (Arc<DeviceScanner>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.device_scanner),)
    }
}

impl Signal for StopListeningRequest {
    type Params = (Arc<DeviceScanner>,);
    type Response = ();

    async fn handle(
        &self,
        (scanner,): Self::Params,
        _: &Self,
    ) -> anyhow::Result<Option<Self::Response>> {
        scanner.stop_listening().await;
        Ok(None)
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
                    error: "".into(),
                })),
                Err(e) => Ok(Some(StartServerResponse {
                    success: false,
                    error: e.to_string(),
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
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        match server_manager.stop().await {
            Ok(_) => Ok(Some(StopServerResponse {
                success: true,
                error: "".into(),
            })),
            Err(e) => Ok(Some(StopServerResponse {
                success: false,
                error: e.to_string(),
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
                    UserStatus::Approved => ClientStatus::Approved.into(),
                    UserStatus::Pending => ClientStatus::Pending.into(),
                    UserStatus::Blocked => ClientStatus::Blocked.into(),
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

    async fn handle(&self, config_path: Self::Params, _: &Self) -> Result<Option<Self::Response>> {
        let path = Path::new(&**config_path);
        let certificate_id = get_or_generate_certificate_id(path).await?;

        let (fingerprint, _certificate, _private_key) =
            generate_or_load_certificates(path, &certificate_id)
                .await
                .context("Failed to initialize certificates")?;

        Ok(Some(GetSslCertificateFingerprintResponse { fingerprint }))
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
        message: &Self,
    ) -> Result<Option<Self::Response>> {
        match permission_manager
            .write()
            .await
            .change_user_status(
                &message.fingerprint,
                match message.status {
                    0 => UserStatus::Approved,
                    1 => UserStatus::Pending,
                    2 => UserStatus::Blocked,
                    _ => UserStatus::Pending,
                },
            )
            .await
        {
            Ok(_) => Ok(Some(UpdateClientStatusResponse {
                success: true,
                error: "".to_owned(),
            })),
            Err(e) => Ok(Some(UpdateClientStatusResponse {
                success: false,
                error: e.to_string(),
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

    async fn handle(&self, validator: Self::Params, req: &Self) -> Result<Option<Self::Response>> {
        let result = validator
            .write()
            .await
            .replace_hosts_for_fingerprint(&req.fingerprint, req.hosts.clone());
        match result {
            Ok(_) => Ok(Some(EditHostsResponse {
                success: true,
                error: String::new(),
            })),
            Err(e) => Ok(Some(EditHostsResponse {
                success: false,
                error: e.to_string(),
            })),
        }
    }
}

impl ParamsExtractor for RemoveTrustRequest {
    type Params = Arc<RwLock<CertValidator>>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        Arc::clone(&all_params.cert_validator)
    }
}

impl Signal for RemoveTrustRequest {
    type Params = Arc<RwLock<CertValidator>>;
    type Response = RemoveTrustResponse;

    async fn handle(&self, validator: Self::Params, req: &Self) -> Result<Option<Self::Response>> {
        let result = validator.write().await.remove_fingerprint(&req.fingerprint);
        match result {
            Ok(_) => Ok(Some(RemoveTrustResponse {
                success: true,
                error: String::new(),
            })),
            Err(e) => Ok(Some(RemoveTrustResponse {
                success: false,
                error: e.to_string(),
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

    async fn handle(&self, validator: Self::Params, req: &Self) -> Result<Option<Self::Response>> {
        let tasks = req.hosts.iter().map(|host| {
            let validator = Arc::clone(&validator);
            let host = host.clone();
            tokio::spawn(async move {
                try_connect(&host, validator.read().await.clone().into_client_config()).await
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
                    }))
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

    async fn handle(&self, _: Self::Params, req: &Self) -> Result<Option<Self::Response>> {
        match fetch_server_certificate(&req.url).await {
            Ok(cert) => Ok(Some(FetchServerCertificateResponse {
                success: true,
                fingerprint: cert.fingerprint,
                error: String::new(),
            })),
            Err(e) => Ok(Some(FetchServerCertificateResponse {
                success: false,
                fingerprint: String::new(),
                error: e.to_string(),
            })),
        }
    }
}
