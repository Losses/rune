use anyhow::Error;
use database::sync::LibrarySyncStore;
use fsio::ObjectStoreCapabilities;
use reqwest::Client;
use std::cmp;
use std::fmt;
use std::fs;
use std::result;
use std::sync as std_sync;
use tokio::task;
use tokio::time as tokio_time;
use tokio::{
    runtime::Handle,
    time::{Duration, timeout},
};
use uuid::Uuid;
pub mod broadcastable;
pub mod diagnostics;
pub mod file_dialog;
pub mod nid;
pub mod player;
pub mod sync_settings;

use database::actions::file::{
    AudioAvailability, audio_availability, audio_origin, remote_content_ids,
};
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    fmt::Debug,
    future::Future,
    io::ErrorKind,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    result::Result as StdResult,
    sync::{Arc, OnceLock},
};

use crate::handlers::local_libraries::publish_library_records_changed;
use crate::node::{NodeContext, NodeSlot};
use anyhow::{Context, Result};
use fsio::{FileIoError, FsIo, OpenDalConfig, OpenDalObjectStore};
use log::info;
use scrobbling::manager::ScrobblingServiceManager;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use ::database::sync::model::RuneModel;
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
    error::NewerLibraryDatabase,
    library_registry::{
        CatalogPlacement, FsAdapter, InvalidRecord, LibraryRecord, LibraryRecordId,
        LibraryRegistry, ProtocolAdapter, RuneForwardMode, ServiceKind, SubsonicOpenMode,
        resolve_library,
    },
    playing_item::{MediaFileHandle, PlayingItemMetadataSummary},
};
use ::discovery::{client::CertValidator, protocol::DiscoveryService, server::PermissionManager};
use ::playback::{
    player::{Playable, PlaybackSource, PlayingItem, RemoteAudioError},
    sfx_player::SfxPlayer,
};
use ::scrobbling::manager::ScrobblingManager;
use ::sync::protocol::SyncModel;
use ::sync::share::{ObjectStore, ObjectValue, StoreError, decode_library, library_key};

use crate::Signal;
use crate::backends::{
    local::local_player_loop,
    remote::{RemoteConnectError, open_remote_library_for_fingerprint},
};
use crate::library_pool::{LibraryHandle, LibraryPool, MediaFolderMissing};
use crate::macros::{NodeRequestHandlers, listen_local_gui};
use crate::messages::*;
use crate::request_error::ExpectedRequestError;
use crate::request_owner::Implementation;
use crate::server::ServerManager;
use crate::service::ServiceBinding;
use crate::service::lists::delete_kept_lists;
use crate::service::registry::VerifiedSignIn;
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

/// The local library a request about this device's own data works on, as the UI has it open. A
/// streaming service library keeps no such data on this device, so while one is open there is
/// none, and such a request answers as if there were nothing to read or change.
#[derive(Clone)]
pub struct LocalLibrary {
    library: Option<LibraryHandle>,
    service_open: bool,
}

impl LocalLibrary {
    pub fn from_global(params: &GlobalParams) -> Self {
        Self {
            library: params.library.clone(),
            service_open: params.node.service_library_open(),
        }
    }

    /// The local library, or none while a service library is open. Without any open library it
    /// is the expected `Unavailable` error that `require_library` gives.
    pub fn or_service(self) -> Result<Option<LibraryHandle>> {
        match (self.library, self.service_open) {
            (Some(library), _) => Ok(Some(library)),
            (None, true) => Ok(None),
            (None, false) => require_library(None).map(Some),
        }
    }
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
    pub library_tasks: Arc<std_sync::Mutex<LibraryTaskOwner>>,
    pub player: Arc<Mutex<dyn Playable>>,
    pub sfx_player: Arc<Mutex<SfxPlayer>>,
    pub scrobbler: Arc<Mutex<dyn ScrobblingServiceManager>>,
    pub broadcaster: Arc<dyn Broadcaster>,
    pub device_scanner: Arc<DiscoveryService>,
    pub cert_validator: Arc<RwLock<CertValidator>>,
    pub permission_manager: Arc<RwLock<PermissionManager>>,
    pub server_manager: OnceLock<Arc<ServerManager>>,
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
    config_dir: &Path,
) -> Result<LibrarySyncStore> {
    let device_id = Uuid::parse_str(device_id)?;
    let token_dir = config_dir.join("causal-writer-identities");
    fs::create_dir_all(&token_dir).context("Create causal writer identity directory")?;
    let token_path = token_dir.join(format!("{library_id}.token"));
    let file_token = match fs::read_to_string(&token_path) {
        Ok(value) => Uuid::parse_str(value.trim()).context("Read causal writer identity")?,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let token = Uuid::new_v4();
            let temporary = token_dir.join(format!("{library_id}.{}.tmp", Uuid::new_v4()));
            fs::write(&temporary, token.to_string()).context("Write causal writer identity")?;
            fs::rename(&temporary, &token_path).context("Install causal writer identity")?;
            Uuid::parse_str(
                fs::read_to_string(&token_path)
                    .context("Read installed causal writer identity")?
                    .trim(),
            )?
        }
        Err(error) => return Err(error).context("Read causal writer identity"),
    };
    let library = LibrarySyncStore::open_with_file_token(
        db.clone(),
        device_id,
        Some(library_id),
        Some(file_token),
    )
    .await?;
    library.import_existing(device_id).await?;
    Ok(library)
}

impl Debug for GlobalParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
    fn as_any(&self) -> &dyn Any;

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
    fn handle_get_open_library_request(
        &self,
        node: &NodeSlot,
        request: &GetOpenLibraryRequest,
    ) -> impl Future<Output = Result<Option<GetOpenLibraryResponse>>> + Send {
        async move {
            let node = node.current().await?;
            Ok(Some(GetOpenLibraryResponse {
                library_id: node
                    .current_ui()
                    .await
                    .ok()
                    .and_then(|params| params.library_id)
                    .map(|library_id| library_id.to_string()),
                call_id: request.call_id,
            }))
        }
    }

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

    fn handle_set_online_lyrics_request(
        &self,
        node: &NodeSlot,
        request: &SetOnlineLyricsRequest,
    ) -> impl Future<Output = Result<Option<SetOnlineLyricsResponse>>> + Send {
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

    fn handle_list_paired_servers_request(
        &self,
        node: &NodeSlot,
        request: &ListPairedServersRequest,
    ) -> impl Future<Output = Result<Option<ListPairedServersResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request.handle(node, None, request).await
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
            let node = node.current().await?;
            request
                .handle((Arc::clone(&node.gui_broadcaster), node), None, request)
                .await
        }
    }

    fn handle_set_visualizer_scenes_request(
        &self,
        node: &NodeSlot,
        request: &SetVisualizerScenesRequest,
    ) -> impl Future<Output = Result<Option<()>>> + Send {
        async move {
            let node = node.current().await?;
            request.handle(node, None, request).await
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
            let node = node.current().await?;
            request.handle(node, None, request).await
        }
    }

    fn handle_list_visualizer_presets_request(
        &self,
        node: &NodeSlot,
        request: &ListVisualizerPresetsRequest,
    ) -> impl Future<Output = Result<Option<ListVisualizerPresetsResponse>>> + Send {
        async move {
            let node = node.current().await?;
            request.handle(node, None, request).await
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
            request.handle(node, None, request).await
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
                    (
                        node.config_path.clone(),
                        Arc::clone(&node.cert_validator),
                        node.run_mode,
                    ),
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
        async move { register_local_library(node, request).await.map(Some) }
    }

    fn handle_register_remote_control_request(
        &self,
        node: &NodeSlot,
        request: &RegisterRemoteControlRequest,
    ) -> impl Future<Output = Result<Option<RegisterLibraryResponse>>> + Send {
        async move { register_remote_control(node, request).await.map(Some) }
    }

    fn handle_register_subsonic_library_request(
        &self,
        node: &NodeSlot,
        request: &RegisterSubsonicLibraryRequest,
    ) -> impl Future<Output = Result<Option<RegisterLibraryResponse>>> + Send {
        async move { register_subsonic_library(node, request).await.map(Some) }
    }

    fn handle_replace_subsonic_credential_request(
        &self,
        node: &NodeSlot,
        request: &ReplaceSubsonicCredentialRequest,
    ) -> impl Future<Output = Result<Option<ReplaceSubsonicCredentialResponse>>> + Send {
        async move {
            let node = node
                .current()
                .await
                .map_err(|_| credential_failure(RegisterLibraryFailureKind::Storage))?;
            replace_subsonic_credential_on_node(request, &node).await?;
            Ok(Some(ReplaceSubsonicCredentialResponse {
                call_id: request.call_id,
            }))
        }
    }

    fn handle_register_service_library_request(
        &self,
        node: &NodeSlot,
        request: &RegisterServiceLibraryRequest,
    ) -> impl Future<Output = Result<Option<RegisterLibraryResponse>>> + Send {
        async move {
            let node = node.current().await.map_err(|_| {
                registration_failure(
                    RegisterLibraryFailureKind::Storage,
                    "Could not access library storage",
                )
            })?;
            register_service_library_on_node(request, &node)
                .await
                .map(Some)
        }
    }

    fn handle_sign_in_service_library_request(
        &self,
        node: &NodeSlot,
        request: &SignInServiceLibraryRequest,
    ) -> impl Future<Output = Result<Option<SignInServiceLibraryResponse>>> + Send {
        async move {
            let node = node.current().await.map_err(|_| {
                registration_failure(
                    RegisterLibraryFailureKind::Storage,
                    "Could not access library storage",
                )
            })?;
            sign_in_service_library_on_node(request, &node).await?;
            Ok(Some(SignInServiceLibraryResponse {
                call_id: request.call_id,
            }))
        }
    }

    fn handle_sign_out_service_library_request(
        &self,
        node: &NodeSlot,
        request: &SignOutServiceLibraryRequest,
    ) -> impl Future<Output = Result<Option<SignOutServiceLibraryResponse>>> + Send {
        async move {
            let node = node.current().await.map_err(|_| {
                registration_failure(
                    RegisterLibraryFailureKind::Storage,
                    "Could not access library storage",
                )
            })?;
            sign_out_service_library_on_node(request, &node).await?;
            Ok(Some(SignOutServiceLibraryResponse {
                call_id: request.call_id,
            }))
        }
    }

    fn handle_register_open_dal_library_request(
        &self,
        node: &NodeSlot,
        request: &RegisterOpenDalLibraryRequest,
    ) -> impl Future<Output = Result<Option<RegisterLibraryResponse>>> + Send {
        async move { register_opendal_library(node, request).await.map(Some) }
    }

    fn handle_detect_open_dal_share_data_request(
        &self,
        node: &NodeSlot,
        request: &DetectOpenDalShareDataRequest,
    ) -> impl Future<Output = Result<Option<DetectOpenDalShareDataResponse>>> + Send {
        async move {
            node.current().await.map_err(|_| {
                registration_failure(
                    RegisterLibraryFailureKind::Storage,
                    "Could not access library storage",
                )
            })?;
            let result = detect_opendal_share_data(&request.service)
                .await
                .map_err(|kind| registration_failure(kind, "Could not inspect the share"))?;
            Ok(Some(DetectOpenDalShareDataResponse {
                result,
                call_id: request.call_id,
            }))
        }
    }

    fn handle_open_library_request(
        &self,
        node: &NodeSlot,
        request: &OpenLibraryRequest,
    ) -> impl Future<Output = Result<Option<OpenLibraryResponse>>> + Send {
        let scrobbler = Arc::clone(&self.scrobbler);
        async move {
            open_library_by_record(node, request, scrobbler)
                .await
                .map(Some)
        }
    }
}

async fn register_opendal_library(
    node_slot: &NodeSlot,
    request: &RegisterOpenDalLibraryRequest,
) -> Result<RegisterLibraryResponse> {
    match node_slot.current().await {
        Ok(node) => register_opendal_library_on_node(request, &node).await,
        Err(_) => Err(registration_failure(
            RegisterLibraryFailureKind::Storage,
            "Could not access library storage",
        )),
    }
}

pub(crate) async fn detect_opendal_share_data(
    service: &OpenDalLibraryService,
) -> StdResult<OpenDalShareDataDetection, RegisterLibraryFailureKind> {
    let config = match service {
        OpenDalLibraryService::WebDav {
            endpoint,
            root,
            username,
            password,
        } => OpenDalConfig::WebDav {
            endpoint: endpoint.clone(),
            root: root.clone(),
            username: username.clone(),
            password: password.clone(),
        },
        OpenDalLibraryService::S3 {
            endpoint,
            bucket,
            region,
            root,
            access_key_id,
            secret_access_key,
        } => OpenDalConfig::S3 {
            endpoint: endpoint.clone(),
            bucket: bucket.clone(),
            region: region.clone(),
            root: root.clone(),
            access_key_id: access_key_id.clone(),
            secret_access_key: secret_access_key.clone(),
        },
    };
    let store = OpenDalObjectStore::new(config)
        .map_err(|_| RegisterLibraryFailureKind::OpenDalUnreachable)?;
    let library = ObjectStore::get(&store, &library_key()).await;
    let probe = if matches!(library, Err(StoreError::NotFound)) {
        let prefix = format!("devices/{}.1", Uuid::new_v4());
        store.probe(&prefix).await
    } else {
        Ok(ObjectStoreCapabilities::default())
    };
    classify_opendal_share_data(library, probe)
}

fn classify_opendal_share_data(
    library: Result<ObjectValue, StoreError>,
    probe: Result<fsio::ObjectStoreCapabilities, StoreError>,
) -> StdResult<OpenDalShareDataDetection, RegisterLibraryFailureKind> {
    match library {
        Ok(value) => match decode_library(&value.bytes) {
            Ok(library)
                if library.library_id != Uuid::nil()
                    && library.model == RuneModel::model_id()
                    && library.schema == RuneModel::schema_id() =>
            {
                Ok(OpenDalShareDataDetection::ExistingLibrary {
                    library_id: library.library_id.to_string(),
                })
            }
            _ => Err(RegisterLibraryFailureKind::OpenDalShareInvalid),
        },
        Err(StoreError::PermissionDenied) => Ok(OpenDalShareDataDetection::ReadOnly),
        Err(StoreError::Unauthorized) => {
            Err(RegisterLibraryFailureKind::OpenDalAuthenticationRejected)
        }
        Err(StoreError::NotFound) => match probe {
            Ok(capabilities) if capabilities.write_probe => {
                Ok(OpenDalShareDataDetection::WritableEmpty)
            }
            Ok(_) | Err(StoreError::PermissionDenied) => Ok(OpenDalShareDataDetection::ReadOnly),
            Err(StoreError::Unauthorized) => {
                Err(RegisterLibraryFailureKind::OpenDalAuthenticationRejected)
            }
            Err(_) => Err(RegisterLibraryFailureKind::OpenDalUnreachable),
        },
        Err(_) => Err(RegisterLibraryFailureKind::OpenDalUnreachable),
    }
}

async fn read_share_library_id(
    config: OpenDalConfig,
) -> Result<Option<Uuid>, RegisterLibraryFailureKind> {
    let store = OpenDalObjectStore::new(config)
        .map_err(|_| RegisterLibraryFailureKind::OpenDalUnreachable)?;
    match ObjectStore::get(&store, &library_key()).await {
        Ok(value) => {
            let library = decode_library(&value.bytes)
                .map_err(|_| RegisterLibraryFailureKind::OpenDalShareInvalid)?;
            if library.library_id == Uuid::nil()
                || library.model != RuneModel::model_id()
                || library.schema != RuneModel::schema_id()
            {
                return Err(RegisterLibraryFailureKind::OpenDalShareInvalid);
            }
            Ok(Some(library.library_id))
        }
        Err(StoreError::NotFound | StoreError::PermissionDenied) => Ok(None),
        Err(StoreError::Unauthorized) => {
            Err(RegisterLibraryFailureKind::OpenDalAuthenticationRejected)
        }
        Err(_) => Err(RegisterLibraryFailureKind::OpenDalUnreachable),
    }
}

pub(crate) async fn register_opendal_library_on_node(
    request: &RegisterOpenDalLibraryRequest,
    node: &Arc<NodeContext>,
) -> Result<RegisterLibraryResponse> {
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
    let fs_config = config.clone();
    let handle = Handle::current();
    let probe = task::spawn_blocking(move || {
        catch_unwind(AssertUnwindSafe(|| -> Result<usize, FileIoError> {
            let fsio = FsIo::new_opendal(fs_config, handle)?;
            let nodes = fsio.walk_dir(Path::new(""), false)?.nodes;
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
            return Err(registration_failure(
                RegisterLibraryFailureKind::OpenDalUnreachable,
                "Could not reach the remote library",
            ));
        }
        Ok(Err(FileIoError::PermissionDenied(_))) => {
            return Err(registration_failure(
                RegisterLibraryFailureKind::OpenDalAuthenticationRejected,
                "The remote credentials were rejected",
            ));
        }
        Ok(Err(FileIoError::Remote(_))) => {
            return Err(registration_failure(
                RegisterLibraryFailureKind::OpenDalUnreachable,
                "Could not reach the remote library",
            ));
        }
        Ok(Err(_)) => {
            return Err(registration_failure(
                RegisterLibraryFailureKind::OpenDalNotFolder,
                "The selected remote root is not a folder",
            ));
        }
        Ok(Ok(count)) => count,
    };
    if is_webdav && count == 0 {
        return Err(registration_failure(
            RegisterLibraryFailureKind::OpenDalNotFolder,
            "The selected remote root is not a folder",
        ));
    }
    let config_dir = Path::new(&*node.config_path);
    let existing_share_id = match read_share_library_id(config.clone()).await {
        Ok(library_id) => library_id,
        Err(kind) => {
            return Err(registration_failure(
                kind,
                "Could not inspect Rune share data",
            ));
        }
    };
    if existing_share_id.is_none() && request.share_data_choice == ShareDataChoice::KeepInShare {
        let store = match OpenDalObjectStore::new(config) {
            Ok(store) => store,
            Err(_) => {
                return Err(registration_failure(
                    RegisterLibraryFailureKind::OpenDalUnreachable,
                    "Could not reach the remote library",
                ));
            }
        };
        let prefix = format!("devices/{}.1", Uuid::new_v4());
        match store.probe(&prefix).await {
            Ok(capabilities) if capabilities.write_probe => {}
            Ok(_) | Err(StoreError::PermissionDenied) => {
                return Err(registration_failure(
                    RegisterLibraryFailureKind::OpenDalReadOnly,
                    "Choose a writable folder to keep the index in the share",
                ));
            }
            Err(StoreError::Unauthorized) => {
                return Err(registration_failure(
                    RegisterLibraryFailureKind::OpenDalAuthenticationRejected,
                    "The remote credentials were rejected",
                ));
            }
            Err(_) => {
                return Err(registration_failure(
                    RegisterLibraryFailureKind::OpenDalUnreachable,
                    "Could not reach the remote library",
                ));
            }
        }
    }
    let registry = match LibraryRegistry::load(config_dir) {
        Ok(registry) => registry,
        Err(_) => {
            return Err(registration_failure(
                RegisterLibraryFailureKind::Storage,
                "Could not read library storage",
            ));
        }
    };
    let existing = registry
        .libraries
        .iter()
        .find(|record| match (&request.service, record.fs.first()) {
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
        })
        .cloned();
    if let Some(library_id) = existing_share_id {
        let conflicts_with_record = registry.libraries.iter().any(|record| {
            record.library_id == Some(library_id)
                && existing
                    .as_ref()
                    .is_none_or(|existing| existing.record_id != record.record_id)
        });
        let conflicts_with_invalid = registry
            .invalid_records()
            .any(|(record, _)| record.library_id == Some(library_id));
        let changed_existing_share = existing.as_ref().is_some_and(|record| {
            matches!(&record.catalog, CatalogPlacement::InShare(_))
                && record.library_id != Some(library_id)
        });
        if conflicts_with_record || conflicts_with_invalid || changed_existing_share {
            return Err(registration_failure(
                RegisterLibraryFailureKind::OpenDalShareChanged,
                "The share belongs to another library",
            ));
        }
    }
    if let Some(mut record) = existing {
        if let Some(library_id) = existing_share_id {
            let storage_key = match &record.catalog {
                CatalogPlacement::InShare(key) | CatalogPlacement::Redirected(key) => *key,
                _ => Uuid::new_v4(),
            };
            record.library_id = Some(library_id);
            record.catalog = CatalogPlacement::InShare(storage_key);
            record = match LibraryRegistry::update(config_dir, |registry| {
                let stored = registry
                    .libraries
                    .iter_mut()
                    .find(|stored| stored.record_id == record.record_id)
                    .context("OpenDAL library record disappeared")?;
                *stored = record.clone();
                Ok(record.clone())
            })
            .await
            {
                Ok(record) => record,
                Err(_) => {
                    return Err(registration_failure(
                        RegisterLibraryFailureKind::Storage,
                        "Could not update library storage",
                    ));
                }
            };
        }
        return Ok(RegisterLibraryResponse {
            call_id: None,
            record: Some((&record).into()),
            outcome: RegisterLibraryOutcome::Registered,
        });
    }
    let (library_id, catalog) = if let Some(library_id) = existing_share_id {
        (library_id, CatalogPlacement::InShare(Uuid::new_v4()))
    } else {
        let storage_key = Uuid::new_v4();
        let catalog = match request.share_data_choice {
            ShareDataChoice::KeepInShare => CatalogPlacement::InShare(storage_key),
            ShareDataChoice::ThisDeviceOnly => CatalogPlacement::Redirected(storage_key),
        };
        (Uuid::new_v4(), catalog)
    };
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
            return Err(registration_failure(
                RegisterLibraryFailureKind::Storage,
                "Could not store credentials",
            ));
        }
    };
    let record = LibraryRecord {
        record_id: LibraryRecordId(Uuid::new_v4()),
        alias: request.alias.clone(),
        library_id: Some(library_id),
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
        catalog,
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
            return Err(registration_failure(
                RegisterLibraryFailureKind::Storage,
                "Could not write library storage",
            ));
        }
    };
    if let Err(error) = publish_library_records_changed(node).await {
        log::warn!("Could not publish library record change: {error:#}");
    }
    Ok(RegisterLibraryResponse {
        call_id: None,
        record: Some((&record).into()),
        outcome: RegisterLibraryOutcome::Registered,
    })
}

/// A failed library registration of `kind`; the detail goes to the logs.
fn registration_failure(
    kind: RegisterLibraryFailureKind,
    detail: impl fmt::Display + Send + Sync + 'static,
) -> anyhow::Error {
    Error::new(ExpectedRequestError::LibraryRegistrationFailed { kind }).context(detail)
}

async fn register_local_library(
    node_slot: &NodeSlot,
    request: &RegisterLocalLibraryRequest,
) -> Result<RegisterLibraryResponse> {
    let node = node_slot.current().await.map_err(|error| {
        registration_failure(RegisterLibraryFailureKind::Storage, format!("{error:#}"))
    })?;
    register_local_library_on_node(request, &node).await
}

pub(crate) async fn register_local_library_on_node(
    request: &RegisterLocalLibraryRequest,
    node: &Arc<NodeContext>,
) -> Result<RegisterLibraryResponse> {
    let fail = |kind, error: anyhow::Error| registration_failure(kind, format!("{error:#}"));
    let config = node.config_path.clone();
    let path = &request.path;
    #[cfg(not(target_os = "android"))]
    let fsio = FsIo::new();
    #[cfg(target_os = "android")]
    let fsio = match FsIo::new(Path::new(".rune/.android-fs.db"), path) {
        Ok(fsio) => fsio,
        Err(error) => return Err(fail(RegisterLibraryFailureKind::Storage, error.into())),
    };
    let state = match check_library_state(&fsio, path) {
        Ok(state) => state,
        Err(error) => return Err(fail(RegisterLibraryFailureKind::InvalidSource, error)),
    };
    if matches!(state, LibraryState::Uninitialized) && request.mode.is_none() {
        return Ok(RegisterLibraryResponse {
            call_id: None,
            record: None,
            outcome: RegisterLibraryOutcome::NeedsInitializeMode,
        });
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
            Ok(RegisterLibraryResponse {
                call_id: None,
                record: Some((&record).into()),
                outcome: RegisterLibraryOutcome::Registered,
            })
        }
        Err(error) if error.downcast_ref::<InvalidRecord>().is_some() => {
            Err(fail(RegisterLibraryFailureKind::InvalidRecord, error))
        }
        Err(error) => Err(fail(RegisterLibraryFailureKind::Storage, error)),
    }
}

async fn register_remote_control(
    node_slot: &NodeSlot,
    request: &RegisterRemoteControlRequest,
) -> Result<RegisterLibraryResponse> {
    let node = node_slot.current().await.map_err(|error| {
        registration_failure(RegisterLibraryFailureKind::Storage, format!("{error:#}"))
    })?;
    register_remote_control_on_node(request, &node).await
}

pub(crate) async fn register_remote_control_on_node(
    request: &RegisterRemoteControlRequest,
    node: &Arc<NodeContext>,
) -> Result<RegisterLibraryResponse> {
    let hosts = node
        .cert_validator
        .read()
        .await
        .get_hosts_for_fingerprint(&request.fingerprint)
        .await;
    if hosts.is_empty() {
        return Err(registration_failure(
            RegisterLibraryFailureKind::Untrusted,
            "Fingerprint is not trusted",
        ));
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
            Ok(RegisterLibraryResponse {
                call_id: None,
                record: Some((&record).into()),
                outcome: RegisterLibraryOutcome::Registered,
            })
        }
        Err(error) if error.downcast_ref::<InvalidRecord>().is_some() => Err(registration_failure(
            RegisterLibraryFailureKind::InvalidRecord,
            format!("{error:#}"),
        )),
        Err(error) => Err(registration_failure(
            RegisterLibraryFailureKind::Storage,
            format!("{error:#}"),
        )),
    }
}

async fn register_subsonic_library(
    node_slot: &NodeSlot,
    request: &RegisterSubsonicLibraryRequest,
) -> Result<RegisterLibraryResponse> {
    match node_slot.current().await {
        Ok(node) => register_subsonic_library_with_node(request, &node).await,
        Err(_) => Err(registration_failure(
            RegisterLibraryFailureKind::Storage,
            "Could not access library storage",
        )),
    }
}

pub(crate) async fn register_subsonic_library_with_node(
    request: &RegisterSubsonicLibraryRequest,
    node: &Arc<NodeContext>,
) -> Result<RegisterLibraryResponse> {
    register_subsonic_library_on_node(request, node).await
}

pub(crate) async fn register_subsonic_library_on_node(
    request: &RegisterSubsonicLibraryRequest,
    node: &Arc<NodeContext>,
) -> Result<RegisterLibraryResponse> {
    let endpoint = match validate_subsonic_endpoint(&request.endpoint) {
        Ok(endpoint) => endpoint,
        Err(()) => {
            return Err(registration_failure(
                RegisterLibraryFailureKind::SubsonicNotSubsonic,
                "The endpoint must be a valid HTTP or HTTPS URL",
            ));
        }
    };
    let http = match Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(http) => http,
        Err(_) => {
            return Err(registration_failure(
                RegisterLibraryFailureKind::Storage,
                "Could not create the Subsonic client",
            ));
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
    let probe = match tokio_time::timeout(Duration::from_secs(5), client.probe()).await {
        Err(_) => {
            return Err(registration_failure(
                RegisterLibraryFailureKind::SubsonicUnreachable,
                "The Subsonic server did not respond in time",
            ));
        }
        Ok(Err(error)) => return Err(subsonic_probe_failure(error)),
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
        return Err(registration_failure(
            RegisterLibraryFailureKind::SubsonicRoleMissing,
            "The account has no supported Subsonic role",
        ));
    }
    let mode = match request.mode.map(SubsonicOpenMode::from) {
        Some(mode) if allowed.contains(&mode) => mode,
        Some(_) => {
            return Err(registration_failure(
                RegisterLibraryFailureKind::SubsonicRoleMissing,
                "The account does not have the role required by that mode",
            ));
        }
        None => {
            return Ok(RegisterLibraryResponse {
                call_id: None,
                record: None,
                outcome: RegisterLibraryOutcome::NeedsSubsonicMode {
                    allowed: allowed.into_iter().map(Into::into).collect(),
                },
            });
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
            return Ok(RegisterLibraryResponse {
                call_id: None,
                record: Some((&record).into()),
                outcome: RegisterLibraryOutcome::Registered,
            });
        }
        Ok(None) => {}
        Err(_) => {
            return Err(registration_failure(
                RegisterLibraryFailureKind::Storage,
                "Could not read library storage",
            ));
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
            return Err(registration_failure(
                RegisterLibraryFailureKind::Storage,
                "Could not store credentials",
            ));
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
        return Err(registration_failure(
            RegisterLibraryFailureKind::InvalidRecord,
            "The library record is invalid",
        ));
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
            return Err(registration_failure(
                RegisterLibraryFailureKind::Storage,
                "Could not write library storage",
            ));
        }
    };
    if let Err(error) = publish_library_records_changed(node).await {
        log::warn!("Could not publish library record change: {error:#}");
    }
    Ok(RegisterLibraryResponse {
        call_id: None,
        record: Some((&record).into()),
        outcome: RegisterLibraryOutcome::Registered,
    })
}

pub(crate) async fn replace_subsonic_credential_on_node(
    request: &ReplaceSubsonicCredentialRequest,
    node: &Arc<NodeContext>,
) -> Result<()> {
    use RegisterLibraryFailureKind as Kind;
    let Ok(record_id) = Uuid::parse_str(&request.record_id).map(LibraryRecordId) else {
        return Err(credential_failure(Kind::InvalidRecord));
    };
    let record = match LibraryRegistry::load(Path::new(&*node.config_path))
        .and_then(|registry| registry.record(record_id).cloned())
    {
        Ok(record) => record,
        Err(_) => return Err(credential_failure(Kind::InvalidRecord)),
    };
    let ProtocolAdapter::SubsonicTranslate {
        endpoint,
        credential,
        ..
    } = &record.protocol
    else {
        return Err(credential_failure(Kind::InvalidRecord));
    };
    let endpoint = match validate_subsonic_endpoint(endpoint) {
        Ok(endpoint) => endpoint,
        Err(()) => return Err(credential_failure(Kind::InvalidRecord)),
    };
    let http = match Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(http) => http,
        Err(_) => return Err(credential_failure(Kind::Storage)),
    };
    let replacement = SubsonicCredentials::Password {
        user: request.username.clone(),
        password: request.password.clone(),
    };
    let client = SubsonicClient::new(endpoint, replacement, http);
    let probe = match timeout(Duration::from_secs(5), client.probe()).await {
        Err(_) => return Err(credential_failure(Kind::SubsonicUnreachable)),
        Ok(Err(SubsonicError::Transport(_) | SubsonicError::HttpStatus(_))) => {
            return Err(credential_failure(Kind::SubsonicUnreachable));
        }
        Ok(Err(SubsonicError::Server {
            kind:
                SubsonicServerError::WrongCredentials
                | SubsonicServerError::NotAuthorized
                | SubsonicServerError::InvalidApiKey,
            ..
        })) => return Err(credential_failure(Kind::SubsonicWrongCredentials)),
        Ok(Err(_)) => return Err(credential_failure(Kind::SubsonicWrongCredentials)),
        Ok(Ok(probe)) => probe,
    };
    {
        let _commit = node.subsonic_credential_commit.lock().await;
        let verified = Credential {
            username: request.username.clone(),
            secret: Secret::new(request.password.as_bytes().to_vec()),
        };
        let previous = node
            .credentials
            .get(*credential)
            .await
            .ok()
            .map(|previous| previous.username);
        node.credentials
            .replace(*credential, &verified)
            .await
            .map_err(|_| credential_failure(Kind::Storage))?;
        let config_dir = Path::new(&*node.config_path);
        let library_id = record.catalog_library_id().unwrap_or(record_id.0);
        match node.service_binding(record_id.0)? {
            Some(binding) => binding.renew_sign_in(&verified),
            // Another username is another account, whose lists the closed library's kept ones
            // are not. They are never given to it, so a deletion that fails leaves only files
            // nothing reads.
            None if previous.is_some_and(|previous| previous != verified.username) => {
                if let Err(error) = delete_kept_lists(config_dir, library_id).await {
                    log::warn!("Could not delete the lists of another account: {error}");
                }
            }
            None => {}
        }
        // The library opens next time with the roles the server gives the new sign-in.
        SubsonicBinding::keep_verified_probe(config_dir, library_id, &verified.username, probe);
    }
    if let Err(error) = publish_library_records_changed(node).await {
        log::warn!("Could not publish library record change: {error:#}");
    }
    Ok(())
}

/// A failed credential replacement, typed like a failed Subsonic registration.
fn credential_failure(kind: RegisterLibraryFailureKind) -> anyhow::Error {
    ExpectedRequestError::LibraryRegistrationFailed { kind }.into()
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

fn subsonic_probe_failure(error: SubsonicError) -> anyhow::Error {
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
        | SubsonicError::RuneVisualizerFeedUnsupported
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
    registration_failure(kind, detail)
}

/// Registers the library of a streaming service that signs in on this device, once the service
/// accepts the platform's sign-in. The sign-in stays in memory: neither the record nor the
/// credential store gets it. The service has one account on this device, so registering again
/// signs the library registered before in.
pub(crate) async fn register_service_library_on_node(
    request: &RegisterServiceLibraryRequest,
    node: &Arc<NodeContext>,
) -> Result<RegisterLibraryResponse> {
    let protocol = ProtocolAdapter::from(request.service);
    let Some(service) = protocol.service() else {
        return Err(registration_failure(
            RegisterLibraryFailureKind::InvalidSource,
            "The record is not a streaming service's",
        ));
    };
    // A sign-out of the library registered already that comes while its service checks the
    // sign-in stands (see `sign_in_service_record`).
    let registered = LibraryRegistry::load(Path::new(&*node.config_path))
        .ok()
        .and_then(|registry| {
            registry
                .libraries
                .iter()
                .find(|record| record.protocol.service() == Some(service))
                .map(|record| record.record_id)
        })
        .map(|record_id| (record_id, node.services.sign_outs(record_id)));
    let sign_in = verified_service_sign_in(node, service, &request.sign_in).await?;
    let new_record = LibraryRecord {
        record_id: LibraryRecordId(Uuid::new_v4()),
        alias: request.alias.clone(),
        library_id: None,
        protocol,
        fs: Vec::new(),
        catalog: CatalogPlacement::None,
        background_sync: false,
    };
    // No other device may reach the library, from the moment its record exists.
    if service.signs_in_on_this_device() {
        node.access_policy
            .keep_on_this_device(new_record.record_id.0);
    }
    let (record, created) = LibraryRegistry::update(Path::new(&*node.config_path), |registry| {
        if let Some(existing) = registry
            .libraries
            .iter()
            .find(|record| record.protocol.service() == Some(service))
        {
            return Ok((existing.clone(), false));
        }
        registry.libraries.push(new_record.clone());
        Ok((new_record, true))
    })
    .await
    .map_err(|error| {
        registration_failure(
            RegisterLibraryFailureKind::Storage,
            format!("Could not write library storage: {error:#}"),
        )
    })?;
    let sign_outs = registered
        .filter(|(record_id, _)| !created && *record_id == record.record_id)
        .map(|(_, sign_outs)| sign_outs);
    let signed_in = sign_in_service_record(node, record.record_id, sign_in, sign_outs).await;
    // A record written here changes the list even when its sign-in then fails. A sign-in that
    // worked has told the list itself.
    if created
        && signed_in.is_err()
        && let Err(error) = publish_library_records_changed(node).await
    {
        log::warn!("Could not publish library record change: {error:#}");
    }
    signed_in?;
    Ok(RegisterLibraryResponse {
        call_id: None,
        record: Some((&record).into()),
        outcome: RegisterLibraryOutcome::Registered,
    })
}

/// The service of the registered library of `record_id`, when its sign-in lives on this device.
/// Failures are typed like a failed registration.
fn service_signing_in_here(
    node: &NodeContext,
    record_id: &str,
) -> Result<(LibraryRecordId, ServiceKind)> {
    use RegisterLibraryFailureKind as Kind;
    let Ok(record_id) = Uuid::parse_str(record_id).map(LibraryRecordId) else {
        return Err(registration_failure(
            Kind::InvalidRecord,
            "The record id is invalid",
        ));
    };
    let registry = LibraryRegistry::load(Path::new(&*node.config_path)).map_err(|error| {
        registration_failure(
            Kind::Storage,
            format!("Could not read library storage: {error:#}"),
        )
    })?;
    match registry
        .record(record_id)
        .ok()
        .and_then(|record| record.protocol.service())
    {
        Some(service) if service.signs_in_on_this_device() => Ok((record_id, service)),
        _ => Err(registration_failure(
            Kind::InvalidRecord,
            "The record is not a registered library that signs in on this device",
        )),
    }
}

/// Signs the registered service library of the request's record in. Failures are typed like
/// a failed registration.
pub(crate) async fn sign_in_service_library_on_node(
    request: &SignInServiceLibraryRequest,
    node: &Arc<NodeContext>,
) -> Result<()> {
    let (record_id, service) = service_signing_in_here(node, &request.record_id)?;
    // A sign-out that comes while the service checks the sign-in stands.
    let sign_outs = node.services.sign_outs(record_id);
    let sign_in = verified_service_sign_in(node, service, &request.sign_in).await?;
    sign_in_service_record(node, record_id, sign_in, Some(sign_outs)).await
}

/// Forgets the sign-in of the registered service library of the request's record. A library
/// without a binding has no sign-in to forget.
pub(crate) async fn sign_out_service_library_on_node(
    request: &SignOutServiceLibraryRequest,
    node: &Arc<NodeContext>,
) -> Result<()> {
    let (record_id, _) = service_signing_in_here(node, &request.record_id)?;
    node.services.sign_out(record_id);
    node.item_capabilities_changed();
    publish_records_changed(node).await;
    Ok(())
}

/// Tells the UI the list of records changed. The change itself is done, so a failure only
/// leaves the UI to catch up with the next refresh.
async fn publish_records_changed(node: &NodeContext) {
    if let Err(error) = publish_library_records_changed(node).await {
        log::warn!("Could not publish library record change: {error:#}");
    }
}

/// Signs the service library of `record_id` in with a sign-in its service accepted, unless it
/// was signed out since `sign_outs` were taken (see `ServiceRegistry::sign_in_since`). A removal
/// writes the registry before it retires the binding, so the record is looked up again
/// afterwards: a binding made for a record removed meanwhile must not stay signed in.
async fn sign_in_service_record(
    node: &NodeContext,
    record_id: LibraryRecordId,
    sign_in: VerifiedSignIn,
    sign_outs: Option<u64>,
) -> Result<()> {
    let binding = match sign_outs {
        Some(sign_outs) => node
            .services
            .sign_in_since(record_id, sign_in, sign_outs)
            .ok_or_else(|| {
                registration_failure(
                    RegisterLibraryFailureKind::InvalidRecord,
                    "The library was signed out during the sign-in",
                )
            })?,
        None => node.services.sign_in(record_id, sign_in),
    };
    node.item_capabilities_changed();
    // A registry that cannot be read proves nothing, so the binding stays as it is.
    let registry = LibraryRegistry::load(Path::new(&*node.config_path)).map_err(|error| {
        registration_failure(
            RegisterLibraryFailureKind::Storage,
            format!("Could not read library storage: {error:#}"),
        )
    })?;
    if registry.record(record_id).is_ok() {
        // The session may hold tracks of the library from before the sign-in, as after a
        // restart. Now they can be named. Those the binding does not hold are read together
        // first, off the session's event loop.
        if let Ok(session) = node.session() {
            tokio::spawn(async move {
                let tracks = session.queued_tracks(record_id.0).await;
                binding.read_tracks(tracks).await;
                session.rename_queue();
            });
        }
        publish_records_changed(node).await;
        return Ok(());
    }
    node.services.retire(record_id).await;
    node.item_capabilities_changed();
    Err(registration_failure(
        RegisterLibraryFailureKind::InvalidRecord,
        "The record was removed during the sign-in",
    ))
}

/// A platform sign-in to `service`, once the service accepted it.
async fn verified_service_sign_in(
    node: &NodeContext,
    service: ServiceKind,
    sign_in: &ServiceSignIn,
) -> Result<VerifiedSignIn> {
    node.services
        .verify_sign_in(service, sign_in)
        .await
        .map_err(|failure| {
            let (kind, detail) = failure.registration_failure();
            registration_failure(kind, detail)
        })
}

async fn open_library_by_record(
    node_slot: &NodeSlot,
    request: &OpenLibraryRequest,
    scrobbler: Arc<Mutex<ScrobblingManager>>,
) -> Result<OpenLibraryResponse> {
    let node = node_slot
        .current()
        .await
        .map_err(|error| open_failure(OpenLibraryFailureKind::Storage, error))?;
    open_library_by_record_on_node(request, node, scrobbler).await
}

/// A library that could not be opened, of `kind`; the error goes to the logs.
fn open_failure(kind: OpenLibraryFailureKind, error: anyhow::Error) -> anyhow::Error {
    Error::new(ExpectedRequestError::LibraryOpenFailed { kind }).context(format!("{error:#}"))
}

pub(crate) async fn open_library_by_record_on_node(
    request: &OpenLibraryRequest,
    node: Arc<NodeContext>,
    scrobbler: Arc<Mutex<ScrobblingManager>>,
) -> Result<OpenLibraryResponse> {
    let failed = |kind, error: anyhow::Error| Err(open_failure(kind, error));
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
            mode,
        } => {
            open_subsonic_record(
                &node,
                record.record_id,
                record.catalog_library_id().unwrap_or(record.record_id.0),
                endpoint,
                *credential,
                *mode == SubsonicOpenMode::PlayOnThisDevice,
            )
            .await
        }
        protocol @ ProtocolAdapter::AppleMusic => {
            open_held_service_record(&node, record.record_id, protocol).await
        }
    };
    match result {
        Ok(generation) => {
            if let Err(error) = LibraryRegistry::update(Path::new(&*node.config_path), |registry| {
                registry.last_open_record = Some(record_id);
                Ok(())
            })
            .await
            {
                // An open that its selection does not survive is not the open it reports, so
                // the binding this one installed closes with it, matching the reported failure.
                if let Ok(params) = node.current_ui().await {
                    node.close_ui(&params.main_token).await;
                }
                return failed(
                    OpenLibraryFailureKind::Storage,
                    error.context("Could not save the library as the last selection"),
                );
            }
            Ok(OpenLibraryResponse {
                call_id: request.call_id,
                record_id: request.record_id.clone(),
                generation,
            })
        }
        Err((kind, error)) => failed(kind, error),
    }
}

async fn open_subsonic_record(
    node: &Arc<NodeContext>,
    record_id: LibraryRecordId,
    library_id: Uuid,
    endpoint: &str,
    credential_ref: CredentialRef,
    play_on_this_device: bool,
) -> StdResult<u64, (OpenLibraryFailureKind, anyhow::Error)> {
    let read_credential = || async {
        node.credentials
            .get(credential_ref)
            .await
            .map_err(|error| (OpenLibraryFailureKind::Storage, Error::new(error)))
    };
    let endpoint = Url::parse(endpoint)
        .map_err(|error| (OpenLibraryFailureKind::RemoteProtocol, error.into()))?;
    let http = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| {
            (
                OpenLibraryFailureKind::Storage,
                anyhow::anyhow!("Could not create Subsonic client"),
            )
        })?;
    let config_dir = Path::new(&*node.config_path);
    let mut credential = read_credential().await?;
    let (client, kept_probe, result, commit) = loop {
        let password = std::str::from_utf8(credential.secret.as_bytes())
            .map_err(|error| (OpenLibraryFailureKind::Storage, error.into()))?;
        let client = SubsonicClient::new(
            endpoint.clone(),
            SubsonicCredentials::Password {
                user: credential.username.clone(),
                password: password.to_owned(),
            },
            http.clone(),
        );
        // What the last check of the server found for this account, when the library was opened
        // before with it. It opens from that and from its kept lists without waiting for the
        // server, which it checks in the background. Opened for the first time, it keeps nothing
        // to show, so it waits for the probe as before, and fails as before when the server
        // cannot be reached or rejects the sign-in.
        let kept_probe = SubsonicBinding::kept_probe(config_dir, library_id, &credential.username);
        let result = match &kept_probe {
            Some(probe) => Ok(Ok(probe.clone())),
            None => timeout(Duration::from_secs(10), client.probe()).await,
        };
        let commit = node.subsonic_credential_commit.lock().await;
        let current = read_credential().await?;
        if credential.username == current.username
            && credential.secret.as_bytes() == current.secret.as_bytes()
        {
            break (client, kept_probe, result, commit);
        }
        // Re-login won while the probe was in flight. Both the client and
        // the roles returned by getUser must come from the saved credential.
        credential = current;
    };
    let probe = match result {
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
    let binding = Arc::new(SubsonicBinding::new(
        client,
        probe,
        config_dir.to_path_buf(),
        library_id,
        play_on_this_device,
        Arc::clone(&node.gui_broadcaster),
        Arc::clone(node.visualizer()),
    ));
    if kept_probe.is_none() {
        binding.keep_probe();
    }
    // Register the cover cache so it can be cancelled when the library is removed
    node.register_cover_cache(library_id, &binding.covers);
    // Like the local and remote paths, the opened library gets its own UI and GUI
    // listeners; attaching it releases the previous library's.
    let mut params = node
        .params_for_ui()
        .map_err(|error| (OpenLibraryFailureKind::Storage, error))?;
    params.library_id = Some(library_id);
    let params = Arc::new(params);
    node.attach_ui(&params).await;
    let generation = node
        .set_open_service_catalog(
            Some(record_id),
            Arc::clone(&binding) as Arc<dyn ServiceBinding>,
        )
        .await
        .map_err(|error| (OpenLibraryFailureKind::Storage, error))?;
    drop(commit);
    listen_local_gui(&params, &params.main_token);
    if kept_probe.is_some() {
        check_subsonic_server(node, &binding);
    }
    Ok(generation)
}

/// Checks the server of a Subsonic library opened from what it kept, in the background. The
/// probe the server gives replaces the kept one, and the UI hears the request slots it gives. A
/// server that rejects the sign-in asks the UI for a new one, as a request it rejects does. A
/// server that cannot be reached leaves the library as it is: the kept lists still show, and the
/// requests that need the server fail as they do while it is down.
fn check_subsonic_server(node: &Arc<NodeContext>, binding: &Arc<SubsonicBinding>) {
    let node = Arc::downgrade(node);
    let client = Arc::clone(&binding.client);
    let check_over = binding.check_over();
    let binding = Arc::downgrade(binding);
    tokio::spawn(async move {
        let probed = tokio::select! {
            // A sign-in renewed meanwhile is the one the library keeps now, and what the server
            // answers to the one before says nothing of it. A library closed meanwhile takes
            // nothing from it.
            _ = check_over.cancelled() => return,
            probed = timeout(Duration::from_secs(10), client.probe()) => probed,
        };
        let Some(binding) = binding.upgrade() else {
            return;
        };
        let Ok(probed) = probed else {
            log::info!("The Subsonic server did not answer its check in time");
            return;
        };
        if binding.take_check(probed)
            && let Some(node) = node.upgrade()
            && let Err(error) = node.publish_capabilities()
        {
            log::warn!("Could not publish the capabilities the server gives: {error:#}");
        }
    });
}

/// Opens the service library of `record_id`, whose sign-in lives in its binding, with the
/// binding this device holds for it: the one its sign-in made, or else one made now, which
/// answers from what it kept on disk until the platform signs it in, so the library opens
/// offline too. A removal writes the registry before it retires the binding, so the record is
/// looked up again once the binding is held: a binding made for a record removed meanwhile must
/// not stay.
async fn open_held_service_record(
    node: &Arc<NodeContext>,
    record_id: LibraryRecordId,
    protocol: &ProtocolAdapter,
) -> StdResult<u64, (OpenLibraryFailureKind, anyhow::Error)> {
    let binding = protocol
        .service()
        .and_then(|service| node.services.hold(record_id, service));
    let Some(binding) = binding else {
        return Err((
            OpenLibraryFailureKind::NotSupportedYet,
            anyhow::anyhow!("The service's library is not opened by its binding"),
        ));
    };
    // A registry that cannot be read proves nothing, so the binding stays as it is.
    if let Ok(registry) = LibraryRegistry::load(Path::new(&*node.config_path))
        && registry.record(record_id).is_err()
    {
        node.services.retire(record_id).await;
        node.item_capabilities_changed();
        return Err((
            OpenLibraryFailureKind::RecordNotFound,
            anyhow::anyhow!("The record was removed while it opened"),
        ));
    }
    node.open_service_catalog(Some(record_id), binding)
        .await
        .map_err(|error| (OpenLibraryFailureKind::Storage, error))
}

/// The binding of the service library `library_id`: the open one, the one the node keeps, or one
/// made from its record for this use.
pub(crate) async fn service_binding_for_library(
    node: &NodeContext,
    library_id: Uuid,
) -> Result<Arc<dyn ServiceBinding>> {
    if let Some(binding) = held_service_binding(node, library_id)? {
        return Ok(binding);
    }
    let registry = LibraryRegistry::load(Path::new(&*node.config_path))?;
    let record = registry
        .record(LibraryRecordId(library_id))
        .map_err(|_| Error::new(RemoteAudioError::LibraryUnavailable))?;
    // A library whose sign-in lives in its binding, neither opened nor signed in since the app
    // started, has none: a request about one of its items waits for the sign-in the platform
    // makes, a typed failure the UI takes as such, never an internal one.
    if record
        .protocol
        .service()
        .is_some_and(ServiceKind::signs_in_on_this_device)
    {
        return Err(ExpectedRequestError::ServiceSignInRequired {
            record_id: record.record_id.0.to_string(),
        }
        .into());
    }
    match &record.protocol {
        ProtocolAdapter::SubsonicTranslate {
            endpoint,
            credential,
            mode,
        } => Ok(
            subsonic_binding_from_record(node, library_id, endpoint, *credential, *mode).await?,
        ),
        ProtocolAdapter::Native
        | ProtocolAdapter::RuneForward { .. }
        | ProtocolAdapter::AppleMusic => Err(Error::new(RemoteAudioError::LibraryUnavailable)),
    }
}

/// The library records of this node, as they are on disk now.
pub(crate) fn load_registry(node: &NodeContext) -> Result<LibraryRegistry> {
    LibraryRegistry::load(Path::new(&*node.config_path))
}

/// The binding this device holds for the service library `library_id`: the open one, or the one
/// the node keeps, signed in or not yet. None when it holds none, as for a library neither
/// opened nor signed in since the app started.
pub(crate) fn held_service_binding(
    node: &NodeContext,
    library_id: Uuid,
) -> Result<Option<Arc<dyn ServiceBinding>>> {
    Ok(node
        .service_binding(library_id)?
        .or_else(|| node.services.held(LibraryRecordId(library_id))))
}

/// A new binding for the Subsonic library `library_id`, made from its record's fields.
async fn subsonic_binding_from_record(
    node: &NodeContext,
    library_id: Uuid,
    endpoint: &str,
    credential: CredentialRef,
    mode: SubsonicOpenMode,
) -> Result<Arc<SubsonicBinding>> {
    let credential = node
        .credentials
        .get(credential)
        .await
        .map_err(|_| Error::new(RemoteAudioError::CredentialsRejected))?;
    let password = String::from_utf8(credential.secret.as_bytes().to_vec())
        .map_err(|_| Error::new(RemoteAudioError::CredentialsRejected))?;
    let endpoint =
        Url::parse(endpoint).map_err(|_| Error::new(RemoteAudioError::LibraryUnavailable))?;
    let client = SubsonicClient::new(
        endpoint,
        SubsonicCredentials::Password {
            user: credential.username,
            password,
        },
        Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| Error::new(RemoteAudioError::LibraryUnavailable))?,
    );
    let probe = tokio_time::timeout(Duration::from_secs(10), client.probe())
        .await
        .map_err(|_| Error::new(RemoteAudioError::unreachable("Subsonic server timeout")))?
        .map_err(|error| match error {
            SubsonicError::Transport(_) | SubsonicError::HttpStatus(_) => {
                Error::new(RemoteAudioError::unreachable("Subsonic server unreachable"))
            }
            SubsonicError::Server {
                kind:
                    SubsonicServerError::WrongCredentials
                    | SubsonicServerError::NotAuthorized
                    | SubsonicServerError::InvalidApiKey,
                ..
            } => Error::new(RemoteAudioError::CredentialsRejected),
            _ => Error::new(RemoteAudioError::LibraryUnavailable),
        })?;
    let binding = Arc::new(SubsonicBinding::new(
        client,
        probe,
        Path::new(&*node.config_path).to_path_buf(),
        library_id,
        mode == SubsonicOpenMode::PlayOnThisDevice,
        Arc::clone(&node.gui_broadcaster),
        Arc::clone(node.visualizer()),
    ));
    // Register the cover cache so it can be cancelled when the library is removed
    node.register_cover_cache(library_id, &binding.covers);
    Ok(binding)
}

pub(crate) async fn playback_implementation_for_library(
    node: &NodeContext,
    library: &LibraryHandle,
) -> Result<Implementation> {
    if library.descriptor.protocol.service().is_some() {
        Ok(Implementation::Service(
            service_binding_for_library(node, library.library_id).await?,
        ))
    } else {
        Ok(Implementation::Local)
    }
}

pub(crate) async fn playback_implementation_for_item(
    node: &NodeContext,
    library_id: Uuid,
) -> Result<Implementation> {
    if let Some(binding) = node.service_binding(library_id)? {
        return Ok(Implementation::Service(binding));
    }
    let registry = LibraryRegistry::load(Path::new(&*node.config_path))?;
    if let Ok(record) = registry.record(LibraryRecordId(library_id))
        && record.protocol.service().is_some()
    {
        return Ok(Implementation::Service(
            service_binding_for_library(node, library_id).await?,
        ));
    }
    let library = node.library_pool.acquire(library_id).await?;
    playback_implementation_for_library(node, &library).await
}

pub(crate) async fn library_item_metadata(
    node: &NodeContext,
    item: &PlayingItem,
    with_cover: bool,
) -> Result<(PlayingItemMetadataSummary, Option<String>)> {
    let PlayingItem::LibraryTrack { library_id, .. } = item else {
        anyhow::bail!("Item does not belong to a library");
    };
    let library = node.library_pool.acquire(*library_id).await?;
    let implementation = playback_implementation_for_library(node, &library).await?;
    implementation
        .item_metadata(item, with_cover, &library)
        .await
}

pub(crate) async fn record_library_item_played(
    node: &NodeContext,
    item: &PlayingItem,
) -> Result<()> {
    let PlayingItem::LibraryTrack { library_id, .. } = item else {
        return Ok(());
    };
    // A service counts the plays of its own tracks, which have no local statistics, and its
    // library has no catalog on this device to look them up in.
    let registry = LibraryRegistry::load(Path::new(&*node.config_path))?;
    if registry
        .record(LibraryRecordId(*library_id))
        .is_ok_and(|record| record.protocol.service().is_some())
    {
        return Ok(());
    }
    let library = node.library_pool.acquire(*library_id).await?;
    let implementation = playback_implementation_for_library(node, &library).await?;
    implementation.record_played(item, &library).await
}

pub(crate) async fn report_item_played_through(
    node: &NodeContext,
    item: &PlayingItem,
) -> Result<()> {
    let PlayingItem::LibraryTrack { library_id, .. } = item else {
        return Ok(());
    };
    let library = node.library_pool.acquire(*library_id).await?;
    let implementation = playback_implementation_for_library(node, &library).await?;
    implementation.track_played_through(item).await;
    Ok(())
}

async fn open_catalog_record(
    node: &Arc<NodeContext>,
    record: &LibraryRecord,
    library_id: Uuid,
    path: Option<&str>,
    scrobbler: Arc<Mutex<ScrobblingManager>>,
) -> result::Result<u64, (OpenLibraryFailureKind, anyhow::Error)> {
    if let Some(path) = path {
        #[cfg(not(target_os = "android"))]
        let fsio = FsIo::new();
        #[cfg(target_os = "android")]
        let fsio = FsIo::new(Path::new(".rune/.android-fs.db"), path)
            .map_err(|error| (OpenLibraryFailureKind::Storage, error.into()))?;
        if let Some(missing) = MediaFolderMissing::of(record) {
            return Err((OpenLibraryFailureKind::MediaFolderMissing, missing.into()));
        }
        check_library_state(&fsio, path)
            .map_err(|error| (OpenLibraryFailureKind::Storage, error))?;
    }
    let opened = node
        .library_pool
        .acquire_ui(library_id)
        .await
        .map_err(|error| (local_open_failure(&error), error))?;
    let generation = run_local_open(Ok(opened), Some(record.record_id), node, &scrobbler)
        .await
        .map_err(|error| (OpenLibraryFailureKind::Storage, error))?;
    Ok(generation)
}

/// Why the pool did not open a library on this device.
pub(crate) fn local_open_failure(error: &anyhow::Error) -> OpenLibraryFailureKind {
    if error.downcast_ref::<MediaFolderMissing>().is_some() {
        OpenLibraryFailureKind::MediaFolderMissing
    } else if error.downcast_ref::<NewerLibraryDatabase>().is_some() {
        OpenLibraryFailureKind::NewerVersion
    } else {
        OpenLibraryFailureKind::Storage
    }
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
    let library_id = library.library_id;
    info!("Resolved library id: {library_id}");
    local_player_loop(library, node.clone(), Arc::clone(scrobbler)).await?;
    node.set_open_local_catalog(record_id, library_id).await
}

pub struct CollectionCoverOptions<'a> {
    pub bake: bool,
    pub library_id: uuid::Uuid,
    pub session: Option<&'a crate::Session>,
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

    // Process cover art paths for this request's caller.
    let cover_art_map =
        process_cover_art_map(raw_cover_art_map, options.library_id, options.session);

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
    session: Option<&crate::Session>,
) -> HashMap<i32, String> {
    cover_art_map
        .into_iter()
        .map(|(id, path)| (id, process_cover_art_path(&path, library_id, session)))
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

    cmp::min(cmp::max(batch_size, min_batch_size), max_batch_size)
}

pub fn process_cover_art_path(
    path: &str,
    library_id: uuid::Uuid,
    session: Option<&crate::Session>,
) -> String {
    let Some(session) = session else {
        return path.to_string();
    };
    if path.is_empty() {
        return path.to_string();
    }

    let file_name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    format!(
        "{}/files/cache/{file_name}?library_id={library_id}",
        session.base_url.as_str().trim_end_matches('/')
    )
}

pub async fn parse_media_files(
    fsio: &FsIo,
    db: &MainDbConnection,
    _node_id: &str,
    library_id: &str,
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
        // Canonicalizing asks the library's storage, which blocks on a remote
        // library, so it runs off the runtime.
        #[cfg(not(target_os = "android"))]
        let media_path = {
            let fsio = fsio.clone();
            let path = Path::new(lib_path.as_ref())
                .join(&file.directory)
                .join(&file.file_name);
            task::spawn_blocking(move || fsio.canonicalize_path(&path))
                .await
                .context("media path task failed")?
        };

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
            library_id: library_id.to_owned(),
            track: file.file_uuid.to_string(),
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
            cover_art_id: file.cover_art_id,
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
    let local_paths = task::spawn_blocking(move || {
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
                PlayingItem::LibraryTrack { library_id, track } => PlayingItem::LibraryTrack {
                    library_id: *library_id,
                    track: track.clone(),
                },
                PlayingItem::IndependentFile(path) => PlayingItem::IndependentFile(path.clone()),
                _ => PlayingItem::LibraryTrack {
                    library_id: library_uuid,
                    track: file.file_uuid?.to_string(),
                },
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

/// The most tracks a mix resolves to for playback.
pub const RESOLVED_TRACK_LIMIT: usize = 4096;

/// The items a resolved mix plays and the index playback starts at. A negative
/// `hint_position` starts at the first item. Otherwise playback starts at the
/// occurrence of `initial` nearest to `hint_position`; when `items` does not hold it,
/// the `fallback` items play from `hint_position` instead.
pub fn resolved_start<T: PartialEq>(
    hint_position: i32,
    items: Vec<T>,
    initial: Option<&T>,
    fallback: Vec<T>,
) -> (Vec<T>, usize) {
    let Ok(hint_position) = usize::try_from(hint_position) else {
        return (items, 0);
    };
    match find_nearest_index(&items, hint_position, |item| initial == Some(item)) {
        Some(start) => (items, start),
        None => (fallback, hint_position),
    }
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
    use super::GlobalParams;
    use crate::library_pool::LibraryHandle;

    impl GlobalParams {
        /// The library a test fixture opened. Fixtures that build `GlobalParams`
        /// for a local library always set it, so a missing one is a broken fixture.
        pub fn test_library(&self) -> &LibraryHandle {
            self.library
                .as_ref()
                .expect("test fixture opened no library")
        }
    }

    use std::{sync::Arc, time::Duration};

    use crate::server::utils::device::RunMode;
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
        let node = node_slot
            .initialize(&path, &path, Arc::new(Quiet), RunMode::App)
            .await?;
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
        node.set_open_local_catalog(None, library_id).await?;

        let open_response = handlers
            .handle_get_sync_settings_request(&node_slot, &request)
            .await?
            .expect("sync settings response with a UI library");
        assert_eq!(open_response.request_id, closed_response.request_id);
        assert_eq!(open_response.media_server, closed_response.media_server);
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

        node.close_ui(&params.main_token).await;
        node.shutdown().await?;
        Ok(())
    }
}

#[cfg(test)]
mod local_open_failure_tests {
    use super::local_open_failure;
    use crate::{library_pool::MediaFolderMissing, messages::OpenLibraryFailureKind};
    use anyhow::Context;
    use anyhow::Error;

    #[test]
    fn a_missing_folder_is_told_apart_from_other_storage_failures() {
        let missing = Error::new(MediaFolderMissing("/music".into()));
        assert_eq!(
            local_open_failure(&missing),
            OpenLibraryFailureKind::MediaFolderMissing
        );
        let wrapped = Err::<(), _>(MediaFolderMissing("/music".into()))
            .context("Library acquisition failed")
            .unwrap_err();
        assert_eq!(
            local_open_failure(&wrapped),
            OpenLibraryFailureKind::MediaFolderMissing
        );
        assert_eq!(
            local_open_failure(&anyhow::anyhow!("database is locked")),
            OpenLibraryFailureKind::Storage
        );
    }
}

#[cfg(test)]
mod cover_art_url_tests {
    use super::process_cover_art_path;
    use crate::Session;
    use url::Url;
    use uuid::Uuid;

    #[test]
    fn remote_cover_url_names_its_library() {
        let library_id = Uuid::new_v4();
        let path = "/tmp/rune/cover_arts/cover-digest";
        let remote_session = Session {
            public_key: "key".to_string(),
            fingerprint: "fingerprint".to_string(),
            base_url: Url::parse("https://peer.example").unwrap(),
        };

        assert_eq!(process_cover_art_path(path, library_id, None), path);
        assert_eq!(
            process_cover_art_path(path, library_id, Some(&remote_session)),
            format!("https://peer.example/files/cache/cover-digest?library_id={library_id}")
        );
    }
}

#[cfg(test)]
mod resolved_start_tests {
    use super::resolved_start;

    #[test]
    fn playback_starts_at_the_clicked_item_nearest_the_hint() {
        assert_eq!(
            resolved_start(3, vec!["a", "b", "a", "c"], Some(&"a"), vec!["fallback"]),
            (vec!["a", "b", "a", "c"], 2)
        );
        assert_eq!(
            resolved_start(0, vec!["a", "b", "a", "c"], Some(&"a"), vec!["fallback"]),
            (vec!["a", "b", "a", "c"], 0)
        );
    }

    #[test]
    fn a_missing_clicked_item_plays_the_fallback_from_the_hint() {
        assert_eq!(
            resolved_start(1, vec!["a", "b"], Some(&"c"), vec!["x", "c"]),
            (vec!["x", "c"], 1)
        );
        assert_eq!(
            resolved_start(1, vec!["a", "b"], None, vec!["x", "y"]),
            (vec!["x", "y"], 1)
        );
    }

    #[test]
    fn a_negative_hint_starts_the_resolved_items_at_the_first() {
        assert_eq!(
            resolved_start(-1, vec!["a", "b"], Some(&"c"), vec!["x"]),
            (vec!["a", "b"], 0)
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

#[cfg(test)]
mod opendal_share_data_detection_tests {
    use super::{OpenDalShareDataDetection, classify_opendal_share_data};
    use crate::messages::RegisterLibraryFailureKind;
    use database::sync::model::RuneModel;
    use fsio::ObjectStoreCapabilities;
    use sync::{
        protocol::SyncModel,
        share::{FORMAT_VERSION, LibraryFile, ObjectTag, ObjectValue, StoreError, encode_library},
    };
    use uuid::Uuid;

    fn library_value(id: Uuid) -> ObjectValue {
        let bytes = encode_library(&LibraryFile {
            format: FORMAT_VERSION,
            library_id: id,
            model: RuneModel::model_id().to_owned(),
            schema: RuneModel::schema_id().to_owned(),
        })
        .unwrap();
        ObjectValue {
            bytes,
            tag: ObjectTag::new("tag"),
        }
    }

    #[test]
    fn detects_existing_share_library_id() {
        let id = Uuid::new_v4();
        assert_eq!(
            classify_opendal_share_data(
                Ok(library_value(id)),
                Ok(ObjectStoreCapabilities::default())
            ),
            Ok(OpenDalShareDataDetection::ExistingLibrary {
                library_id: id.to_string()
            }),
        );
    }

    #[test]
    fn detects_writable_empty_share() {
        let capabilities = ObjectStoreCapabilities {
            write_probe: true,
            ..ObjectStoreCapabilities::default()
        };
        assert_eq!(
            classify_opendal_share_data(Err(StoreError::NotFound), Ok(capabilities)),
            Ok(OpenDalShareDataDetection::WritableEmpty),
        );
    }

    #[test]
    fn detects_read_only_empty_share() {
        assert_eq!(
            classify_opendal_share_data(
                Err(StoreError::NotFound),
                Err(StoreError::PermissionDenied)
            ),
            Ok(OpenDalShareDataDetection::ReadOnly),
        );
    }

    #[test]
    fn detects_typed_share_probe_failure() {
        assert_eq!(
            classify_opendal_share_data(
                Err(StoreError::NotFound),
                Err(StoreError::Transient("offline".into()))
            ),
            Err(RegisterLibraryFailureKind::OpenDalUnreachable),
        );
    }
}

mod library_tasks;
pub use library_tasks::LibraryTaskOwner;
