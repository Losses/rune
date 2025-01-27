use std::sync::Arc;

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
        scanner.start_broadcast(request.duration_seconds).await;
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
        _: &Self,
    ) -> anyhow::Result<Option<Self::Response>> {
        scanner.start_listening().await;
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
