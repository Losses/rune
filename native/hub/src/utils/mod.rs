use database::sync::LibrarySyncStore;
use tokio::{
    runtime::Handle,
    time::{Duration, timeout},
};
use uuid::Uuid;
pub mod broadcastable;
pub mod diagnostics;
pub mod nid;
pub mod player;
pub mod sync_settings;

use database::actions::file::{
    AudioAvailability, audio_availability, audio_origin, remote_content_ids,
};
use std::{
    any::TypeId,
    collections::HashMap,
    fmt::Debug,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    result::Result as StdResult,
    sync::{Arc, OnceLock},
};

use crate::handlers::local_libraries::publish_library_records_changed;
use crate::node::{NodeContext, NodeSlot};
use anyhow::{Context, Result};
use fsio::{FileIoError, FsIo, OpenDalConfig};
use log::info;
use scrobbling::manager::ScrobblingServiceManager;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use ::database::{
    actions::{
        cover_art::bake_cover_art_by_media_files, metadata::MetadataSummary,
        mixes::query_mix_media_files,
    },
    connection::{
        LibraryState, MainDbConnection, RecommendationDbConnection, check_library_state,
        check_storage_mode, connect_main_db, connect_main_db_redirected, connect_recommendation_db,
        connect_recommendation_db_redirected, create_redirect,
    },
    credentials::{Credential, CredentialRef, CredentialStore, Secret},
    entities::media_files,
    library_registry::{
        CatalogPlacement, FsAdapter, InvalidRecord, LibraryRecord, LibraryRecordId,
        LibraryRegistry, ProtocolAdapter, RuneForwardMode, SubsonicOpenMode, resolve_library,
    },
    playing_item::MediaFileHandle,
};
use ::discovery::{client::CertValidator, protocol::DiscoveryService, server::PermissionManager};
use ::playback::{
    player::{Playable, PlaybackSource, PlayingItem},
    sfx_player::SfxPlayer,
};
use ::scrobbling::manager::ScrobblingManager;

use crate::Signal;
use crate::backends::{
    local::local_player_loop,
    remote::{RemoteConnectError, open_remote_library_for_fingerprint},
};
use crate::library_pool::{LibraryHandle, LibraryPool};
use crate::macros::NodeRequestHandlers;
use crate::messages::*;
use crate::request_error::ExpectedRequestError;
use crate::server::ServerManager;
use crate::subsonic_binding::SubsonicBinding;
use subsonic::{SubsonicClient, SubsonicCredentials, SubsonicError, SubsonicServerError};
use url::Url;

#[cfg(target_os = "android")]
use tracing_logcat::{LogcatMakeWriter, LogcatTag};
#[cfg(not(target_os = "android"))]
use tracing_subscriber::EnvFilter;
#[cfg(target_os = "android")]
use tracing_subscriber::fmt::format::Format;

pub struct DatabaseConnections {
    pub main_db: Arc<MainDbConnection>,
    pub recommend_db: Arc<RecommendationDbConnection>,
}

pub async fn initialize_databases(
    fsio: &FsIo,
    path: &str,
    db_path: Option<&str>,
    node_id: &str,
) -> Result<DatabaseConnections> {
    info!("Initializing databases");

    let main_db = connect_main_db(fsio, path, db_path, node_id)
        .await
        .with_context(|| "Failed to connect to main DB")?;

    let recommend_db = connect_recommendation_db(fsio, path, db_path, &main_db)
        .await
        .with_context(|| "Failed to connect to recommendation DB")?;

    Ok(DatabaseConnections {
        main_db: Arc::new(main_db),
        recommend_db: Arc::new(recommend_db),
    })
}

pub async fn initialize_opendal_databases(
    db_dir: &Path,
    storage_key: Uuid,
    node_id: &str,
) -> Result<DatabaseConnections> {
    let db_dir = db_dir.join(storage_key.to_string());
    let main_db = connect_main_db_redirected(&db_dir, storage_key, node_id)
        .await
        .context("Failed to connect to main DB")?;
    let recommend_db = connect_recommendation_db_redirected(&db_dir, &main_db)
        .await
        .context("Failed to connect to recommendation DB")?;
    Ok(DatabaseConnections {
        main_db: Arc::new(main_db),
        recommend_db: Arc::new(recommend_db),
    })
}

#[derive(Debug, Default)]
pub struct TaskTokens {
    pub scan_token: Option<CancellationToken>,
    pub scan_id: Option<String>,
    pub scan_record_id: Option<String>,
    pub analyze_token: Option<CancellationToken>,
    pub analyze_record_id: Option<String>,
    pub deduplicate_token: Option<CancellationToken>,
    pub deduplicate_record_id: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum RunningMode {
    Server,
    Client,
}

/// The current UI library for a request, or the expected `Unavailable` error
/// when this process has none open (for example in remote-control mode).
pub fn require_library(library: Option<LibraryHandle>) -> Result<LibraryHandle> {
    library.ok_or_else(|| {
        ExpectedRequestError::Unavailable {
            what: "no library is open on this device",
        }
        .into()
    })
}

#[derive(Clone)]
pub struct GlobalParams {
    pub node: Arc<NodeContext>,
    pub library_id: Option<Uuid>,
    pub library_pool: Arc<LibraryPool>,
    /// The library open in this UI, the one source of per-library state for
    /// request handlers. `None` in remote-control mode.
    pub library: Option<LibraryHandle>,
    pub ui_library: Arc<Mutex<Option<LibraryHandle>>>,
    pub config_path: Arc<String>,
    pub sync_settings: Arc<RwLock<sync_settings::SyncSettings>>,
    pub node_id: Arc<String>,
    pub main_token: Arc<CancellationToken>,
    pub task_tokens: Arc<Mutex<TaskTokens>>,
    pub player: Arc<Mutex<dyn Playable>>,
    pub sfx_player: Arc<Mutex<SfxPlayer>>,
    pub scrobbler: Arc<Mutex<dyn ScrobblingServiceManager>>,
    pub broadcaster: Arc<dyn Broadcaster>,
    pub device_scanner: Arc<DiscoveryService>,
    pub cert_validator: Arc<RwLock<CertValidator>>,
    pub permission_manager: Arc<RwLock<PermissionManager>>,
    pub server_manager: OnceLock<Arc<ServerManager>>,
    pub running_mode: RunningMode,
}

impl GlobalParams {
    /// Construct request parameters scoped to the addressed library.
    pub fn for_library(&self, library: LibraryHandle) -> Self {
        let library_id = library.library_id;
        let mut params = self.clone();
        params.library_id = Some(library_id);
        params.library = Some(library);
        params
    }
}

/// The application owns the replicated catalog scope. Pairing authorization
/// determines which devices may exchange it; this UUID is not an access token.
pub async fn initialize_library_replica(
    db: &MainDbConnection,
    device_id: &str,
    library_id: Uuid,
) -> Result<LibrarySyncStore> {
    let device_id = Uuid::parse_str(device_id)?;
    let library = LibrarySyncStore::open(db.clone(), device_id, Some(library_id)).await?;
    library.import_existing(device_id).await?;
    Ok(library)
}

#[cfg(test)]
impl GlobalParams {
    /// The library a test fixture opened. Fixtures that build `GlobalParams`
    /// for a local library always set it, so a missing one is a broken fixture.
    pub fn test_library(&self) -> &LibraryHandle {
        self.library
            .as_ref()
            .expect("test fixture opened no library")
    }
}

impl Debug for GlobalParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlobalParams")
            .field("library_id", &self.library_id)
            .finish()
    }
}

pub trait ParamsExtractor {
    type Params;
    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params;
}

pub trait RinfRustSignal: 'static {
    fn name(&self) -> String;

    fn remote_event(&self) -> Option<RemoteEventScope> {
        let type_id = TypeId::of::<Self>();
        broadcastable::REMOTE_EVENT_DECLARATIONS
            .iter()
            .find(|declaration| declaration.type_id == type_id)
            .map(|declaration| declaration.scope)
    }

    /// Encodes the message into a byte vector.
    fn encode_to_vec(&self) -> Result<Vec<u8>>;

    /// Sends the message to Dart.
    fn send_to_dart(&self);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteEventScope {
    Session,
    Library,
}

pub struct RemoteEventDeclaration {
    pub name: &'static str,
    pub type_id: TypeId,
    pub scope: RemoteEventScope,
    pub decode: fn(&[u8]) -> Result<Box<dyn RinfRustSignal>>,
}

pub trait Broadcaster: Send + Sync {
    fn broadcast(&self, message: &dyn RinfRustSignal);
}

pub struct LocalGuiBroadcaster;

impl Broadcaster for LocalGuiBroadcaster {
    fn broadcast(&self, message: &dyn RinfRustSignal) {
        message.send_to_dart();
    }
}

pub(crate) struct ProductionNodeRequestHandlers {
    scrobbler: Arc<Mutex<ScrobblingManager>>,
}

impl ProductionNodeRequestHandlers {
    pub(crate) fn new(scrobbler: Arc<Mutex<ScrobblingManager>>) -> Self {
        Self { scrobbler }
    }
}

// Explicit `Send` futures let the generated receiver loop run in a Tokio task.
#[allow(clippy::manual_async_fn)]
impl NodeRequestHandlers for ProductionNodeRequestHandlers {
    fn handle_list_controllers_request(
        &self,
        node: &NodeSlot,
        request: &ListControllersRequest,
    ) -> impl Future<Output = Result<Option<ListControllersResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request.handle(node, None, request).await
        }
    }

    fn handle_list_local_libraries_request(
        &self,
        node: &NodeSlot,
        request: &ListLocalLibrariesRequest,
    ) -> impl Future<Output = Result<Option<ListLocalLibrariesResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request
                .handle(node.config_path.clone(), None, request)
                .await
        }
    }

    fn handle_get_device_grants_request(
        &self,
        node: &NodeSlot,
        request: &GetDeviceGrantsRequest,
    ) -> impl Future<Output = Result<Option<GetDeviceGrantsResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request.handle(node, None, request).await
        }
    }

    fn handle_set_device_grants_request(
        &self,
        node: &NodeSlot,
        request: &SetDeviceGrantsRequest,
    ) -> impl Future<Output = Result<Option<SetDeviceGrantsResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request.handle(node, None, request).await
        }
    }

    fn handle_set_library_background_sync_request(
        &self,
        node: &NodeSlot,
        request: &SetLibraryBackgroundSyncRequest,
    ) -> impl Future<Output = Result<Option<SetLibraryBackgroundSyncResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request.handle(node, None, request).await
        }
    }

    fn handle_list_peer_libraries_request(
        &self,
        node: &NodeSlot,
        request: &ListPeerLibrariesRequest,
    ) -> impl Future<Output = Result<Option<ListPeerLibrariesResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request
                .handle(node.server_manager.get().cloned(), None, request)
                .await
        }
    }

    fn handle_add_remote_library_request(
        &self,
        node: &NodeSlot,
        request: &AddRemoteLibraryRequest,
    ) -> impl Future<Output = Result<Option<AddRemoteLibraryResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request
                .handle(node.config_path.clone(), None, request)
                .await
        }
    }

    fn handle_register_license_request(
        &self,
        node: &NodeSlot,
        request: &RegisterLicenseRequest,
    ) -> impl Future<Output = Result<Option<RegisterLicenseResponse>>> + Send {
        async move {
            node.current().await?;
            request.handle((), None, request).await
        }
    }

    fn handle_validate_license_request(
        &self,
        node: &NodeSlot,
        request: &ValidateLicenseRequest,
    ) -> impl Future<Output = Result<Option<ValidateLicenseResponse>>> + Send {
        async move {
            node.current().await?;
            request.handle((), None, request).await
        }
    }

    fn handle_get_discovered_device_request(
        &self,
        node: &NodeSlot,
        request: &GetDiscoveredDeviceRequest,
    ) -> impl Future<Output = Result<Option<GetDiscoveredDeviceResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request
                .handle(Arc::clone(&node.device_scanner), None, request)
                .await
        }
    }

    fn handle_list_clients_request(
        &self,
        node: &NodeSlot,
        request: &ListClientsRequest,
    ) -> impl Future<Output = Result<Option<ListClientsResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request
                .handle(Arc::clone(&node.permission_manager), None, request)
                .await
        }
    }

    fn handle_start_server_request(
        &self,
        node: &NodeSlot,
        request: &StartServerRequest,
    ) -> impl Future<Output = Result<Option<StartServerResponse>>> + Send {
        async move {
            let node = node.current().await?;
            let manager = node
                .server_manager
                .get()
                .cloned()
                .context("Server manager is not initialized")?;
            request
                .handle((node.config_path.clone(), manager), None, request)
                .await
        }
    }

    fn handle_stop_server_request(
        &self,
        node: &NodeSlot,
        request: &StopServerRequest,
    ) -> impl Future<Output = Result<Option<StopServerResponse>>> + Send {
        async move {
            let node = node.current().await?;
            let manager = node
                .server_manager
                .get()
                .cloned()
                .context("Server manager is not initialized")?;
            request.handle(manager, None, request).await
        }
    }

    fn handle_start_broadcast_request(
        &self,
        node: &NodeSlot,
        request: &StartBroadcastRequest,
    ) -> impl Future<Output = Result<Option<()>>> + Send {
        async move {
            let node = node.current().await?;
            request
                .handle(
                    (Arc::clone(&node.device_scanner), node.config_path.clone()),
                    None,
                    request,
                )
                .await
        }
    }

    fn handle_stop_broadcast_request(
        &self,
        node: &NodeSlot,
        request: &StopBroadcastRequest,
    ) -> impl Future<Output = Result<Option<()>>> + Send {
        async move {
            let node = node.current().await?;
            request
                .handle((Arc::clone(&node.device_scanner),), None, request)
                .await
        }
    }

    fn handle_start_listening_request(
        &self,
        node: &NodeSlot,
        request: &StartListeningRequest,
    ) -> impl Future<Output = Result<Option<()>>> + Send {
        async move {
            let node = node.current().await?;
            request
                .handle(
                    (Arc::clone(&node.device_scanner), node.config_path.clone()),
                    None,
                    request,
                )
                .await
        }
    }

    fn handle_stop_listening_request(
        &self,
        node: &NodeSlot,
        request: &StopListeningRequest,
    ) -> impl Future<Output = Result<Option<()>>> + Send {
        async move {
            let node = node.current().await?;
            request
                .handle((Arc::clone(&node.device_scanner),), None, request)
                .await
        }
    }

    fn handle_set_visualizer_preset_dirs_request(
        &self,
        node: &NodeSlot,
        request: &SetVisualizerPresetDirsRequest,
    ) -> impl Future<Output = Result<Option<()>>> + Send {
        async move {
            node.current().await?;
            request.handle((), None, request).await
        }
    }

    fn handle_set_visualizer_scenes_request(
        &self,
        node: &NodeSlot,
        request: &SetVisualizerScenesRequest,
    ) -> impl Future<Output = Result<Option<()>>> + Send {
        async move {
            node.current().await?;
            request.handle((), None, request).await
        }
    }

    fn handle_get_ssl_certificate_fingerprint_request(
        &self,
        node: &NodeSlot,
        request: &GetSslCertificateFingerprintRequest,
    ) -> impl Future<Output = Result<Option<GetSslCertificateFingerprintResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request
                .handle(node.config_path.clone(), None, request)
                .await
        }
    }

    fn handle_visualizer_support_request(
        &self,
        node: &NodeSlot,
        request: &VisualizerSupportRequest,
    ) -> impl Future<Output = Result<Option<VisualizerSupportResponse>>> + Send {
        async move {
            node.current().await?;
            request.handle((), None, request).await
        }
    }

    fn handle_list_visualizer_presets_request(
        &self,
        node: &NodeSlot,
        request: &ListVisualizerPresetsRequest,
    ) -> impl Future<Output = Result<Option<ListVisualizerPresetsResponse>>> + Send {
        async move {
            node.current().await?;
            request.handle((), None, request).await
        }
    }

    fn handle_test_library_initialized_request(
        &self,
        node: &NodeSlot,
        request: &TestLibraryInitializedRequest,
    ) -> impl Future<Output = Result<Option<TestLibraryInitializedResponse>>> + Send {
        async move {
            node.current().await?;
            request.handle((), None, request).await
        }
    }

    fn handle_set_mix_with_others_request(
        &self,
        node: &NodeSlot,
        request: &SetMixWithOthersRequest,
    ) -> impl Future<Output = Result<Option<()>>> + Send {
        async move {
            let node = node.current().await?;
            let session = node.session()?;
            request
                .handle((Arc::clone(&session.player),), None, request)
                .await
        }
    }

    fn handle_get_sync_settings_request(
        &self,
        node: &NodeSlot,
        request: &GetSyncSettingsRequest,
    ) -> impl Future<Output = Result<Option<GetSyncSettingsResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request
                .handle(Arc::clone(&node.sync_settings), None, request)
                .await
        }
    }

    fn handle_set_sync_settings_request(
        &self,
        node: &NodeSlot,
        request: &SetSyncSettingsRequest,
    ) -> impl Future<Output = Result<Option<SetSyncSettingsResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request
                .handle(
                    (
                        Arc::clone(&node.sync_settings),
                        node.config_path.clone(),
                        node.server_manager.get().cloned(),
                    ),
                    None,
                    request,
                )
                .await
        }
    }

    fn handle_system_info_request(
        &self,
        node: &NodeSlot,
        request: &SystemInfoRequest,
    ) -> impl Future<Output = Result<Option<SystemInfoResponse>>> + Send {
        async move {
            node.current().await?;
            request.handle((), None, request).await
        }
    }

    fn handle_add_trusted_server_request(
        &self,
        node: &NodeSlot,
        request: &AddTrustedServerRequest,
    ) -> impl Future<Output = Result<Option<AddTrustedServerResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request
                .handle(Arc::clone(&node.cert_validator), None, request)
                .await
        }
    }

    fn handle_remove_trusted_client_request(
        &self,
        node: &NodeSlot,
        request: &RemoveTrustedClientRequest,
    ) -> impl Future<Output = Result<Option<RemoveTrustedClientResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request.handle(node, None, request).await
        }
    }

    fn handle_update_client_status_request(
        &self,
        node: &NodeSlot,
        request: &UpdateClientStatusRequest,
    ) -> impl Future<Output = Result<Option<UpdateClientStatusResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request.handle(node, None, request).await
        }
    }

    fn handle_remove_trusted_server_request(
        &self,
        node: &NodeSlot,
        request: &RemoveTrustedServerRequest,
    ) -> impl Future<Output = Result<Option<RemoveTrustedServerResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request.handle(node, None, request).await
        }
    }

    fn handle_server_availability_test_request(
        &self,
        node: &NodeSlot,
        request: &ServerAvailabilityTestRequest,
    ) -> impl Future<Output = Result<Option<ServerAvailabilityTestResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request
                .handle(Arc::clone(&node.cert_validator), None, request)
                .await
        }
    }

    fn handle_register_device_on_server_request(
        &self,
        node: &NodeSlot,
        request: &RegisterDeviceOnServerRequest,
    ) -> impl Future<Output = Result<Option<RegisterDeviceOnServerResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request
                .handle(
                    (node.config_path.clone(), Arc::clone(&node.cert_validator)),
                    None,
                    request,
                )
                .await
        }
    }

    fn handle_check_device_on_server_request(
        &self,
        node: &NodeSlot,
        request: &CheckDeviceOnServerRequest,
    ) -> impl Future<Output = Result<Option<CheckDeviceOnServerResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request
                .handle(
                    (Arc::clone(&node.cert_validator), node.config_path.clone()),
                    None,
                    request,
                )
                .await
        }
    }

    fn handle_connect_request(
        &self,
        node: &NodeSlot,
        request: &ConnectRequest,
    ) -> impl Future<Output = Result<Option<ConnectResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request
                .handle(Arc::clone(&node.cert_validator), None, request)
                .await
        }
    }

    fn handle_fetch_server_certificate_request(
        &self,
        node: &NodeSlot,
        request: &FetchServerCertificateRequest,
    ) -> impl Future<Output = Result<Option<FetchServerCertificateResponse>>> + Send {
        async move {
            node.current().await?;
            request.handle((), None, request).await
        }
    }

    fn handle_list_library_records_request(
        &self,
        node: &NodeSlot,
        request: &ListLibraryRecordsRequest,
    ) -> impl Future<Output = Result<Option<ListLibraryRecordsResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request.handle(node, None, request).await
        }
    }

    fn handle_remove_library_record_request(
        &self,
        node: &NodeSlot,
        request: &RemoveLibraryRecordRequest,
    ) -> impl Future<Output = Result<Option<RemoveLibraryRecordResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request.handle(node, None, request).await
        }
    }

    fn handle_register_local_library_request(
        &self,
        node: &NodeSlot,
        request: &RegisterLocalLibraryRequest,
    ) -> impl Future<Output = Result<Option<RegisterLibraryResponse>>> + Send {
        async move { Ok(Some(register_local_library(node, request).await)) }
    }

    fn handle_register_remote_control_request(
        &self,
        node: &NodeSlot,
        request: &RegisterRemoteControlRequest,
    ) -> impl Future<Output = Result<Option<RegisterLibraryResponse>>> + Send {
        async move { Ok(Some(register_remote_control(node, request).await)) }
    }

    fn handle_register_subsonic_library_request(
        &self,
        node: &NodeSlot,
        request: &RegisterSubsonicLibraryRequest,
    ) -> impl Future<Output = Result<Option<RegisterLibraryResponse>>> + Send {
        async move { Ok(Some(register_subsonic_library(node, request).await)) }
    }

    fn handle_register_open_dal_library_request(
        &self,
        node: &NodeSlot,
        request: &RegisterOpenDalLibraryRequest,
    ) -> impl Future<Output = Result<Option<RegisterLibraryResponse>>> + Send {
        async move { Ok(Some(register_opendal_library(node, request).await)) }
    }

    fn handle_open_library_request(
        &self,
        node: &NodeSlot,
        request: &OpenLibraryRequest,
    ) -> impl Future<Output = Result<Option<OpenLibraryResponse>>> + Send {
        let scrobbler = Arc::clone(&self.scrobbler);
        async move { Ok(Some(open_library_by_record(node, request, scrobbler).await)) }
    }
}

async fn register_opendal_library(
    node_slot: &NodeSlot,
    request: &RegisterOpenDalLibraryRequest,
) -> RegisterLibraryResponse {
    match node_slot.current().await {
        Ok(node) => register_opendal_library_on_node(request, &node).await,
        Err(_) => opendal_registration_failure(
            RegisterLibraryFailureKind::Storage,
            "Could not access library storage",
        ),
    }
}

pub(crate) async fn register_opendal_library_on_node(
    request: &RegisterOpenDalLibraryRequest,
    node: &Arc<NodeContext>,
) -> RegisterLibraryResponse {
    let (config, username) = match &request.service {
        OpenDalLibraryService::WebDav {
            endpoint,
            root,
            username,
            password,
        } => (
            OpenDalConfig::WebDav {
                endpoint: endpoint.clone(),
                root: root.clone(),
                username: username.clone(),
                password: password.clone(),
            },
            username.clone(),
        ),
        OpenDalLibraryService::S3 {
            endpoint,
            bucket,
            region,
            root,
            access_key_id,
            secret_access_key,
        } => (
            OpenDalConfig::S3 {
                endpoint: endpoint.clone(),
                bucket: bucket.clone(),
                region: region.clone(),
                root: root.clone(),
                access_key_id: access_key_id.clone(),
                secret_access_key: secret_access_key.clone(),
            },
            access_key_id.clone(),
        ),
    };
    let is_webdav = matches!(&request.service, OpenDalLibraryService::WebDav { .. });
    let handle = Handle::current();
    let probe = tokio::task::spawn_blocking(move || {
        catch_unwind(AssertUnwindSafe(|| -> Result<usize, FileIoError> {
            let fsio = FsIo::new_opendal(config, handle)?;
            let nodes = fsio.walk_dir(Path::new(""), false)?;
            match fsio.canonicalize(Path::new("")) {
                Ok(root) if root.is_dir => {}
                Ok(_) => {
                    return Err(FileIoError::NotSupported(
                        "remote root is not a folder".into(),
                    ));
                }
                Err(FileIoError::PathNotFound(_)) if !is_webdav => {}
                Err(error) => return Err(error),
            }
            Ok(nodes.len())
        }))
        .unwrap_or_else(|_| {
            Err(FileIoError::NotSupported(
                "remote root is not a folder".into(),
            ))
        })
    })
    .await;
    let count = match probe {
        Err(_) => {
            return opendal_registration_failure(
                RegisterLibraryFailureKind::OpenDalUnreachable,
                "Could not reach the remote library",
            );
        }
        Ok(Err(FileIoError::PermissionDenied(_))) => {
            return opendal_registration_failure(
                RegisterLibraryFailureKind::OpenDalAuthenticationRejected,
                "The remote credentials were rejected",
            );
        }
        Ok(Err(FileIoError::Remote(_))) => {
            return opendal_registration_failure(
                RegisterLibraryFailureKind::OpenDalUnreachable,
                "Could not reach the remote library",
            );
        }
        Ok(Err(_)) => {
            return opendal_registration_failure(
                RegisterLibraryFailureKind::OpenDalNotFolder,
                "The selected remote root is not a folder",
            );
        }
        Ok(Ok(count)) => count,
    };
    if is_webdav && count == 0 {
        return opendal_registration_failure(
            RegisterLibraryFailureKind::OpenDalNotFolder,
            "The selected remote root is not a folder",
        );
    }
    let config_dir = Path::new(&*node.config_path);
    let existing = match LibraryRegistry::load(config_dir) {
        Ok(registry) => registry.libraries.into_iter().find(|record| {
            match (&request.service, record.fs.first()) {
                (
                    OpenDalLibraryService::WebDav {
                        endpoint,
                        root,
                        username,
                        ..
                    },
                    Some(FsAdapter::WebDav {
                        endpoint: e,
                        root: r,
                        username: u,
                        ..
                    }),
                ) => endpoint == e && root == r && username == u,
                (
                    OpenDalLibraryService::S3 {
                        endpoint,
                        bucket,
                        root,
                        access_key_id,
                        ..
                    },
                    Some(FsAdapter::S3 {
                        endpoint: e,
                        bucket: b,
                        root: r,
                        access_key_id: k,
                        ..
                    }),
                ) => endpoint == e && bucket == b && root == r && access_key_id == k,
                _ => false,
            }
        }),
        Err(_) => {
            return opendal_registration_failure(
                RegisterLibraryFailureKind::Storage,
                "Could not read library storage",
            );
        }
    };
    if let Some(record) = existing {
        return RegisterLibraryResponse {
            call_id: None,
            record: Some((&record).into()),
            outcome: RegisterLibraryOutcome::Registered,
            detail: None,
        };
    }
    let secret = match &request.service {
        OpenDalLibraryService::WebDav { password, .. } => password.as_bytes(),
        OpenDalLibraryService::S3 {
            secret_access_key, ..
        } => secret_access_key.as_bytes(),
    };
    let credential = match node
        .credentials
        .put(Credential {
            username,
            secret: Secret::new(secret.to_vec()),
        })
        .await
    {
        Ok(reference) => reference,
        Err(_) => {
            return opendal_registration_failure(
                RegisterLibraryFailureKind::Storage,
                "Could not store credentials",
            );
        }
    };
    let storage_key = Uuid::new_v4();
    let record = LibraryRecord {
        record_id: LibraryRecordId(Uuid::new_v4()),
        alias: request.alias.clone(),
        library_id: Some(Uuid::new_v4()),
        protocol: ProtocolAdapter::Native,
        fs: vec![match &request.service {
            OpenDalLibraryService::WebDav {
                endpoint,
                root,
                username,
                ..
            } => FsAdapter::WebDav {
                endpoint: endpoint.clone(),
                root: root.clone(),
                username: username.clone(),
                credential,
            },
            OpenDalLibraryService::S3 {
                endpoint,
                bucket,
                region,
                root,
                access_key_id,
                ..
            } => FsAdapter::S3 {
                endpoint: endpoint.clone(),
                bucket: bucket.clone(),
                region: region.clone(),
                root: root.clone(),
                access_key_id: access_key_id.clone(),
                credential,
            },
        }],
        catalog: CatalogPlacement::Redirected(storage_key),
        background_sync: true,
    };
    let stored = if record.validate().is_err() {
        Err(anyhow::anyhow!("invalid record"))
    } else {
        LibraryRegistry::update(config_dir, |registry| {
            registry.libraries.push(record.clone());
            Ok(record.clone())
        })
        .await
    };
    let record = match stored {
        Ok(record) => record,
        Err(_) => {
            let _ = node.credentials.remove(credential).await;
            return opendal_registration_failure(
                RegisterLibraryFailureKind::Storage,
                "Could not write library storage",
            );
        }
    };
    if let Err(error) = publish_library_records_changed(node).await {
        log::warn!("Could not publish library record change: {error:#}");
    }
    RegisterLibraryResponse {
        call_id: None,
        record: Some((&record).into()),
        outcome: RegisterLibraryOutcome::Registered,
        detail: None,
    }
}

fn opendal_registration_failure(
    kind: RegisterLibraryFailureKind,
    detail: &str,
) -> RegisterLibraryResponse {
    RegisterLibraryResponse {
        call_id: None,
        record: None,
        outcome: RegisterLibraryOutcome::Failed(kind),
        detail: Some(detail.into()),
    }
}

async fn register_local_library(
    node_slot: &NodeSlot,
    request: &RegisterLocalLibraryRequest,
) -> RegisterLibraryResponse {
    let node = match node_slot.current().await {
        Ok(node) => node,
        Err(error) => {
            return RegisterLibraryResponse {
                call_id: None,
                record: None,
                outcome: RegisterLibraryOutcome::Failed(RegisterLibraryFailureKind::Storage),
                detail: Some(format!("{error:#}")),
            };
        }
    };
    register_local_library_on_node(request, &node).await
}

pub(crate) async fn register_local_library_on_node(
    request: &RegisterLocalLibraryRequest,
    node: &Arc<NodeContext>,
) -> RegisterLibraryResponse {
    let fail = |kind, error: anyhow::Error| RegisterLibraryResponse {
        call_id: None,
        record: None,
        outcome: RegisterLibraryOutcome::Failed(kind),
        detail: Some(format!("{error:#}")),
    };
    let config = node.config_path.clone();
    let path = &request.path;
    #[cfg(not(target_os = "android"))]
    let fsio = FsIo::new();
    #[cfg(target_os = "android")]
    let fsio = match FsIo::new(Path::new(".rune/.android-fs.db"), path) {
        Ok(fsio) => fsio,
        Err(error) => return fail(RegisterLibraryFailureKind::Storage, error.into()),
    };
    let state = match check_library_state(&fsio, path) {
        Ok(state) => state,
        Err(error) => return fail(RegisterLibraryFailureKind::InvalidSource, error),
    };
    if matches!(state, LibraryState::Uninitialized) && request.mode.is_none() {
        return RegisterLibraryResponse {
            call_id: None,
            record: None,
            outcome: RegisterLibraryOutcome::NeedsInitializeMode,
            detail: None,
        };
    }
    let result = async {
        if request.mode == Some(LibraryInitializeMode::Redirected) {
            create_redirect(&fsio, path).await?;
        }
        let mode = check_storage_mode(&fsio, path)?;
        let mut registry = LibraryRegistry::load(Path::new(&*config))?;
        resolve_library(&fsio, &mut registry, path, &request.alias, mode).await
    }
    .await;
    match result {
        Ok(record) => {
            if let Err(error) = publish_library_records_changed(node).await {
                log::warn!("Could not publish library record change: {error:#}");
            }
            RegisterLibraryResponse {
                call_id: None,
                record: Some((&record).into()),
                outcome: RegisterLibraryOutcome::Registered,
                detail: None,
            }
        }
        Err(error) if error.downcast_ref::<InvalidRecord>().is_some() => {
            fail(RegisterLibraryFailureKind::InvalidRecord, error)
        }
        Err(error) => fail(RegisterLibraryFailureKind::Storage, error),
    }
}

async fn register_remote_control(
    node_slot: &NodeSlot,
    request: &RegisterRemoteControlRequest,
) -> RegisterLibraryResponse {
    let node = match node_slot.current().await {
        Ok(node) => node,
        Err(error) => {
            return RegisterLibraryResponse {
                call_id: None,
                record: None,
                outcome: RegisterLibraryOutcome::Failed(RegisterLibraryFailureKind::Storage),
                detail: Some(format!("{error:#}")),
            };
        }
    };
    register_remote_control_on_node(request, &node).await
}

pub(crate) async fn register_remote_control_on_node(
    request: &RegisterRemoteControlRequest,
    node: &Arc<NodeContext>,
) -> RegisterLibraryResponse {
    let hosts = node
        .cert_validator
        .read()
        .await
        .get_hosts_for_fingerprint(&request.fingerprint)
        .await;
    if hosts.is_empty() {
        return RegisterLibraryResponse {
            call_id: None,
            record: None,
            outcome: RegisterLibraryOutcome::Failed(RegisterLibraryFailureKind::Untrusted),
            detail: Some("Fingerprint is not trusted".into()),
        };
    }
    match LibraryRegistry::register_remote_control(
        Path::new(&*node.config_path),
        &request.fingerprint,
        &request.alias,
    )
    .await
    {
        Ok(record) => {
            if let Err(error) = publish_library_records_changed(node).await {
                log::warn!("Could not publish library record change: {error:#}");
            }
            RegisterLibraryResponse {
                call_id: None,
                record: Some((&record).into()),
                outcome: RegisterLibraryOutcome::Registered,
                detail: None,
            }
        }
        Err(error) if error.downcast_ref::<InvalidRecord>().is_some() => RegisterLibraryResponse {
            call_id: None,
            record: None,
            outcome: RegisterLibraryOutcome::Failed(RegisterLibraryFailureKind::InvalidRecord),
            detail: Some(format!("{error:#}")),
        },
        Err(error) => RegisterLibraryResponse {
            call_id: None,
            record: None,
            outcome: RegisterLibraryOutcome::Failed(RegisterLibraryFailureKind::Storage),
            detail: Some(format!("{error:#}")),
        },
    }
}

async fn register_subsonic_library(
    node_slot: &NodeSlot,
    request: &RegisterSubsonicLibraryRequest,
) -> RegisterLibraryResponse {
    match node_slot.current().await {
        Ok(node) => register_subsonic_library_with_node(request, &node).await,
        Err(_) => subsonic_registration_failure(
            RegisterLibraryFailureKind::Storage,
            "Could not access library storage",
        ),
    }
}

pub(crate) async fn register_subsonic_library_with_node(
    request: &RegisterSubsonicLibraryRequest,
    node: &Arc<NodeContext>,
) -> RegisterLibraryResponse {
    register_subsonic_library_on_node(request, node).await
}

pub(crate) async fn register_subsonic_library_on_node(
    request: &RegisterSubsonicLibraryRequest,
    node: &Arc<NodeContext>,
) -> RegisterLibraryResponse {
    let endpoint = match validate_subsonic_endpoint(&request.endpoint) {
        Ok(endpoint) => endpoint,
        Err(()) => {
            return subsonic_registration_failure(
                RegisterLibraryFailureKind::SubsonicNotSubsonic,
                "The endpoint must be a valid HTTP or HTTPS URL",
            );
        }
    };
    let http = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(http) => http,
        Err(_) => {
            return subsonic_registration_failure(
                RegisterLibraryFailureKind::Storage,
                "Could not create the Subsonic client",
            );
        }
    };
    let client = SubsonicClient::new(
        endpoint.clone(),
        SubsonicCredentials::Password {
            user: request.username.clone(),
            password: request.password.clone(),
        },
        http,
    );
    let probe = match tokio::time::timeout(Duration::from_secs(5), client.probe()).await {
        Err(_) => {
            return subsonic_registration_failure(
                RegisterLibraryFailureKind::SubsonicUnreachable,
                "The Subsonic server did not respond in time",
            );
        }
        Ok(Err(error)) => return subsonic_probe_failure(error),
        Ok(Ok(probe)) => probe,
    };
    let allowed = if probe.roles_known {
        let mut allowed = Vec::with_capacity(2);
        if probe.roles.stream_role {
            allowed.push(SubsonicOpenMode::PlayOnThisDevice);
        }
        if probe.roles.jukebox_role {
            allowed.push(SubsonicOpenMode::RemoteControl);
        }
        allowed
    } else {
        vec![SubsonicOpenMode::PlayOnThisDevice]
    };
    if allowed.is_empty() {
        return subsonic_registration_failure(
            RegisterLibraryFailureKind::SubsonicRoleMissing,
            "The account has no supported Subsonic role",
        );
    }
    let mode = match request.mode.map(SubsonicOpenMode::from) {
        Some(mode) if allowed.contains(&mode) => mode,
        Some(_) => {
            return subsonic_registration_failure(
                RegisterLibraryFailureKind::SubsonicRoleMissing,
                "The account does not have the role required by that mode",
            );
        }
        None if allowed.len() == 1 => allowed[0],
        None => {
            return RegisterLibraryResponse {
                call_id: None,
                record: None,
                outcome: RegisterLibraryOutcome::NeedsSubsonicMode {
                    allowed: allowed.into_iter().map(Into::into).collect(),
                },
                detail: None,
            };
        }
    };

    let config = Path::new(&*node.config_path);
    match existing_subsonic_record(
        config,
        &node.credentials,
        endpoint.as_str(),
        &request.username,
        mode,
    )
    .await
    {
        Ok(Some(record)) => {
            return RegisterLibraryResponse {
                call_id: None,
                record: Some((&record).into()),
                outcome: RegisterLibraryOutcome::Registered,
                detail: None,
            };
        }
        Ok(None) => {}
        Err(_) => {
            return subsonic_registration_failure(
                RegisterLibraryFailureKind::Storage,
                "Could not read library storage",
            );
        }
    }

    let credential = match node
        .credentials
        .put(Credential {
            username: request.username.clone(),
            secret: Secret::new(request.password.as_bytes().to_vec()),
        })
        .await
    {
        Ok(credential) => credential,
        Err(_) => {
            return subsonic_registration_failure(
                RegisterLibraryFailureKind::Storage,
                "Could not store credentials",
            );
        }
    };
    let record = LibraryRecord {
        record_id: LibraryRecordId(Uuid::new_v4()),
        alias: request.alias.clone(),
        library_id: None,
        protocol: ProtocolAdapter::SubsonicTranslate {
            endpoint: endpoint.to_string(),
            credential,
            mode,
        },
        fs: Vec::new(),
        catalog: CatalogPlacement::None,
        background_sync: false,
    };
    if record.validate().is_err() {
        let _ = node.credentials.remove(credential).await;
        return subsonic_registration_failure(
            RegisterLibraryFailureKind::InvalidRecord,
            "The library record is invalid",
        );
    }
    let stored = LibraryRegistry::update(config, |registry| {
        registry.libraries.push(record.clone());
        Ok(record.clone())
    })
    .await;
    let record = match stored {
        Ok(value) => value,
        Err(_) => {
            let _ = node.credentials.remove(credential).await;
            return subsonic_registration_failure(
                RegisterLibraryFailureKind::Storage,
                "Could not write library storage",
            );
        }
    };
    if let Err(error) = publish_library_records_changed(node).await {
        log::warn!("Could not publish library record change: {error:#}");
    }
    RegisterLibraryResponse {
        call_id: None,
        record: Some((&record).into()),
        outcome: RegisterLibraryOutcome::Registered,
        detail: None,
    }
}

fn validate_subsonic_endpoint(raw: &str) -> Result<Url, ()> {
    let endpoint = Url::parse(raw).map_err(|_| ())?;
    if !matches!(endpoint.scheme(), "http" | "https")
        || endpoint.host_str().is_none()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(());
    }
    Ok(endpoint)
}

async fn existing_subsonic_record(
    config: &Path,
    credentials: &CredentialStore,
    endpoint: &str,
    username: &str,
    mode: SubsonicOpenMode,
) -> anyhow::Result<Option<LibraryRecord>> {
    let registry = LibraryRegistry::load(config)?;
    for record in &registry.libraries {
        if let ProtocolAdapter::SubsonicTranslate {
            endpoint: stored_endpoint,
            credential,
            mode: stored_mode,
        } = &record.protocol
            && stored_endpoint == endpoint
            && *stored_mode == mode
            && credentials
                .get(*credential)
                .await
                .is_ok_and(|stored| stored.username == username)
        {
            return Ok(Some(record.clone()));
        }
    }
    Ok(None)
}

fn subsonic_probe_failure(error: SubsonicError) -> RegisterLibraryResponse {
    let kind = match error {
        SubsonicError::Transport(_) | SubsonicError::HttpStatus(_) => {
            RegisterLibraryFailureKind::SubsonicUnreachable
        }
        SubsonicError::Server {
            kind: SubsonicServerError::WrongCredentials | SubsonicServerError::NotAuthorized,
            ..
        } => RegisterLibraryFailureKind::SubsonicWrongCredentials,
        SubsonicError::Server {
            kind: SubsonicServerError::ServerTooOld,
            ..
        } => RegisterLibraryFailureKind::SubsonicServerTooOld,
        SubsonicError::Server {
            kind: SubsonicServerError::ClientTooOld,
            ..
        } => RegisterLibraryFailureKind::SubsonicClientTooOld,
        SubsonicError::Malformed(_)
        | SubsonicError::Server { .. }
        | SubsonicError::JukeboxUnsupported
        | SubsonicError::InvalidJukeboxGain => RegisterLibraryFailureKind::SubsonicNotSubsonic,
    };
    let detail = match kind {
        RegisterLibraryFailureKind::SubsonicUnreachable => "Could not reach the Subsonic server",
        RegisterLibraryFailureKind::SubsonicWrongCredentials => {
            "The Subsonic credentials were rejected"
        }
        RegisterLibraryFailureKind::SubsonicServerTooOld => "The Subsonic server is too old",
        RegisterLibraryFailureKind::SubsonicClientTooOld => {
            "The Subsonic server requires a newer client"
        }
        _ => "The endpoint did not return a valid Subsonic response",
    };
    subsonic_registration_failure(kind, detail)
}

fn subsonic_registration_failure(
    kind: RegisterLibraryFailureKind,
    detail: &'static str,
) -> RegisterLibraryResponse {
    RegisterLibraryResponse {
        call_id: None,
        record: None,
        outcome: RegisterLibraryOutcome::Failed(kind),
        detail: Some(detail.into()),
    }
}

async fn open_library_by_record(
    node_slot: &NodeSlot,
    request: &OpenLibraryRequest,
    scrobbler: Arc<Mutex<ScrobblingManager>>,
) -> OpenLibraryResponse {
    let node = match node_slot.current().await {
        Ok(node) => node,
        Err(error) => {
            return OpenLibraryResponse {
                call_id: request.call_id,
                record_id: request.record_id.clone(),
                generation: 0,
                result: OpenLibraryResult::Failed(OpenLibraryFailure {
                    kind: OpenLibraryFailureKind::Storage,
                    detail: format!("{error:#}"),
                }),
            };
        }
    };
    open_library_by_record_on_node(request, node, scrobbler).await
}

pub(crate) async fn open_library_by_record_on_node(
    request: &OpenLibraryRequest,
    node: Arc<NodeContext>,
    scrobbler: Arc<Mutex<ScrobblingManager>>,
) -> OpenLibraryResponse {
    let failed = |kind, error: anyhow::Error| OpenLibraryResponse {
        call_id: request.call_id,
        record_id: request.record_id.clone(),
        generation: 0,
        result: OpenLibraryResult::Failed(OpenLibraryFailure {
            kind,
            detail: format!("{error:#}"),
        }),
    };
    let record_id = match Uuid::parse_str(&request.record_id) {
        Ok(id) => LibraryRecordId(id),
        Err(error) => return failed(OpenLibraryFailureKind::RecordNotFound, error.into()),
    };
    let record = match LibraryRegistry::load(Path::new(&*node.config_path))
        .and_then(|registry| registry.record(record_id).cloned())
    {
        Ok(record) => record,
        Err(error) if error.downcast_ref::<InvalidRecord>().is_some() => {
            return failed(OpenLibraryFailureKind::InvalidRecord, error);
        }
        Err(error) => return failed(OpenLibraryFailureKind::RecordNotFound, error),
    };
    let result = match &record.protocol {
        ProtocolAdapter::Native => {
            open_catalog_record(
                &node,
                &record,
                record.library_id.unwrap_or_default(),
                record.local_folder_path(),
                Arc::clone(&scrobbler),
            )
            .await
        }
        ProtocolAdapter::RuneForward {
            mode: RuneForwardMode::Replica { .. },
            ..
        } => {
            let owned_path;
            let path = if let Some(path) = record.local_folder_path() {
                path
            } else {
                owned_path = match record.root(Path::new(&*node.config_path)) {
                    Some(path) => path,
                    None => {
                        return failed(
                            OpenLibraryFailureKind::Storage,
                            anyhow::anyhow!("Replica has no storage root"),
                        );
                    }
                };
                if !owned_path.join(".rune/library.json").is_file() {
                    return failed(
                        OpenLibraryFailureKind::MediaFolderMissing,
                        anyhow::anyhow!("Replica folder is missing: {}", owned_path.display()),
                    );
                }
                match owned_path.to_str() {
                    Some(path) => path,
                    None => {
                        return failed(
                            OpenLibraryFailureKind::Storage,
                            anyhow::anyhow!("Replica path is not UTF-8"),
                        );
                    }
                }
            };
            open_catalog_record(
                &node,
                &record,
                record.library_id.unwrap_or_default(),
                Some(path),
                Arc::clone(&scrobbler),
            )
            .await
        }
        ProtocolAdapter::RuneForward {
            owner_fingerprint: fingerprint,
            mode: RuneForwardMode::RemoteControl,
        } => {
            let hosts = node
                .cert_validator
                .read()
                .await
                .get_hosts_for_fingerprint(fingerprint)
                .await;
            if hosts.is_empty() {
                Err((
                    OpenLibraryFailureKind::RemoteUntrusted,
                    anyhow::anyhow!("Fingerprint is no longer trusted"),
                ))
            } else {
                match open_remote_library_for_fingerprint(
                    node.clone(),
                    fingerprint,
                    hosts,
                    Some(record.record_id),
                )
                .await
                {
                    Ok((_, generation)) => Ok(generation),
                    Err(error) => Err((remote_open_failure(&error), error)),
                }
            }
        }
        ProtocolAdapter::SubsonicTranslate {
            endpoint,
            credential,
            ..
        } => open_subsonic_record(&node, record.record_id, endpoint, *credential).await,
    };
    match result {
        Ok(generation) => {
            if let Err(error) = LibraryRegistry::update(Path::new(&*node.config_path), |registry| {
                registry.last_open_record = Some(record_id);
                Ok(())
            })
            .await
            {
                log::error!("Could not persist last open library record: {error:#}");
            }
            OpenLibraryResponse {
                call_id: request.call_id,
                record_id: request.record_id.clone(),
                generation,
                result: OpenLibraryResult::Opened,
            }
        }
        Err((kind, error)) => failed(kind, error),
    }
}

async fn open_subsonic_record(
    node: &Arc<NodeContext>,
    record_id: LibraryRecordId,
    endpoint: &str,
    credential_ref: CredentialRef,
) -> StdResult<u64, (OpenLibraryFailureKind, anyhow::Error)> {
    let credential = node
        .credentials
        .get(credential_ref)
        .await
        .map_err(|error| (OpenLibraryFailureKind::Storage, anyhow::Error::new(error)))?;
    let password = String::from_utf8(credential.secret.as_bytes().to_vec())
        .map_err(|error| (OpenLibraryFailureKind::Storage, error.into()))?;
    let endpoint = Url::parse(endpoint)
        .map_err(|error| (OpenLibraryFailureKind::RemoteProtocol, error.into()))?;
    let client = SubsonicClient::new(
        endpoint,
        SubsonicCredentials::Password {
            user: credential.username,
            password,
        },
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| {
                (
                    OpenLibraryFailureKind::Storage,
                    anyhow::anyhow!("Could not create Subsonic client"),
                )
            })?,
    );
    let probe = match timeout(Duration::from_secs(10), client.probe()).await {
        Err(_) => {
            return Err((
                OpenLibraryFailureKind::RemoteUnreachable,
                anyhow::anyhow!("Subsonic server timed out"),
            ));
        }
        Ok(Ok(probe)) => probe,
        Ok(Err(error)) => {
            let kind = match error {
                SubsonicError::Transport(_) => OpenLibraryFailureKind::RemoteUnreachable,
                SubsonicError::Server {
                    kind:
                        SubsonicServerError::WrongCredentials
                        | SubsonicServerError::NotAuthorized
                        | SubsonicServerError::InvalidApiKey,
                    ..
                } => OpenLibraryFailureKind::RemoteNotAuthorized,
                _ => OpenLibraryFailureKind::RemoteProtocol,
            };
            let detail = match kind {
                OpenLibraryFailureKind::RemoteUnreachable => "Could not reach the Subsonic server",
                OpenLibraryFailureKind::RemoteNotAuthorized => {
                    "The Subsonic credentials were rejected"
                }
                _ => "The Subsonic server returned an invalid response",
            };
            return Err((kind, anyhow::anyhow!(detail)));
        }
    };
    let binding = Arc::new(SubsonicBinding::new(client, probe));
    node.set_open_subsonic_catalog(Some(record_id), binding)
        .map_err(|error| (OpenLibraryFailureKind::Storage, error))
}

async fn open_catalog_record(
    node: &Arc<NodeContext>,
    record: &LibraryRecord,
    library_id: Uuid,
    path: Option<&str>,
    scrobbler: Arc<Mutex<ScrobblingManager>>,
) -> std::result::Result<u64, (OpenLibraryFailureKind, anyhow::Error)> {
    if let Some(path) = path {
        #[cfg(not(target_os = "android"))]
        let fsio = FsIo::new();
        #[cfg(target_os = "android")]
        let fsio = FsIo::new(Path::new(".rune/.android-fs.db"), path)
            .map_err(|error| (OpenLibraryFailureKind::Storage, error.into()))?;
        if !path.starts_with("content://") && !Path::new(path).is_dir() {
            return Err((
                OpenLibraryFailureKind::MediaFolderMissing,
                anyhow::anyhow!("Media folder is missing: {path}"),
            ));
        }
        check_library_state(&fsio, path)
            .map_err(|error| (OpenLibraryFailureKind::Storage, error))?;
    }
    let opened = node
        .library_pool
        .acquire_ui(library_id)
        .await
        .map_err(|error| (OpenLibraryFailureKind::Storage, error))?;
    let generation = run_local_open(Ok(opened), Some(record.record_id), node, &scrobbler)
        .await
        .map_err(|error| (OpenLibraryFailureKind::Storage, error))?;
    Ok(generation)
}

fn remote_open_failure(error: &anyhow::Error) -> OpenLibraryFailureKind {
    match error.downcast_ref::<RemoteConnectError>() {
        Some(RemoteConnectError::UntrustedEndpoint) => OpenLibraryFailureKind::RemoteUntrusted,
        Some(RemoteConnectError::NotAuthorized) => OpenLibraryFailureKind::RemoteNotAuthorized,
        Some(RemoteConnectError::Unreachable) => OpenLibraryFailureKind::RemoteUnreachable,
        Some(RemoteConnectError::PeerNeedsUpdate { .. }) => {
            OpenLibraryFailureKind::ProtocolIncompatible
        }
        Some(RemoteConnectError::Protocol(_)) => OpenLibraryFailureKind::RemoteProtocol,
        None => OpenLibraryFailureKind::RemoteProtocol,
    }
}

async fn run_local_open(
    opened: Result<LibraryHandle>,
    record_id: Option<LibraryRecordId>,
    node: &Arc<NodeContext>,
    scrobbler: &Arc<Mutex<ScrobblingManager>>,
) -> Result<u64> {
    let library = opened?;
    info!("Resolved library id: {}", library.library_id);
    local_player_loop(library, node.clone(), Arc::clone(scrobbler)).await?;
    node.set_open_local_catalog(record_id)
}

pub struct CollectionCoverOptions<'a> {
    pub bake: bool,
    pub library_id: uuid::Uuid,
    pub running_mode: &'a RunningMode,
    pub remote_host: &'a Option<String>,
    pub playback_queue: &'a [i32],
}

pub async fn inject_cover_art_map(
    fsio: &FsIo,
    main_db: &MainDbConnection,
    recommend_db: Arc<RecommendationDbConnection>,
    collection: Collection,
    n: Option<i32>,
    options: CollectionCoverOptions<'_>,
) -> Result<Collection> {
    let files = query_cover_arts(
        main_db,
        recommend_db,
        if collection.collection_type == CollectionType::Track {
            if collection.queries.is_empty() {
                vec![]
            } else {
                vec![collection.queries[0].clone()]
            }
        } else {
            collection.queries.clone()
        },
        n,
        options.playback_queue,
    )
    .await?;

    // Get the base cover art paths
    let raw_cover_art_map = bake_cover_art_by_media_files(fsio, main_db, files).await?;

    // Process the cover art paths based on the running mode
    let cover_art_map = process_cover_art_map(
        raw_cover_art_map,
        options.library_id,
        options.running_mode,
        options.remote_host,
    );

    Ok(Collection {
        id: collection.id,
        name: collection.name,
        queries: collection.queries,
        collection_type: collection.collection_type,
        cover_art_map,
        readonly: collection.readonly,
    })
}

pub fn process_cover_art_map(
    cover_art_map: HashMap<i32, String>,
    library_id: uuid::Uuid,
    running_mode: &RunningMode,
    remote_host: &Option<String>,
) -> HashMap<i32, String> {
    cover_art_map
        .into_iter()
        .map(|(id, path)| {
            (
                id,
                process_cover_art_path(&path, library_id, running_mode, remote_host),
            )
        })
        .collect()
}

pub async fn query_cover_arts(
    main_db: &MainDbConnection,
    recommend_db: Arc<RecommendationDbConnection>,
    queries: Vec<MixQuery>,
    n: Option<i32>,
    queue: &[i32],
) -> Result<Vec<media_files::Model>> {
    query_mix_media_files(
        main_db,
        &recommend_db,
        queries
            .into_iter()
            .map(|q| (q.operator, q.parameter))
            .chain([("filter::with_cover_art".to_string(), "true".to_string())])
            .collect(),
        0,
        match n {
            Some(n) => {
                if n == 0 {
                    18
                } else {
                    n as usize
                }
            }
            None => 18,
        },
        queue,
    )
    .await
}

pub fn determine_batch_size(workload_factor: f32) -> usize {
    let num_cores = num_cpus::get();
    let batch_size = ((num_cores as f32) * workload_factor).round() as usize;
    let min_batch_size = 1;
    let max_batch_size = 1000;

    std::cmp::min(std::cmp::max(batch_size, min_batch_size), max_batch_size)
}

pub fn process_cover_art_path(
    path: &str,
    library_id: uuid::Uuid,
    running_mode: &RunningMode,
    remote_host: &Option<String>,
) -> String {
    match running_mode {
        RunningMode::Server => {
            // Path is from server, we need to convert to a remote URL
            if path.is_empty() {
                path.to_string()
            } else {
                // If we have a remote host, we need to construct a URL for the file
                if let Some(host) = remote_host {
                    // Extract the filename from the path, as we only need the cache part
                    let file_name = Path::new(path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default();

                    // Construct the URL using the host and cache prefix
                    format!("{host}/files/cache/{file_name}?library_id={library_id}")
                } else {
                    // No host available, return the original path
                    path.to_string()
                }
            }
        }
        RunningMode::Client => {
            // We're running as a client, so the path is already correct
            path.to_string()
        }
    }
}

pub async fn parse_media_files(
    fsio: &FsIo,
    db: &MainDbConnection,
    _node_id: &str,
    media_summaries: Vec<MetadataSummary>,
    lib_path: Arc<String>,
) -> Result<Vec<MediaFile>> {
    let ids: Vec<_> = media_summaries.iter().map(|file| file.id).collect();
    let availability = audio_availability(db, fsio, Path::new(&*lib_path), &ids).await?;
    let remote_content = remote_content_ids(db, &ids).await?;
    let mut media_files = Vec::with_capacity(media_summaries.len());

    for file in media_summaries {
        // On Android the path stays relative to the SAF tree root and is
        // opened through FsIo at playback time; canonicalizing it would
        // produce a /mnt/user/... path that is not directly readable.
        #[cfg(target_os = "android")]
        let media_path: Result<PathBuf, fsio::FileIoError> = {
            let _ = &lib_path;
            let _ = &fsio;
            Ok(Path::new(&file.directory).join(&file.file_name))
        };
        #[cfg(not(target_os = "android"))]
        let media_path = fsio.canonicalize_path(
            &Path::new(lib_path.as_ref())
                .join(&file.directory)
                .join(&file.file_name),
        );

        use crate::messages::AudioAvailability as ApiState;
        use database::actions::file::AudioAvailability as LocalState;
        let state = availability
            .get(&file.id)
            .copied()
            .unwrap_or(LocalState::Unavailable);
        let (media_path, state) = if state == LocalState::Local {
            match media_path {
                Ok(path) => (path, ApiState::Local),
                Err(_) => (PathBuf::new(), ApiState::Unavailable),
            }
        } else {
            let state = ApiState::from_local(state, remote_content.contains(&file.id));
            (PathBuf::new(), state)
        };
        let origin = audio_origin(
            if state == ApiState::Local {
                LocalState::Local
            } else {
                LocalState::Unavailable
            },
            &file.directory,
        );
        let media_file = MediaFile {
            id: file.id,
            path: media_path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Media path is None"))?
                .to_string(),
            artist: if file.artist.is_empty() {
                "Unknown Artist".to_owned()
            } else {
                file.artist
            },
            album: if file.album.is_empty() {
                "Unknown Album".to_owned()
            } else {
                file.album
            },
            title: if file.title.is_empty() {
                file.file_name
            } else {
                file.title
            },
            duration: file.duration,
            cover_art_id: file.cover_art_id.unwrap_or(-1),
            track_number: file.track_number,
            audio_availability: state,
            audio_origin: origin.into(),
        };

        media_files.push(media_file);
    }

    Ok(media_files)
}

pub async fn files_to_playback_request<P: AsRef<Path>>(
    fsio: &FsIo,
    db: &MainDbConnection,
    library_id: &str,
    lib_path: &P,
    files: &[MediaFileHandle],
) -> Result<Vec<(PlayingItem, PlaybackSource)>> {
    let ids: Vec<i32> = files.iter().filter_map(|file| file.file_id).collect();
    let library_uuid = library_id.parse::<uuid::Uuid>()?;
    let availability = audio_availability(db, fsio, lib_path.as_ref(), &ids).await?;
    let remote_content = remote_content_ids(db, &ids).await?;
    let fsio_for_paths = fsio.clone();
    let root = lib_path.as_ref().to_path_buf();
    let files_for_paths = files.to_vec();
    let local_paths = tokio::task::spawn_blocking(move || {
        files_for_paths
            .iter()
            .map(|file| match file.file_id {
                Some(_) => fsio_for_paths
                    .canonicalize_path(&root.join(&file.directory).join(&file.file_name))
                    .ok(),
                None if let PlayingItem::IndependentFile(path) = &file.item => {
                    fsio_for_paths.canonicalize_path_str(path).ok()
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    })
    .await
    .context("playback source path task failed")?;
    Ok(files
        .iter()
        .zip(local_paths)
        .filter_map(|(file, local_path)| {
            let source = match file.file_id {
                Some(id)
                    if matches!(
                        availability.get(&id),
                        Some(AudioAvailability::RemoteOnly)
                            | Some(AudioAvailability::ImportPending)
                    ) && remote_content.contains(&id) =>
                {
                    PlaybackSource::LibraryStream
                }
                Some(id) if !matches!(availability.get(&id), Some(AudioAvailability::Local)) => {
                    return None;
                }
                Some(_) => PlaybackSource::Local(local_path?),
                None if matches!(&file.item, PlayingItem::IndependentFile(_)) => {
                    PlaybackSource::Local(local_path?)
                }
                None => return None,
            };
            let item = match &file.item {
                PlayingItem::InLibrary(identity) => PlayingItem::InLibrary(*identity),
                PlayingItem::IndependentFile(path) => PlayingItem::IndependentFile(path.clone()),
                _ => PlayingItem::InLibrary(playback::player::LibraryItem {
                    library_id: library_uuid,
                    file_uuid: file.file_uuid?,
                }),
            };
            Some((item, source))
        })
        .collect())
}

pub fn find_nearest_index<T, F>(vec: &[T], hint_position: usize, predicate: F) -> Option<usize>
where
    F: Fn(&T) -> bool,
{
    if vec.is_empty() {
        return None;
    }

    let len = vec.len();
    let mut left = hint_position;
    let mut right = hint_position;

    loop {
        if left < len && predicate(&vec[left]) {
            return Some(left);
        }

        if right < len && predicate(&vec[right]) {
            return Some(right);
        }

        if left == 0 && right >= len - 1 {
            break;
        }

        left = left.saturating_sub(1);

        if right < len - 1 {
            right += 1;
        }
    }

    None
}

#[cfg(not(target_os = "android"))]
pub fn init_logging() {
    let stdout_filter = EnvFilter::new(
        "symphonia_format_ogg=off,symphonia_core=off,symphonia_bundle_mp3::demuxer=off,sea_orm_migration::migrator=off,info",
    );

    tracing_subscriber::fmt()
        .with_env_filter(stdout_filter)
        .init();
}

#[cfg(test)]
mod moved_node_request_tests {
    use std::{sync::Arc, time::Duration};

    use anyhow::Result;
    use database::library_registry::{
        CatalogPlacement, FsAdapter, LibraryRecord, LibraryRecordId, LibraryRegistry,
        ProtocolAdapter,
    };
    use scrobbling::manager::ScrobblingManager;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use super::{Broadcaster, ProductionNodeRequestHandlers, RinfRustSignal};
    use crate::{
        macros::NodeRequestHandlers,
        messages::{GetSyncSettingsRequest, MissingHostFilePolicy},
        node::NodeSlot,
    };

    struct Quiet;

    impl Broadcaster for Quiet {
        fn broadcast(&self, _: &dyn RinfRustSignal) {}
    }

    #[tokio::test]
    async fn get_sync_settings_succeeds_with_and_without_an_open_library() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().to_string_lossy().into_owned();
        let node_slot = NodeSlot::new();
        let node = node_slot.initialize(&path, &path, Arc::new(Quiet)).await?;
        let handlers = ProductionNodeRequestHandlers::new(Arc::new(Mutex::new(
            ScrobblingManager::new(10, Duration::from_secs(5)),
        )));
        let request = GetSyncSettingsRequest {
            request_id: "sync-settings-test".into(),
            call_id: None,
        };

        let closed_response = handlers
            .handle_get_sync_settings_request(&node_slot, &request)
            .await?
            .expect("sync settings response with no UI library");

        let library_id = Uuid::new_v4();
        LibraryRegistry::update(directory.path(), |registry| {
            registry.libraries.push(LibraryRecord {
                record_id: LibraryRecordId(Uuid::new_v4()),
                alias: "Test library".into(),
                library_id: Some(library_id),
                protocol: ProtocolAdapter::Native,
                fs: vec![FsAdapter::Local { path: path.clone() }],
                catalog: CatalogPlacement::InMediaRoot,
                background_sync: true,
            });
            Ok(())
        })
        .await?;
        let library = node.library_pool.acquire_ui(library_id).await?;
        let params = Arc::new(node.params_for_ui()?.for_library(library.clone()));
        *params.ui_library.lock().await = Some(library);
        node.attach_ui(&params).await;
        node.set_open_local_catalog(None)?;

        let open_response = handlers
            .handle_get_sync_settings_request(&node_slot, &request)
            .await?
            .expect("sync settings response with a UI library");
        assert_eq!(open_response.request_id, closed_response.request_id);
        assert_eq!(open_response.listen, closed_response.listen);
        assert!(matches!(
            (
                open_response.missing_host_file,
                closed_response.missing_host_file
            ),
            (
                MissingHostFilePolicy::RemoveEverywhere,
                MissingHostFilePolicy::RemoveEverywhere
            ) | (
                MissingHostFilePolicy::KeepAsRemote,
                MissingHostFilePolicy::KeepAsRemote
            )
        ));
        assert_eq!(open_response.error, closed_response.error);
        assert!(open_response.error.is_none());

        node.close_ui(&params.main_token).await;
        node.shutdown().await?;
        Ok(())
    }
}

#[cfg(test)]
mod cover_art_url_tests {
    use super::{RunningMode, process_cover_art_path};
    use uuid::Uuid;

    #[test]
    fn remote_cover_url_names_its_library() {
        let library_id = Uuid::new_v4();
        let path = "/tmp/rune/cover_arts/cover-digest";
        let remote_host = Some("https://peer.example".to_string());

        assert_eq!(
            process_cover_art_path(path, library_id, &RunningMode::Server, &remote_host),
            format!("https://peer.example/files/cache/cover-digest?library_id={library_id}")
        );
        assert_eq!(
            process_cover_art_path(path, library_id, &RunningMode::Client, &remote_host),
            path
        );
    }
}

#[cfg(target_os = "android")]
pub fn init_logging() {
    let tag = LogcatTag::Fixed(env!("CARGO_PKG_NAME").to_owned());
    let writer = match LogcatMakeWriter::new(tag) {
        Ok(writer) => writer,
        Err(e) => {
            eprintln!("Failed to initialize logcat writer: {e:?}");
            return;
        }
    };

    // JNI_OnLoad (playback::android_utils) may have already installed a global
    // subscriber; a second .init() would panic and kill the Rust main thread.
    let _ = tracing_subscriber::fmt()
        .event_format(Format::default().with_level(false).without_time())
        .with_writer(writer)
        .with_ansi(false)
        .try_init();
}
