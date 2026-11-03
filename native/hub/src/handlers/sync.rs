use crate::{
    Session, Signal,
    messages::{SetDownloadRuleRequest, SetDownloadRuleResponse},
    server::utils::device::load_device_info,
    utils::{GlobalParams, ParamsExtractor, sync_settings::SyncSettings},
};
use database::actions::downloads::set_download_rule;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use database::connection::MainDbConnection;

impl ParamsExtractor for SetDownloadRuleRequest {
    type Params = Arc<MainDbConnection>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.main_db.clone()
    }
}

impl Signal for SetDownloadRuleRequest {
    type Params = Arc<MainDbConnection>;
    type Response = SetDownloadRuleResponse;

    async fn handle(
        &self,
        db: Self::Params,
        _session: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let result = set_download_rule(
            &db,
            request
                .queries
                .iter()
                .map(|q| (q.operator.clone(), q.parameter.clone()))
                .collect(),
            request.mix_id,
            request.enabled,
        )
        .await;
        let (rule_id, error) = match result {
            Ok(id) => (id, None),
            Err(error) => (String::new(), Some(error.to_string())),
        };
        Ok(Some(SetDownloadRuleResponse {
            request_id: request.request_id.clone(),
            rule_id,
            error,
        }))
    }
}

impl ParamsExtractor for crate::messages::GetSyncSettingsRequest {
    type Params = Arc<tokio::sync::RwLock<SyncSettings>>;
    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.sync_settings.clone()
    }
}
impl Signal for crate::messages::GetSyncSettingsRequest {
    type Params = Arc<tokio::sync::RwLock<SyncSettings>>;
    type Response = crate::messages::GetSyncSettingsResponse;
    async fn handle(
        &self,
        settings: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let settings = settings.read().await;
        Ok(Some(Self::Response {
            request_id: request.request_id.clone(),
            listen: settings.listen,
            missing_host_file: settings.missing_host_file.into(),
            error: None,
        }))
    }
}
impl ParamsExtractor for crate::messages::SetSyncSettingsRequest {
    type Params = (
        Arc<tokio::sync::RwLock<SyncSettings>>,
        Arc<String>,
        Option<Arc<crate::server::ServerManager>>,
    );
    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        (
            params.sync_settings.clone(),
            params.config_path.clone(),
            params.server_manager.get().cloned(),
        )
    }
}
impl Signal for crate::messages::SetSyncSettingsRequest {
    type Params = (
        Arc<tokio::sync::RwLock<SyncSettings>>,
        Arc<String>,
        Option<Arc<crate::server::ServerManager>>,
    );
    type Response = crate::messages::SetSyncSettingsResponse;
    async fn handle(
        &self,
        (settings, config, manager): Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let result: Result<()> = async {
            let policy = request.missing_host_file.into();
            let mut current = settings.write().await;
            let changed = current.listen != request.listen;
            let next = SyncSettings {
                listen: request.listen,
                missing_host_file: policy,
            };
            next.save(Path::new(&**config))?;
            *current = next;
            if changed {
                let manager =
                    manager.ok_or_else(|| anyhow::anyhow!("server manager unavailable"))?;
                if request.listen {
                    if !manager.is_running() {
                        let info = load_device_info(Path::new(&**config)).await?;
                        manager
                            .start_sync(
                                ([0, 0, 0, 0], 7863).into(),
                                discovery::DiscoveryParams { device_info: info },
                            )
                            .await?;
                    }
                } else if manager.is_running() {
                    manager.stop().await?;
                }
            }
            Ok(())
        }
        .await;
        Ok(Some(Self::Response {
            request_id: request.request_id.clone(),
            error: result.err().map(|error| format!("{error:#}")),
        }))
    }
}
