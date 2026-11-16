use crate::node::NodeContext;
use crate::request_error::ExpectedRequestError;
use crate::{
    Session, Signal,
    library_pool::LibraryHandle,
    messages::{
        LibraryContentChanged, RemoveDownloadRuleRequest, RemoveDownloadRuleResponse,
        SetDownloadRuleRequest, SetDownloadRuleResponse,
    },
    server::utils::device::{RunMode, load_device_info},
    utils::{
        Broadcaster, GlobalParams, ParamsExtractor, require_library, sync_settings::SyncSettings,
    },
};
use database::actions::downloads::{
    cleanup_unselected_downloads, remove_download_rule, set_download_rule,
};
use discovery::endpoint::DEFAULT_DEVICE_PORT;
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
            .session_queue_for_library(library.library_id)
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
            path: library.display_name.clone(),
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
        let rule_id = set_download_rule(
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
        Ok(Some(Self::Response {
            call_id: None,
            request_id: request.request_id.clone(),
            rule_id,
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
        remove_download_rule(&library.main_db, &request.rule_id).await?;
        if request.delete_unselected_files {
            p.cleanup(&library).await?;
        }
        Ok(Some(Self::Response {
            call_id: None,
            request_id: request.request_id.clone(),
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
            call_id: None,
            request_id: request.request_id.clone(),
            listen: settings.listen,
            missing_host_file: settings.missing_host_file.into(),
        }))
    }
}
impl ParamsExtractor for crate::messages::SetSyncSettingsRequest {
    type Params = (
        Arc<tokio::sync::RwLock<SyncSettings>>,
        Arc<String>,
        Option<Arc<crate::server::ServerManager>>,
        RunMode,
    );
    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        (
            params.sync_settings.clone(),
            params.config_path.clone(),
            params.server_manager.get().cloned(),
            params.node.run_mode,
        )
    }
}
impl Signal for crate::messages::SetSyncSettingsRequest {
    type Params = (
        Arc<tokio::sync::RwLock<SyncSettings>>,
        Arc<String>,
        Option<Arc<crate::server::ServerManager>>,
        RunMode,
    );
    type Response = crate::messages::SetSyncSettingsResponse;
    async fn handle(
        &self,
        (settings, config, manager, run_mode): Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
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
                    let info = load_device_info(Path::new(&**config), run_mode).await?;
                    manager
                        .start_sync(
                            ([0, 0, 0, 0], DEFAULT_DEVICE_PORT).into(),
                            discovery::DiscoveryParams { device_info: info },
                        )
                        .await?;
                }
            } else if manager.is_running() {
                manager.stop().await?;
            }
        }
        Ok(Some(Self::Response {
            call_id: None,
            request_id: request.request_id.clone(),
        }))
    }
}
