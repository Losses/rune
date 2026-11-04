use crate::node::NodeContext;
use crate::request_error::ExpectedRequestError;
use crate::{
    Session, Signal,
    library_pool::LibraryHandle,
    messages::{
        LibraryContentChanged, RemoveDownloadRuleRequest, RemoveDownloadRuleResponse,
        SetDownloadRuleRequest, SetDownloadRuleResponse,
    },
    server::utils::device::load_device_info,
    utils::{
        Broadcaster, GlobalParams, ParamsExtractor, require_library, sync_settings::SyncSettings,
    },
};
use database::actions::downloads::{
    cleanup_unselected_downloads, remove_download_rule, set_download_rule,
};
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

#[derive(Clone)]
pub struct DownloadParams {
    library: Option<LibraryHandle>,
    broadcaster: Arc<dyn Broadcaster>,
    node: Arc<NodeContext>,
}
impl DownloadParams {
    fn from_global(p: &GlobalParams) -> Self {
        Self {
            library: p.library.clone(),
            broadcaster: p.broadcaster.clone(),
            node: p.node.clone(),
        }
    }
    async fn cleanup(&self, library: &LibraryHandle) -> Result<()> {
        let queue = self
            .node
            .session_queue_for_library(library.descriptor.library_id)
            .await?;
        let result = cleanup_unselected_downloads(
            &library.main_db,
            &library.recommend_db,
            &library.fsio,
            Path::new(&library.media_root),
            &library.library_sync.record_store(),
            &queue,
        )
        .await;
        self.broadcaster.broadcast(&LibraryContentChanged {
            path: library.media_root.clone(),
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
        let library = require_library(p.library.clone())?;
        let result = async {
            let id = set_download_rule(
                &library.main_db,
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
                p.cleanup(&library).await?;
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
        let library = require_library(p.library.clone())?;
        let result = async {
            remove_download_rule(&library.main_db, &request.rule_id).await?;
            if request.delete_unselected_files {
                p.cleanup(&library).await?;
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
