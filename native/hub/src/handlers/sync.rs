use crate::request_error::ExpectedRequestError;
use crate::{
    Session, Signal,
    messages::{
        LibraryContentChanged, RemoveDownloadRuleRequest, RemoveDownloadRuleResponse,
        SetDownloadRuleRequest, SetDownloadRuleResponse,
    },
    server::utils::device::load_device_info,
    utils::{Broadcaster, GlobalParams, ParamsExtractor, sync_settings::SyncSettings},
};
use database::actions::downloads::{
    cleanup_unselected_downloads, remove_download_rule, set_download_rule,
};
use database::connection::RecommendationDbConnection;
use database::sync::SeaOrmSyncStore;
use fsio::FsIo;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use database::connection::MainDbConnection;

#[derive(Clone)]
pub struct DownloadParams {
    db: Arc<MainDbConnection>,
    recommend: Arc<RecommendationDbConnection>,
    fsio: Arc<FsIo>,
    root: Arc<String>,
    store: Option<SeaOrmSyncStore>,
    broadcaster: Arc<dyn Broadcaster>,
}
impl DownloadParams {
    fn from_global(p: &GlobalParams) -> Self {
        Self {
            db: p.main_db.clone(),
            recommend: p.recommend_db.clone(),
            fsio: p.fsio.clone(),
            root: p.lib_path.clone(),
            store: p.library_sync.as_ref().map(|s| s.record_store()),
            broadcaster: p.broadcaster.clone(),
        }
    }
    async fn cleanup(&self) -> Result<()> {
        let store = self
            .store
            .as_ref()
            .ok_or(ExpectedRequestError::Unavailable {
                what: "library sync store",
            })?;
        let result = cleanup_unselected_downloads(
            &self.db,
            &self.recommend,
            &self.fsio,
            Path::new(&**self.root),
            store,
        )
        .await;
        self.broadcaster.broadcast(&LibraryContentChanged {
            path: (*self.root).clone(),
        });
        result.map(|_| ())
    }
}
impl ParamsExtractor for SetDownloadRuleRequest {
    type Params = DownloadParams;
    fn extract_params(&self, p: &GlobalParams) -> Self::Params {
        DownloadParams::from_global(p)
    }
}
impl Signal for SetDownloadRuleRequest {
    type Params = DownloadParams;
    type Response = SetDownloadRuleResponse;
    async fn handle(
        &self,
        p: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let result = async {
            let id = set_download_rule(
                &p.db,
                request
                    .queries
                    .iter()
                    .map(|q| (q.operator.clone(), q.parameter.clone()))
                    .collect(),
                request.mix_id,
                request.enabled,
            )
            .await?;
            if !request.enabled && request.delete_unselected_files {
                p.cleanup().await?;
            }
            Ok::<_, anyhow::Error>(id)
        }
        .await;
        let (rule_id, error) = match result {
            Ok(id) => (id, None),
            Err(error) => (String::new(), Some(format!("{error:#}"))),
        };
        Ok(Some(Self::Response {
            request_id: request.request_id.clone(),
            rule_id,
            error,
        }))
    }
}
impl ParamsExtractor for RemoveDownloadRuleRequest {
    type Params = DownloadParams;
    fn extract_params(&self, p: &GlobalParams) -> Self::Params {
        DownloadParams::from_global(p)
    }
}
impl Signal for RemoveDownloadRuleRequest {
    type Params = DownloadParams;
    type Response = RemoveDownloadRuleResponse;
    async fn handle(
        &self,
        p: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let result = async {
            remove_download_rule(&p.db, &request.rule_id).await?;
            if request.delete_unselected_files {
                p.cleanup().await?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        Ok(Some(Self::Response {
            request_id: request.request_id.clone(),
            error: result.err().map(|e| format!("{e:#}")),
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
                let manager = manager.ok_or(ExpectedRequestError::Unavailable {
                    what: "server manager",
                })?;
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
