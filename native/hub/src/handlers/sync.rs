use crate::node::NodeContext;
use crate::request_error::ExpectedRequestError;
use crate::{
    Session, Signal,
    library_pool::LibraryHandle,
    messages::{
        LibraryContentChanged, RemoveDownloadRuleRequest, RemoveDownloadRuleResponse,
        SetDownloadRuleRequest, SetDownloadRuleResponse, SyncShareLibraryRequest,
        SyncShareLibraryResponse,
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
use uuid::Uuid;

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
            .own_player_queue_for_library(library.library_id)
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
            auto_device_sync: settings.auto_device_sync,
            auto_share_sync: settings.auto_share_sync,
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
            if let Some(media_server) = request.media_server {
                settings.media_server = media_server;
            }
            if let Some(broadcast) = request.broadcast {
                settings.broadcast = broadcast;
            }
            if let Some(accept_remote_control) = request.accept_remote_control {
                settings.accept_remote_control = accept_remote_control;
            }
            if let Some(accept_sync) = request.accept_sync {
                settings.accept_sync = accept_sync;
            }
            if let Some(auto_device_sync) = request.auto_device_sync {
                settings.auto_device_sync = auto_device_sync;
            }
            if let Some(auto_share_sync) = request.auto_share_sync {
                settings.auto_share_sync = auto_share_sync;
            }
            if let Some(missing_host_file) = request.missing_host_file {
                settings.missing_host_file = missing_host_file.into();
            }
        })
        .await?;
        Ok(Some(Self::Response {
            call_id: None,
            request_id: request.request_id.clone(),
        }))
    }
}

impl ParamsExtractor for SyncShareLibraryRequest {
    type Params = Arc<NodeContext>;
    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}
impl Signal for SyncShareLibraryRequest {
    type Params = Arc<NodeContext>;
    type Response = SyncShareLibraryResponse;
    async fn handle(
        &self,
        node: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let library_id = Uuid::parse_str(&request.library_id).map_err(|error| {
            ExpectedRequestError::invalid_input(format!("invalid library UUID: {error}"))
        })?;
        let share_failure = node
            .share_sync_requests
            .sync(library_id, &node.token)
            .await?
            .err();
        Ok(Some(Self::Response {
            share_failure,
            call_id: None,
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

    /// Changes the sync settings the way the pages do: only the changed fields are sent.
    async fn set_sync_settings(
        node: &Arc<NodeContext>,
        change: impl FnOnce(&mut SetSyncSettingsRequest),
    ) -> Result<()> {
        let mut request = SetSyncSettingsRequest {
            request_id: "set-sync-settings".into(),
            media_server: None,
            broadcast: None,
            accept_remote_control: None,
            accept_sync: None,
            auto_device_sync: None,
            auto_share_sync: None,
            missing_host_file: None,
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
            let error = set_sync_settings(&node, |request| request.media_server = Some(true))
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

        set_sync_settings(&node, |request| request.media_server = Some(true)).await?;
        assert!(manager.is_running());
        assert!(SyncSettings::load(dir.path()).media_server);
        assert!(shown_settings(&node).await?.media_server);
        let address = manager.get_address().await;

        // A save failure must not stop a server that was already running before the request:
        // the saved setting still says enabled.
        {
            let blocked = BlockedSettingsFile::new(dir.path())?;
            let error = set_sync_settings(&node, |request| {
                request.missing_host_file = Some(MissingHostFilePolicy::KeepAsRemote)
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
            request.missing_host_file = Some(MissingHostFilePolicy::KeepAsRemote)
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
            let error = set_sync_settings(&node, |request| request.media_server = Some(false))
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

        set_sync_settings(&node, |request| request.media_server = Some(false)).await?;
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
            let error = set_sync_settings(&node, |request| request.media_server = Some(true))
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

        set_sync_settings(&node, |request| request.media_server = Some(true)).await?;
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

        set_sync_settings(&node, |request| request.media_server = Some(false)).await?;
        assert!(!manager.is_running());
        assert_eq!(manager.get_address().await, None);
        assert!(!node.device_scanner.is_announcing().await);
        let saved = SyncSettings::load(dir.path());
        assert!(!saved.media_server);
        assert!(
            saved.broadcast,
            "stopping the server changed another setting"
        );

        set_sync_settings(&node, |request| request.media_server = Some(true)).await?;
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

        set_sync_settings(&node, |request| request.broadcast = Some(true)).await?;
        assert!(node.device_scanner.is_announcing().await);
        assert_eq!(manager.get_address().await, Some(address));
        assert!(SyncSettings::load(dir.path()).broadcast);

        // Repeating the request keeps the announcement that is already running.
        set_sync_settings(&node, |request| request.broadcast = Some(true)).await?;
        assert!(node.device_scanner.is_announcing().await);
        assert_eq!(manager.get_address().await, Some(address));

        set_sync_settings(&node, |request| request.broadcast = Some(false)).await?;
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
        let error = set_sync_settings(&node, |request| request.broadcast = Some(false))
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

        set_sync_settings(&node, |request| request.broadcast = Some(false)).await?;
        assert!(!manager.is_running());
        assert!(!node.device_scanner.is_announcing().await);
        assert!(!SyncSettings::load(dir.path()).broadcast);

        set_sync_settings(&node, |request| request.broadcast = Some(true)).await?;
        assert!(!manager.is_running());
        assert!(!node.device_scanner.is_announcing().await);
        let saved = SyncSettings::load(dir.path());
        assert!(saved.broadcast);
        assert!(!saved.media_server);

        node.shutdown().await?;
        Ok(())
    }

    /// Settings in which every field differs from its default, so that a field a request
    /// left alone can be told from one it reset.
    fn non_default_settings() -> SyncSettings {
        SyncSettings {
            media_server: false,
            broadcast: false,
            accept_remote_control: true,
            accept_sync: false,
            auto_device_sync: false,
            auto_share_sync: false,
            missing_host_file: SavedMissingHostFilePolicy::KeepAsRemote,
        }
    }

    /// Brings a node to [`non_default_settings`] through requests, one field at a time.
    async fn set_every_setting_to_non_default(node: &Arc<NodeContext>) -> Result<()> {
        set_sync_settings(node, |request| request.media_server = Some(false)).await?;
        set_sync_settings(node, |request| request.broadcast = Some(false)).await?;
        set_sync_settings(node, |request| request.accept_remote_control = Some(true)).await?;
        set_sync_settings(node, |request| request.accept_sync = Some(false)).await?;
        set_sync_settings(node, |request| request.auto_device_sync = Some(false)).await?;
        set_sync_settings(node, |request| request.auto_share_sync = Some(false)).await?;
        set_sync_settings(node, |request| {
            request.missing_host_file = Some(MissingHostFilePolicy::KeepAsRemote)
        })
        .await
    }

    /// Asserts that both the settings in use and the saved ones equal `expected`.
    async fn assert_settings(
        node: &Arc<NodeContext>,
        config_dir: &Path,
        expected: &SyncSettings,
        context: &str,
    ) -> Result<()> {
        let expected = serde_json::to_value(expected)?;
        assert_eq!(
            serde_json::to_value(&*node.sync_settings.read().await)?,
            expected,
            "{context}: settings in use"
        );
        assert_eq!(
            serde_json::to_value(SyncSettings::load(config_dir))?,
            expected,
            "{context}: saved settings"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_request_with_one_field_changes_that_field_and_leaves_every_other() -> Result<()> {
        type Case = (
            &'static str,
            fn(&mut SetSyncSettingsRequest),
            fn(&mut SyncSettings),
        );
        // Each case puts one field back to its default. The media server comes last: it is
        // the only one that binds the device port.
        let cases: [Case; 7] = [
            (
                "broadcast",
                |request| request.broadcast = Some(true),
                |settings| settings.broadcast = true,
            ),
            (
                "accept_remote_control",
                |request| request.accept_remote_control = Some(false),
                |settings| settings.accept_remote_control = false,
            ),
            (
                "accept_sync",
                |request| request.accept_sync = Some(true),
                |settings| settings.accept_sync = true,
            ),
            (
                "auto_device_sync",
                |request| request.auto_device_sync = Some(true),
                |settings| settings.auto_device_sync = true,
            ),
            (
                "auto_share_sync",
                |request| request.auto_share_sync = Some(true),
                |settings| settings.auto_share_sync = true,
            ),
            (
                "missing_host_file",
                |request| request.missing_host_file = Some(MissingHostFilePolicy::RemoveEverywhere),
                |settings| {
                    settings.missing_host_file = SavedMissingHostFilePolicy::RemoveEverywhere
                },
            ),
            (
                "media_server",
                |request| request.media_server = Some(true),
                |settings| settings.media_server = true,
            ),
        ];
        let _sync_port = crate::test_node::SYNC_PORT.lock().await;

        for (field, change_request, change_expected) in cases {
            let dir = tempfile::tempdir()?;
            let node = test_node(dir.path()).await?;
            set_every_setting_to_non_default(&node).await?;
            assert_settings(&node, dir.path(), &non_default_settings(), field).await?;

            set_sync_settings(&node, change_request).await?;

            let mut expected = non_default_settings();
            change_expected(&mut expected);
            assert_settings(&node, dir.path(), &expected, field).await?;
            node.shutdown().await?;
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_request_with_no_field_succeeds_and_changes_nothing() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let node = test_node(dir.path()).await?;
        set_every_setting_to_non_default(&node).await?;

        set_sync_settings(&node, |_| {}).await?;

        assert_settings(&node, dir.path(), &non_default_settings(), "empty request").await?;
        node.shutdown().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn requests_for_different_fields_keep_both_changes_in_either_order() -> Result<()> {
        let requests: [fn(&mut SetSyncSettingsRequest); 2] = [
            |request| request.accept_sync = Some(true),
            |request| request.auto_share_sync = Some(true),
        ];
        let expected = SyncSettings {
            accept_sync: true,
            auto_share_sync: true,
            ..non_default_settings()
        };

        for (order, [first, second]) in [
            ("accept_sync first", [requests[0], requests[1]]),
            ("auto_share_sync first", [requests[1], requests[0]]),
        ] {
            let dir = tempfile::tempdir()?;
            let node = test_node(dir.path()).await?;
            set_every_setting_to_non_default(&node).await?;

            set_sync_settings(&node, first).await?;
            set_sync_settings(&node, second).await?;

            assert_settings(&node, dir.path(), &expected, order).await?;
            node.shutdown().await?;
        }
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
