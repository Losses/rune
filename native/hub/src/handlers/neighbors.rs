use std::future::Future;
use std::sync::Arc;

use anyhow::Result;

use discovery::utils::{DeviceInfo, DeviceType};
use discovery::DiscoveryParams;

use crate::server::ServerManager;
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

    async fn handle(
        &self,
        (scanner,): Self::Params,
        _: &Self,
    ) -> anyhow::Result<Option<Self::Response>> {
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
    ) -> anyhow::Result<Option<Self::Response>> {
        scanner
            .start_listening(&DeviceInfo {
                alias: request.alias.clone(),
                device_model: Some("RuneAudio".to_string()),
                version: "Technical Preview".to_owned(),
                device_type: Some(DeviceType::Desktop),
                fingerprint: request.fingerprint.clone(),
                api_port: 7863,
                protocol: "http".to_owned(),
            })
            .await;
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

impl Signal for StartServerRequest {
    type Params = Arc<ServerManager>;
    type Response = StartServerResponse;

    #[allow(clippy::manual_async_fn)]
    fn handle(
        &self,
        server_manager: Self::Params,
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
                fingerprint: request.fingerprint.clone(),
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
    ) -> anyhow::Result<Option<Self::Response>> {
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
