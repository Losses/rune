use crate::node::NodeContext;
use crate::request_error::ExpectedRequestError;
use crate::{
    Session, Signal,
    library_pool::LibraryHandle,
    messages::{
        LibraryContentChanged, RemoveDownloadRuleRequest, RemoveDownloadRuleResponse,
        SetDownloadRuleRequest, SetDownloadRuleResponse,
    },
    server::ServerManager,
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
            media_server: settings.media_server,
            broadcast: settings.broadcast,
            accept_remote_control: settings.accept_remote_control,
            accept_sync: settings.accept_sync,
            missing_host_file: settings.missing_host_file.into(),
        }))
    }
}

async fn start_media_server(
    manager: &Arc<ServerManager>,
    node: &NodeContext,
    broadcast: bool,
) -> Result<()> {
    let device_info = node.device_info().await?;
    manager
        .clone()
        .start(
            ([0, 0, 0, 0], DEFAULT_DEVICE_PORT).into(),
            discovery::DiscoveryParams { device_info },
            broadcast,
        )
        .await
}

/// Changes the sync settings and makes the media server match them. Every writer of the
/// settings goes through here.
///
/// The server is reconciled on every call instead of only when a value changes: an earlier
/// attempt may have saved a setting without the server reaching that state. The settings are
/// persisted only after the server reached the requested state, so a failed attempt leaves
/// the saved and displayed settings equal to what is actually served. If the save itself
/// fails, the server is put back to the state the saved settings still describe.
///
/// `change` is applied twice: to the settings in use, and to the saved ones. A process may
/// run with settings it forced on for itself only, and those must not reach the file when
/// another setting changes.
pub(crate) async fn update_sync_settings(
    node: &NodeContext,
    change: impl Fn(&mut SyncSettings),
) -> Result<()> {
    let _update = node.sync_settings_update.lock().await;
    let current = node.sync_settings.read().await.clone();
    let mut next = current.clone();
    change(&mut next);

    let manager = node.server_manager.get();
    let running = manager.is_some_and(|manager| manager.is_running());
    let started = next.media_server && !running;
    let stopped = !next.media_server && running;
    if next.media_server {
        let manager = manager.ok_or(ExpectedRequestError::Unavailable {
            what: "server manager",
        })?;
        if started {
            start_media_server(manager, node, next.broadcast).await?;
        }
    } else if stopped
        && let Some(manager) = manager
        && let Err(stop_error) = manager.stop().await
    {
        // stop() clears the server state before returning a join error, so the server may
        // already be gone while the saved setting still says on.
        let restore = if manager.is_running() {
            Ok(())
        } else {
            start_media_server(manager, node, current.broadcast).await
        };
        return Err(match restore {
            Ok(()) => stop_error,
            Err(restore_error) => stop_error.context(format!(
                "the server could not be restored after stopping it failed: {restore_error:#}"
            )),
        });
    }
    if let Some(manager) = manager
        && let Err(broadcast_error) = manager.set_broadcast(next.broadcast).await
    {
        // Announcing needs a multicast-capable network. A device without one must still be
        // able to change its other settings, so only a request that turns announcing on
        // fails when it cannot start.
        if current.broadcast {
            log::warn!("Could not announce this device: {broadcast_error:#}");
        } else {
            let restore = if started {
                manager.stop().await
            } else {
                Ok(())
            };
            return Err(match restore {
                Ok(()) => broadcast_error,
                Err(restore_error) => broadcast_error.context(format!(
                    "the server could not be restored after announcing failed: {restore_error:#}"
                )),
            });
        }
    }
    let config_path = Path::new(&**node.config_path);
    let mut saved = SyncSettings::load(config_path);
    change(&mut saved);
    if let Err(save_error) = saved.save(config_path) {
        let restore = match manager {
            Some(manager) if started && manager.is_running() => manager.stop().await,
            Some(manager) if stopped && !manager.is_running() => {
                start_media_server(manager, node, current.broadcast).await
            }
            Some(manager) if next.broadcast != current.broadcast => {
                manager.set_broadcast(current.broadcast).await
            }
            _ => Ok(()),
        };
        return Err(match restore {
            Ok(()) => save_error,
            Err(restore_error) => save_error.context(format!(
                "the server could not be restored after the settings failed to save: {restore_error:#}"
            )),
        });
    }
    let remote_control_disabled = current.accept_remote_control && !next.accept_remote_control;
    *node.sync_settings.write().await = next;
    // The new value is stored first, so a controller that reconnects is refused before the
    // open sessions are closed.
    if remote_control_disabled && let Some(manager) = manager {
        manager.disconnect_remote_control();
    }
    Ok(())
}

impl ParamsExtractor for crate::messages::SetSyncSettingsRequest {
    type Params = Arc<NodeContext>;
    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}
impl Signal for crate::messages::SetSyncSettingsRequest {
    type Params = Arc<NodeContext>;
    type Response = crate::messages::SetSyncSettingsResponse;
    async fn handle(
        &self,
        node: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        update_sync_settings(&node, |settings| {
            settings.media_server = request.media_server;
            settings.broadcast = request.broadcast;
            settings.accept_remote_control = request.accept_remote_control;
            settings.accept_sync = request.accept_sync;
            settings.missing_host_file = request.missing_host_file.into();
        })
        .await?;
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

    /// Changes the sync settings the way the Devices page does: every field is sent, the
    /// changed one together with the ones the page loaded.
    async fn set_sync_settings(
        node: &Arc<NodeContext>,
        change: impl FnOnce(&mut SetSyncSettingsRequest),
    ) -> Result<()> {
        let shown = shown_settings(node).await?;
        let mut request = SetSyncSettingsRequest {
            request_id: "set-sync-settings".into(),
            media_server: shown.media_server,
            broadcast: shown.broadcast,
            accept_remote_control: shown.accept_remote_control,
            accept_sync: shown.accept_sync,
            missing_host_file: shown.missing_host_file,
            call_id: None,
        };
        change(&mut request);
        request
            .handle(Arc::clone(node), None, &request)
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

    async fn node_in(dir: &Path) -> Result<Arc<NodeContext>> {
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

    /// A node whose saved settings keep the server off, so that node start binds nothing.
    async fn test_node(dir: &Path) -> Result<Arc<NodeContext>> {
        crate::test_node::disable_media_server(dir)?;
        node_in(dir).await
    }

    /// Starts the server on a loopback port instead of the fixed device port, and makes the
    /// settings describe it. Returns the bound address.
    async fn start_on_loopback(node: &Arc<NodeContext>, broadcast: bool) -> Result<SocketAddr> {
        let manager = node.manager()?;
        let settings = SyncSettings {
            media_server: true,
            broadcast,
            ..node.sync_settings.read().await.clone()
        };
        manager
            .clone()
            .start(
                ([127, 0, 0, 1], 0).into(),
                discovery::DiscoveryParams {
                    device_info: node.device_info().await?,
                },
                broadcast,
            )
            .await?;
        settings.save(Path::new(&**node.config_path))?;
        *node.sync_settings.write().await = settings;
        manager.get_address().await.context("server address")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn failed_enable_keeps_the_setting_off_and_the_retry_starts_the_server() -> Result<()> {
        let _sync_port = crate::test_node::SYNC_PORT.lock().await;
        let dir = tempfile::tempdir()?;
        let node = test_node(dir.path()).await?;
        let manager = node.manager()?;
        assert!(!manager.is_running());

        // The media server always binds the default device port, so holding it here makes an
        // enable request fail at the socket bind, before the server runs.
        let occupied = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], DEFAULT_DEVICE_PORT)))
            .context("occupy the device port")?;
        for attempt in 1..=2 {
            let error = set_sync_settings(&node, |request| request.media_server = true)
                .await
                .err()
                .unwrap_or_else(|| {
                    panic!("attempt {attempt}: enabling while the device port is occupied")
                });
            assert!(
                format!("{error:#}").contains("Failed to bind sync server"),
                "attempt {attempt}: {error:#}"
            );
        }
        assert!(!manager.is_running());
        assert!(!node.device_scanner.is_announcing().await);
        assert!(
            !SyncSettings::load(dir.path()).media_server,
            "a failed enable left the saved setting enabled without a server"
        );
        assert!(
            !shown_settings(&node).await?.media_server,
            "a failed enable left the shown setting enabled without a server"
        );
        drop(occupied);

        set_sync_settings(&node, |request| request.media_server = true).await?;
        assert!(manager.is_running());
        assert!(SyncSettings::load(dir.path()).media_server);
        assert!(shown_settings(&node).await?.media_server);
        let address = manager.get_address().await;

        // A save failure must not stop a server that was already running before the request:
        // the saved setting still says enabled.
        {
            let blocked = BlockedSettingsFile::new(dir.path())?;
            let error = set_sync_settings(&node, |request| {
                request.missing_host_file = MissingHostFilePolicy::KeepAsRemote
            })
            .await
            .expect_err("changing a setting while the settings cannot be saved");
            assert!(
                format!("{error:#}").contains("replace sync settings"),
                "{error:#}"
            );
            assert_eq!(
                manager.get_address().await,
                address,
                "an already running server was stopped or restarted"
            );
            assert!(shown_settings(&node).await?.media_server);
            assert!(matches!(
                shown_settings(&node).await?.missing_host_file,
                MissingHostFilePolicy::RemoveEverywhere
            ));
            drop(blocked);
            assert!(SyncSettings::load(dir.path()).media_server);
        }

        // The same request succeeds without starting a second server.
        set_sync_settings(&node, |request| {
            request.missing_host_file = MissingHostFilePolicy::KeepAsRemote
        })
        .await?;
        assert_eq!(manager.get_address().await, address);
        let shown = shown_settings(&node).await?;
        assert!(shown.media_server);
        assert!(matches!(
            shown.missing_host_file,
            MissingHostFilePolicy::KeepAsRemote
        ));

        // A disable that cannot be saved must restart the server the saved setting still
        // describes, announcing as that setting says.
        {
            let blocked = BlockedSettingsFile::new(dir.path())?;
            let error = set_sync_settings(&node, |request| request.media_server = false)
                .await
                .expect_err("disabling while the settings cannot be saved");
            assert!(
                format!("{error:#}").contains("replace sync settings"),
                "{error:#}"
            );
            assert!(manager.is_running(), "the stopped server was not restarted");
            assert!(node.device_scanner.is_announcing().await);
            let shown = shown_settings(&node).await?;
            assert!(shown.media_server);
            assert!(matches!(
                shown.missing_host_file,
                MissingHostFilePolicy::KeepAsRemote
            ));
            drop(blocked);
            let saved = SyncSettings::load(dir.path());
            assert!(saved.media_server);
            assert_eq!(
                saved.missing_host_file,
                SavedMissingHostFilePolicy::KeepAsRemote
            );
        }

        set_sync_settings(&node, |request| request.media_server = false).await?;
        assert!(!manager.is_running());
        let saved = SyncSettings::load(dir.path());
        assert!(!saved.media_server);
        assert_eq!(
            saved.missing_host_file,
            SavedMissingHostFilePolicy::KeepAsRemote
        );
        assert!(!shown_settings(&node).await?.media_server);

        // An enable that starts the server but cannot be saved must stop it again: the saved
        // setting still says disabled.
        {
            let blocked = BlockedSettingsFile::new(dir.path())?;
            let error = set_sync_settings(&node, |request| request.media_server = true)
                .await
                .expect_err("enabling while the settings cannot be saved");
            assert!(
                format!("{error:#}").contains("replace sync settings"),
                "{error:#}"
            );
            assert!(
                !manager.is_running(),
                "the server started by the failed enable was kept"
            );
            assert!(!node.device_scanner.is_announcing().await);
            assert!(!shown_settings(&node).await?.media_server);
            drop(blocked);
            assert!(!SyncSettings::load(dir.path()).media_server);
        }

        set_sync_settings(&node, |request| request.media_server = true).await?;
        assert!(manager.is_running());
        assert!(SyncSettings::load(dir.path()).media_server);
        assert!(shown_settings(&node).await?.media_server);

        node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn media_server_switch_stops_and_starts_the_server_and_its_announcements() -> Result<()> {
        let _sync_port = crate::test_node::SYNC_PORT.lock().await;
        let dir = tempfile::tempdir()?;
        let node = test_node(dir.path()).await?;
        let manager = node.manager()?;
        start_on_loopback(&node, true).await?;
        assert!(manager.is_running());
        assert!(node.device_scanner.is_announcing().await);

        set_sync_settings(&node, |request| request.media_server = false).await?;
        assert!(!manager.is_running());
        assert_eq!(manager.get_address().await, None);
        assert!(!node.device_scanner.is_announcing().await);
        let saved = SyncSettings::load(dir.path());
        assert!(!saved.media_server);
        assert!(
            saved.broadcast,
            "stopping the server changed another setting"
        );

        set_sync_settings(&node, |request| request.media_server = true).await?;
        assert!(manager.is_running());
        assert_eq!(
            manager.get_address().await.map(|address| address.port()),
            Some(DEFAULT_DEVICE_PORT)
        );
        assert!(node.device_scanner.is_announcing().await);
        assert!(SyncSettings::load(dir.path()).media_server);

        node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn node_start_keeps_the_server_off_for_a_setting_saved_as_listen() -> Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("sync_settings.json"), b"{\"listen\":false}")?;
        let node = node_in(dir.path()).await?;
        let manager = node.manager()?;

        assert!(!manager.is_running());
        assert_eq!(manager.get_address().await, None);
        assert!(!node.device_scanner.is_announcing().await);
        let shown = shown_settings(&node).await?;
        assert!(!shown.media_server);
        assert!(shown.broadcast);
        assert!(!shown.accept_remote_control);
        assert!(shown.accept_sync);

        node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn broadcast_switch_starts_and_stops_announcing_without_a_restart() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let node = test_node(dir.path()).await?;
        let manager = node.manager()?;
        let address = start_on_loopback(&node, false).await?;
        assert!(manager.is_running());
        assert!(!node.device_scanner.is_announcing().await);

        set_sync_settings(&node, |request| request.broadcast = true).await?;
        assert!(node.device_scanner.is_announcing().await);
        assert_eq!(manager.get_address().await, Some(address));
        assert!(SyncSettings::load(dir.path()).broadcast);

        // Repeating the request keeps the announcement that is already running.
        set_sync_settings(&node, |request| request.broadcast = true).await?;
        assert!(node.device_scanner.is_announcing().await);
        assert_eq!(manager.get_address().await, Some(address));

        set_sync_settings(&node, |request| request.broadcast = false).await?;
        assert!(!node.device_scanner.is_announcing().await);
        assert_eq!(manager.get_address().await, Some(address));
        assert!(!SyncSettings::load(dir.path()).broadcast);

        node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn failed_save_puts_announcing_back_to_the_saved_setting() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let node = test_node(dir.path()).await?;
        let manager = node.manager()?;
        let address = start_on_loopback(&node, true).await?;
        assert!(node.device_scanner.is_announcing().await);

        let blocked = BlockedSettingsFile::new(dir.path())?;
        let error = set_sync_settings(&node, |request| request.broadcast = false)
            .await
            .expect_err("switching broadcast off while the settings cannot be saved");
        assert!(
            format!("{error:#}").contains("replace sync settings"),
            "{error:#}"
        );
        assert!(
            node.device_scanner.is_announcing().await,
            "announcing stayed off although the saved setting says on"
        );
        assert_eq!(manager.get_address().await, Some(address));
        assert!(shown_settings(&node).await?.broadcast);
        drop(blocked);
        assert!(SyncSettings::load(dir.path()).broadcast);

        node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn broadcast_switch_is_only_saved_while_the_server_is_off() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let node = test_node(dir.path()).await?;
        let manager = node.manager()?;

        set_sync_settings(&node, |request| request.broadcast = false).await?;
        assert!(!manager.is_running());
        assert!(!node.device_scanner.is_announcing().await);
        assert!(!SyncSettings::load(dir.path()).broadcast);

        set_sync_settings(&node, |request| request.broadcast = true).await?;
        assert!(!manager.is_running());
        assert!(!node.device_scanner.is_announcing().await);
        let saved = SyncSettings::load(dir.path());
        assert!(saved.broadcast);
        assert!(!saved.media_server);

        node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_setting_forced_on_in_memory_is_not_saved_by_a_later_change() -> Result<()> {
        let dir = tempfile::tempdir()?;
        SyncSettings {
            media_server: false,
            accept_remote_control: false,
            ..SyncSettings::default()
        }
        .save(dir.path())?;
        let node = node_in(dir.path()).await?;
        node.sync_settings.write().await.accept_remote_control = true;

        update_sync_settings(&node, |settings| settings.broadcast = false).await?;

        let saved = SyncSettings::load(dir.path());
        assert!(!saved.broadcast);
        assert!(
            !saved.accept_remote_control,
            "a setting forced on for this process was written to the file"
        );
        let in_use = node.sync_settings.read().await.clone();
        assert!(!in_use.broadcast);
        assert!(
            in_use.accept_remote_control,
            "the forced setting was lost from the running process"
        );

        node.shutdown().await?;
        Ok(())
    }
}
