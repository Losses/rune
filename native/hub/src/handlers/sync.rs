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
use tokio::sync as tokio_sync;

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
    type Params = Arc<tokio_sync::RwLock<SyncSettings>>;
    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.sync_settings.clone()
    }
}
impl Signal for crate::messages::GetSyncSettingsRequest {
    type Params = Arc<tokio_sync::RwLock<SyncSettings>>;
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

async fn start_paired_sync_listener(
    manager: &Arc<crate::server::ServerManager>,
    config: &Arc<String>,
    run_mode: RunMode,
) -> Result<()> {
    let info = load_device_info(Path::new(&**config), run_mode).await?;
    manager
        .clone()
        .start_sync(
            ([0, 0, 0, 0], DEFAULT_DEVICE_PORT).into(),
            discovery::DiscoveryParams { device_info: info },
        )
        .await
}

impl ParamsExtractor for crate::messages::SetSyncSettingsRequest {
    type Params = (
        Arc<tokio_sync::RwLock<SyncSettings>>,
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
        Arc<tokio_sync::RwLock<SyncSettings>>,
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
        // The setting describes the paired sync listener, so reconcile that listener on every
        // request instead of only when the stored value changes: an earlier attempt may have
        // saved the setting without starting the listener. A full server started through
        // `StartServerRequest` is independent and must never be stopped or replaced here.
        // Persist only after the listener reached the requested state, so a failed attempt
        // leaves the saved and displayed setting equal to the state that is actually served.
        // If the save itself fails, put the sync listener back to the state the saved setting
        // still describes.
        let was_sync_listener = manager
            .as_ref()
            .is_some_and(|manager| manager.is_sync_listener());
        let any_server_running = manager.as_ref().is_some_and(|manager| manager.is_running());
        let started = request.listen && !any_server_running;
        let stopped = !request.listen && was_sync_listener;
        if request.listen {
            let manager = manager.as_ref().ok_or(ExpectedRequestError::Unavailable {
                what: "server manager",
            })?;
            if started {
                start_paired_sync_listener(manager, &config, run_mode).await?;
            }
        } else if stopped
            && let Some(manager) = manager.as_ref()
            && let Err(stop_error) = manager.stop().await
        {
            // stop() clears the listener state before returning a join error, so the
            // sync listener may already be gone while the saved setting still says on.
            let restore = if manager.is_running() {
                Ok(())
            } else {
                start_paired_sync_listener(manager, &config, run_mode).await
            };
            return Err(match restore {
                Ok(()) => stop_error,
                Err(restore_error) => stop_error.context(format!(
                    "the sync listener could not be restored after stopping it failed: {restore_error:#}"
                )),
            });
        }
        let next = SyncSettings {
            listen: request.listen,
            missing_host_file: policy,
        };
        if let Err(save_error) = next.save(Path::new(&**config)) {
            let restore = match manager.as_ref() {
                Some(manager) if started && manager.is_sync_listener() => manager.stop().await,
                Some(manager) if stopped && !manager.is_running() => {
                    start_paired_sync_listener(manager, &config, run_mode).await
                }
                _ => Ok(()),
            };
            return Err(match restore {
                Ok(()) => save_error,
                Err(restore_error) => save_error.context(format!(
                    "the sync listener could not be restored after the settings failed to save: {restore_error:#}"
                )),
            });
        }
        *current = next;
        Ok(Some(Self::Response {
            call_id: None,
            request_id: request.request_id.clone(),
        }))
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;
    use crate::messages::{GetSyncSettingsRequest, MissingHostFilePolicy, SetSyncSettingsRequest};
    use crate::server::utils::device::RunMode;
    use crate::utils::LocalGuiBroadcaster;
    use anyhow::Context;
    use database::actions::metadata::MissingHostFilePolicy as SavedMissingHostFilePolicy;
    use std::net::{SocketAddr, TcpListener};

    async fn set_sync_settings(
        node: &Arc<NodeContext>,
        listen: bool,
        missing_host_file: MissingHostFilePolicy,
    ) -> Result<()> {
        let request = SetSyncSettingsRequest {
            request_id: "set-sync-settings".into(),
            listen,
            missing_host_file,
            call_id: None,
        };
        request
            .handle(
                (
                    Arc::clone(&node.sync_settings),
                    Arc::clone(&node.config_path),
                    node.server_manager.get().cloned(),
                    node.run_mode,
                ),
                None,
                &request,
            )
            .await
            .map(|_| ())
    }

    /// What the Devices page loads and shows.
    async fn shown_settings(
        node: &Arc<NodeContext>,
    ) -> Result<crate::messages::GetSyncSettingsResponse> {
        let request = GetSyncSettingsRequest {
            request_id: "get-sync-settings".into(),
            call_id: None,
        };
        request
            .handle(Arc::clone(&node.sync_settings), None, &request)
            .await?
            .context("sync settings response")
    }

    /// Replaces the settings file with a directory, so the next save fails at the rename with
    /// `EISDIR` regardless of the process's write permissions. The original file is restored
    /// when dropped, including while unwinding from a failed assertion.
    struct BlockedSettingsFile {
        path: std::path::PathBuf,
        contents: Vec<u8>,
    }

    impl BlockedSettingsFile {
        fn new(config_dir: &Path) -> Result<Self> {
            let path = config_dir.join("sync_settings.json");
            let contents = std::fs::read(&path)?;
            std::fs::remove_file(&path)?;
            std::fs::create_dir(&path)?;
            Ok(Self { path, contents })
        }
    }

    impl Drop for BlockedSettingsFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir(&self.path);
            let _ = std::fs::write(&self.path, &self.contents);
        }
    }

    async fn test_node(dir: &Path) -> Result<Arc<NodeContext>> {
        crate::test_node::disable_sync_listener(dir)?;
        let config = Arc::new(dir.to_string_lossy().into_owned());
        NodeContext::new(
            &config,
            &config,
            Arc::new(LocalGuiBroadcaster),
            false,
            RunMode::App,
            Arc::new(::visualizer::Visualizer::new()),
        )
        .await
    }

    /// Starts the full server the way `StartServerRequest` does, and returns its bound address.
    async fn start_full_server(node: &Arc<NodeContext>) -> Result<SocketAddr> {
        let manager = node.manager()?;
        manager
            .clone()
            .start(
                ([127, 0, 0, 1], 0).into(),
                discovery::DiscoveryParams {
                    device_info: node.device_info().await?,
                },
            )
            .await?;
        manager.get_address().await.context("full server address")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn failed_enable_keeps_the_setting_off_and_the_retry_starts_the_listener() -> Result<()> {
        let _sync_port = crate::test_node::SYNC_PORT.lock().await;
        let dir = tempfile::tempdir()?;
        let node = test_node(dir.path()).await?;
        let manager = node.manager()?;
        assert!(!manager.is_running());

        // The paired sync listener always binds the default device port, so holding it here
        // makes an enable request fail at the socket bind, before the listener runs.
        let occupied = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], DEFAULT_DEVICE_PORT)))
            .context("occupy the paired sync port")?;
        for attempt in 1..=2 {
            let error = set_sync_settings(&node, true, MissingHostFilePolicy::RemoveEverywhere)
                .await
                .err()
                .unwrap_or_else(|| {
                    panic!("attempt {attempt}: enabling while the paired sync port is occupied")
                });
            assert!(
                format!("{error:#}").contains("Failed to bind sync server"),
                "attempt {attempt}: {error:#}"
            );
        }
        assert!(!manager.is_running());
        assert!(
            !SyncSettings::load(dir.path()).listen,
            "a failed enable left the saved setting enabled without a listener"
        );
        assert!(
            !shown_settings(&node).await?.listen,
            "a failed enable left the shown setting enabled without a listener"
        );
        drop(occupied);

        set_sync_settings(&node, true, MissingHostFilePolicy::RemoveEverywhere).await?;
        assert!(manager.is_running());
        assert!(SyncSettings::load(dir.path()).listen);
        assert!(shown_settings(&node).await?.listen);

        // A save failure must not stop a listener that was already running before the
        // request: the saved setting still says enabled.
        {
            let blocked = BlockedSettingsFile::new(dir.path())?;
            let error = set_sync_settings(&node, true, MissingHostFilePolicy::RemoveEverywhere)
                .await
                .expect_err("enabling while the settings cannot be saved");
            assert!(
                format!("{error:#}").contains("replace sync settings"),
                "{error:#}"
            );
            assert!(
                manager.is_running(),
                "an already running listener was stopped"
            );
            assert!(shown_settings(&node).await?.listen);
            drop(blocked);
            assert!(SyncSettings::load(dir.path()).listen);
        }

        // The same enable request succeeds without starting a second listener.
        set_sync_settings(&node, true, MissingHostFilePolicy::KeepAsRemote).await?;
        assert!(manager.is_running());
        let shown = shown_settings(&node).await?;
        assert!(shown.listen);
        assert!(matches!(
            shown.missing_host_file,
            MissingHostFilePolicy::KeepAsRemote
        ));

        // A disable that cannot be saved must restart the listener the saved setting still
        // describes.
        {
            let blocked = BlockedSettingsFile::new(dir.path())?;
            let error = set_sync_settings(&node, false, MissingHostFilePolicy::KeepAsRemote)
                .await
                .expect_err("disabling while the settings cannot be saved");
            assert!(
                format!("{error:#}").contains("replace sync settings"),
                "{error:#}"
            );
            assert!(
                manager.is_running(),
                "the stopped listener was not restarted"
            );
            let shown = shown_settings(&node).await?;
            assert!(shown.listen);
            assert!(matches!(
                shown.missing_host_file,
                MissingHostFilePolicy::KeepAsRemote
            ));
            drop(blocked);
            let saved = SyncSettings::load(dir.path());
            assert!(saved.listen);
            assert_eq!(
                saved.missing_host_file,
                SavedMissingHostFilePolicy::KeepAsRemote
            );
        }

        set_sync_settings(&node, false, MissingHostFilePolicy::KeepAsRemote).await?;
        assert!(!manager.is_running());
        let saved = SyncSettings::load(dir.path());
        assert!(!saved.listen);
        assert_eq!(
            saved.missing_host_file,
            SavedMissingHostFilePolicy::KeepAsRemote
        );
        assert!(!shown_settings(&node).await?.listen);

        // An enable that starts the listener but cannot be saved must stop it again: the
        // saved setting still says disabled.
        {
            let blocked = BlockedSettingsFile::new(dir.path())?;
            let error = set_sync_settings(&node, true, MissingHostFilePolicy::KeepAsRemote)
                .await
                .expect_err("enabling while the settings cannot be saved");
            assert!(
                format!("{error:#}").contains("replace sync settings"),
                "{error:#}"
            );
            assert!(
                !manager.is_running(),
                "the listener started by the failed enable was kept"
            );
            assert!(!shown_settings(&node).await?.listen);
            drop(blocked);
            assert!(!SyncSettings::load(dir.path()).listen);
        }

        set_sync_settings(&node, true, MissingHostFilePolicy::KeepAsRemote).await?;
        assert!(manager.is_running());
        assert!(SyncSettings::load(dir.path()).listen);
        assert!(shown_settings(&node).await?.listen);

        node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn disabled_setting_keeps_an_independent_full_server_running() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let node = test_node(dir.path()).await?;
        let manager = node.manager()?;
        let address = start_full_server(&node).await?;
        assert!(manager.is_running());
        assert!(!manager.is_sync_listener());

        // The saved setting is already off, so this request only persists the policy.
        set_sync_settings(&node, false, MissingHostFilePolicy::KeepAsRemote).await?;

        assert!(manager.is_running(), "the full server was stopped");
        assert_eq!(manager.get_address().await, Some(address));
        assert!(
            !manager.is_sync_listener(),
            "the full server became a sync listener"
        );
        assert!(!SyncSettings::load(dir.path()).listen);

        node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn failed_save_keeps_an_independent_full_server_running() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let node = test_node(dir.path()).await?;
        let manager = node.manager()?;

        // A full server is already serving, so enabling the setting must not start a
        // sync listener and disabling it must not stop the full server.
        let address = start_full_server(&node).await?;
        set_sync_settings(&node, true, MissingHostFilePolicy::RemoveEverywhere).await?;
        assert!(manager.is_running());
        assert!(!manager.is_sync_listener());

        let blocked = BlockedSettingsFile::new(dir.path())?;
        let error = set_sync_settings(&node, false, MissingHostFilePolicy::KeepAsRemote)
            .await
            .expect_err("disabling while the settings cannot be saved");
        assert!(
            format!("{error:#}").contains("replace sync settings"),
            "{error:#}"
        );
        assert!(manager.is_running(), "the full server was stopped");
        assert_eq!(manager.get_address().await, Some(address));
        assert!(
            !manager.is_sync_listener(),
            "the full server was replaced by a sync listener"
        );
        drop(blocked);
        assert!(SyncSettings::load(dir.path()).listen);

        node.shutdown().await?;
        Ok(())
    }
}
