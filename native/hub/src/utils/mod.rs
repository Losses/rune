use database::sync::LibrarySyncStore;
use tokio::time::Duration;
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
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use crate::handlers::local_libraries::publish_library_records_changed;
use crate::node::{NodeContext, current_node, initialize_node};
use anyhow::{Context, Result};
use fsio::FsIo;
use log::{error, info};
use rinf::{DartSignal, DartSignalPack};
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
        check_storage_mode, connect_main_db, connect_recommendation_db, create_redirect,
    },
    entities::media_files,
    library_registry::{
        LibraryKind, LibraryLocation, LibraryRecord, LibraryRecordId, LibraryRegistry,
        OwnedStorage, ReplicaStorage, resolve_library,
    },
    playing_item::MediaFileHandle,
};
use ::discovery::{client::CertValidator, protocol::DiscoveryService, server::PermissionManager};
use ::playback::{
    player::{Playable, PlaybackSource, PlayingItem},
    sfx_player::SfxPlayer,
};
use ::scrobbling::manager::ScrobblingManager;

use crate::backends::{
    local::local_player_loop,
    remote::{RemoteConnectError, open_remote_library, open_remote_library_for_fingerprint},
};
use crate::library_pool::{LibraryHandle, LibraryPool};
use crate::messages::*;
use crate::request_error::ExpectedRequestError;
use crate::server::ServerManager;

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

#[derive(Debug, Default)]
pub struct TaskTokens {
    pub scan_token: Option<CancellationToken>,
    pub scan_id: Option<String>,
    pub analyze_token: Option<CancellationToken>,
    pub deduplicate_token: Option<CancellationToken>,
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

pub async fn receive_media_library_path(scrobbler: Arc<Mutex<ScrobblingManager>>) -> Result<()> {
    let receiver = SetMediaLibraryPathRequest::get_dart_signal_receiver();
    let broadcaster: Arc<dyn Broadcaster> = Arc::new(LocalGuiBroadcaster);

    info!("Receive media library path loop started");

    loop {
        while let Some(dart_signal) = receiver.recv().await {
            let path = dart_signal.message.path.clone();
            // A panic while handling one message must not wedge the whole rinf
            // channel (the process stays alive but every Dart signal goes
            // unanswered). Catch it, report, keep listening.
            let handling = std::panic::AssertUnwindSafe(handle_media_library_path(
                dart_signal,
                Arc::clone(&scrobbler),
                Arc::clone(&broadcaster),
            ));
            if let Err(panic) = futures::FutureExt::catch_unwind(handling).await {
                error!("Panicked while handling media library path {path}: {panic:?}");
                broadcaster.broadcast(&SetMediaLibraryPathResponse {
                    restore_last_index: false,
                    error_kind: None,
                    path,
                    success: false,
                    error: Some(
                        "Internal error while opening the library; details are in the logs"
                            .to_string(),
                    ),
                    not_ready: false,
                });
            }
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn handle_media_library_path(
    dart_signal: DartSignalPack<SetMediaLibraryPathRequest>,
    scrobbler: Arc<Mutex<ScrobblingManager>>,
    broadcaster: Arc<dyn Broadcaster>,
) {
    {
        info!("Received media library path message");
        let media_library_path = &dart_signal.message.path;

        info!("Received media library {media_library_path}");

        let config_path = &dart_signal.message.config_path;
        let alias = &dart_signal.message.alias;
        // A replica is named by its library id; it lives in the data directory, which can
        // move, so its folder comes from the registry rather than from Dart.
        let replica_root =
            match LibraryLocation::parse(media_library_path).and_then(|location| match location {
                LibraryLocation::Replica(id) => {
                    let registry = LibraryRegistry::load(Path::new(config_path))?;
                    let descriptor = registry.replica(id)?;
                    let root = descriptor
                        .root(Path::new(config_path))
                        .context("Replica has no storage root")?;
                    Ok(Some((
                        id,
                        descriptor.record_id,
                        root.to_str()
                            .context("Replica path is not UTF-8")?
                            .to_owned(),
                    )))
                }
                LibraryLocation::Path(_) => Ok(None),
            }) {
                Ok(replica) => replica,
                Err(error) => {
                    broadcaster.broadcast(&SetMediaLibraryPathResponse {
                        restore_last_index: false,
                        error_kind: None,
                        path: media_library_path.to_string(),
                        success: false,
                        error: Some(format!("{error:#}")),
                        not_ready: false,
                    });
                    return;
                }
            };
        #[cfg(not(target_os = "android"))]
        let fsio = Arc::new(FsIo::new());
        #[cfg(target_os = "android")]
        let fsio = match FsIo::new(
            Path::new(".rune/.android-fs.db"),
            replica_root
                .as_ref()
                .map_or(media_library_path, |(_, root)| root),
        ) {
            Ok(fsio) => Arc::new(fsio),
            Err(e) => {
                error!("Failed to initialize Android SAF filesystem: {e:#?}");
                broadcaster.broadcast(&SetMediaLibraryPathResponse {
                    restore_last_index: false,
                    error_kind: None,
                    path: media_library_path.to_string(),
                    success: false,
                    error: Some(format!("Failed to access the selected directory: {e:#?}")),
                    not_ready: false,
                });
                return;
            }
        };
        let node = match initialize_node(
            config_path,
            &dart_signal.message.db_path,
            broadcaster.clone(),
        )
        .await
        {
            Ok(node) => node,
            Err(error) => {
                broadcaster.broadcast(&SetMediaLibraryPathResponse {
                    restore_last_index: false,
                    error_kind: None,
                    path: media_library_path.to_string(),
                    success: false,
                    error: Some(format!("Failed to initialize node: {error:#}")),
                    not_ready: false,
                });
                return;
            }
        };

        match &dart_signal.message.hosted_on {
            OperationDestination::Local => {
                let database_mode = dart_signal.message.mode;
                info!("Received path: {media_library_path}");

                if let Some((id, record_id, _)) = replica_root {
                    let opened = node.library_pool.acquire_ui(id).await;
                    finish_local_open(
                        opened,
                        Some(record_id),
                        &node,
                        &scrobbler,
                        &broadcaster,
                        media_library_path,
                    )
                    .await;
                    return;
                }

                let library_test = match check_library_state(&fsio, media_library_path) {
                    Ok(x) => x,
                    Err(e) => {
                        error!("Failed to check library state: {e:#?}");
                        broadcaster.broadcast(&SetMediaLibraryPathResponse {
                            restore_last_index: false,
                            error_kind: None,
                            path: media_library_path.to_string(),
                            success: false,
                            error: Some(format!("{e:#?}")),
                            not_ready: false,
                        });
                        return;
                    }
                };

                if database_mode.is_none() {
                    match &library_test {
                        LibraryState::Uninitialized => {
                            broadcaster.broadcast(&SetMediaLibraryPathResponse {
                                restore_last_index: false,
                                error_kind: None,
                                path: media_library_path.to_string(),
                                success: true,
                                error: None,
                                not_ready: true,
                            });
                            return;
                        }
                        LibraryState::Initialized(_) => {}
                    }
                }

                if let Some(mode) = database_mode
                    && mode == LibraryInitializeMode::Redirected
                    && let Err(e) = create_redirect(&fsio, media_library_path).await
                {
                    broadcaster.broadcast(&SetMediaLibraryPathResponse {
                        restore_last_index: false,
                        error_kind: None,
                        path: media_library_path.to_string(),
                        success: false,
                        error: Some(format!("{e:#?}")),
                        not_ready: false,
                    });
                    return;
                }

                let opened = async {
                    let mode = check_storage_mode(&fsio, media_library_path)?;
                    let mut registry = LibraryRegistry::load(Path::new(config_path))?;
                    let descriptor =
                        resolve_library(&fsio, &mut registry, media_library_path, alias, mode)
                            .await?;
                    let record_id = descriptor.record_id;
                    let opened = node
                        .library_pool
                        .acquire_ui(
                            descriptor
                                .catalog_library_id()
                                .context("Resolved endpoint has no catalog")?,
                        )
                        .await?;
                    Ok::<_, anyhow::Error>((opened, Some(record_id)))
                }
                .await;
                match opened {
                    Ok((opened, record_id)) => {
                        finish_local_open(
                            Ok(opened),
                            record_id,
                            &node,
                            &scrobbler,
                            &broadcaster,
                            media_library_path,
                        )
                        .await
                    }
                    Err(error) => {
                        finish_local_open(
                            Err(error),
                            None,
                            &node,
                            &scrobbler,
                            &broadcaster,
                            media_library_path,
                        )
                        .await
                    }
                }
            }
            OperationDestination::Remote => {
                match open_remote_library(node, media_library_path).await {
                    Ok(_) => {
                        broadcaster.broadcast(&SetMediaLibraryPathResponse {
                            restore_last_index: false,
                            error_kind: None,
                            path: media_library_path.to_string(),
                            success: true,
                            error: None,
                            not_ready: false,
                        });
                    }
                    Err(e) => {
                        error!("Failed to open remote library: {e:#?}");
                        broadcaster.broadcast(&SetMediaLibraryPathResponse {
                            restore_last_index: false,
                            error_kind: Some(
                                e.downcast_ref::<RemoteConnectError>()
                                    .map_or(LibraryOpenErrorKind::RemoteProtocol, |error| {
                                        error.kind()
                                    }),
                            ),
                            path: media_library_path.to_string(),
                            success: false,
                            error: Some(format!("{e:#?}")),
                            not_ready: false,
                        });
                    }
                }
            }
        }
    }
}

/// Receive record-oriented registration and open requests independently of an open UI library.
pub async fn receive_library_record_requests(
    scrobbler: Arc<Mutex<ScrobblingManager>>,
) -> Result<()> {
    let local_receiver = RegisterLocalLibraryRequest::get_dart_signal_receiver();
    let remote_receiver = RegisterRemoteControlRequest::get_dart_signal_receiver();
    let open_receiver = OpenLibraryRequest::get_dart_signal_receiver();
    let broadcaster: Arc<dyn Broadcaster> = Arc::new(LocalGuiBroadcaster);
    loop {
        tokio::select! {
            Some(signal) = local_receiver.recv() => {
                let result = register_local_library(&signal.message).await;
                broadcaster.broadcast(&result);
            }
            Some(signal) = remote_receiver.recv() => {
                let result = register_remote_control(&signal.message).await;
                broadcaster.broadcast(&result);
            }
            Some(signal) = open_receiver.recv() => {
                let result = open_library_by_record(&signal.message, Arc::clone(&scrobbler), Arc::clone(&broadcaster)).await;
                broadcaster.broadcast(&result);
            }
            else => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
}

async fn register_local_library(request: &RegisterLocalLibraryRequest) -> RegisterLibraryResponse {
    let node = match current_node().await {
        Ok(node) => node,
        Err(error) => {
            return RegisterLibraryResponse {
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
            if let Err(error) = publish_library_records_changed(node) {
                log::warn!("Could not publish library record change: {error:#}");
            }
            RegisterLibraryResponse {
                record: Some((&record).into()),
                outcome: RegisterLibraryOutcome::Registered,
                detail: None,
            }
        }
        Err(error) => fail(RegisterLibraryFailureKind::Storage, error),
    }
}

async fn register_remote_control(
    request: &RegisterRemoteControlRequest,
) -> RegisterLibraryResponse {
    let node = match current_node().await {
        Ok(node) => node,
        Err(error) => {
            return RegisterLibraryResponse {
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
            if let Err(error) = publish_library_records_changed(node) {
                log::warn!("Could not publish library record change: {error:#}");
            }
            RegisterLibraryResponse {
                record: Some((&record).into()),
                outcome: RegisterLibraryOutcome::Registered,
                detail: None,
            }
        }
        Err(error) => RegisterLibraryResponse {
            record: None,
            outcome: RegisterLibraryOutcome::Failed(RegisterLibraryFailureKind::Storage),
            detail: Some(format!("{error:#}")),
        },
    }
}

async fn open_library_by_record(
    request: &OpenLibraryRequest,
    scrobbler: Arc<Mutex<ScrobblingManager>>,
    broadcaster: Arc<dyn Broadcaster>,
) -> OpenLibraryResponse {
    let node = match current_node().await {
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
    open_library_by_record_on_node(request, node, scrobbler, broadcaster).await
}

pub(crate) async fn open_library_by_record_on_node(
    request: &OpenLibraryRequest,
    node: Arc<NodeContext>,
    scrobbler: Arc<Mutex<ScrobblingManager>>,
    broadcaster: Arc<dyn Broadcaster>,
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
        Err(error) => return failed(OpenLibraryFailureKind::RecordNotFound, error),
    };
    let result = match &record.kind {
        LibraryKind::Owned {
            library_id,
            storage,
            ..
        } => {
            open_catalog_record(
                &node,
                &record,
                *library_id,
                storage_path_owned(storage),
                Arc::clone(&scrobbler),
                &broadcaster,
            )
            .await
        }
        LibraryKind::RuneReplica {
            library_id,
            storage,
            ..
        } => {
            let owned_path;
            let path = match storage {
                ReplicaStorage::Local { path, .. } => path.as_str(),
                ReplicaStorage::Replica => {
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
                }
            };
            open_catalog_record(
                &node,
                &record,
                *library_id,
                Some(path),
                Arc::clone(&scrobbler),
                &broadcaster,
            )
            .await
        }
        LibraryKind::RuneRemoteControl { fingerprint } => {
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
        LibraryKind::Subsonic { .. } => Err((
            OpenLibraryFailureKind::NotSupportedYet,
            anyhow::anyhow!("Subsonic record opening is not implemented yet"),
        )),
    };
    match result {
        Ok(generation) => OpenLibraryResponse {
            call_id: request.call_id,
            record_id: request.record_id.clone(),
            generation,
            result: OpenLibraryResult::Opened,
        },
        Err((kind, error)) => failed(kind, error),
    }
}

fn storage_path_owned(storage: &OwnedStorage) -> Option<&str> {
    match storage {
        OwnedStorage::Local { path, .. } => Some(path),
    }
}

async fn open_catalog_record(
    node: &Arc<NodeContext>,
    record: &LibraryRecord,
    library_id: Uuid,
    path: Option<&str>,
    scrobbler: Arc<Mutex<ScrobblingManager>>,
    broadcaster: &Arc<dyn Broadcaster>,
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
    broadcaster.broadcast(&SetMediaLibraryPathResponse {
        restore_last_index: node
            .session()
            .is_ok_and(|session| session.take_legacy_restore()),
        error_kind: None,
        path: record.app_name().unwrap_or_default(),
        success: true,
        error: None,
        not_ready: false,
    });
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

/// Run the local player loop for an opened library and report the result to Dart.
async fn finish_local_open(
    opened: Result<LibraryHandle>,
    record_id: Option<LibraryRecordId>,
    node: &Arc<NodeContext>,
    scrobbler: &Arc<Mutex<ScrobblingManager>>,
    broadcaster: &Arc<dyn Broadcaster>,
    media_library_path: &str,
) {
    let result = run_local_open(opened, record_id, node, scrobbler).await;
    match result {
        Ok(_) => broadcaster.broadcast(&SetMediaLibraryPathResponse {
            restore_last_index: node
                .session()
                .is_ok_and(|session| session.take_legacy_restore()),
            error_kind: None,
            path: media_library_path.to_string(),
            success: true,
            error: None,
            not_ready: false,
        }),
        Err(error) => {
            error!("Library initialization failed: {error:#?}");
            broadcaster.broadcast(&SetMediaLibraryPathResponse {
                restore_last_index: false,
                error_kind: None,
                path: media_library_path.to_string(),
                success: false,
                error: Some(format!("{error:#?}")),
                not_ready: false,
            });
        }
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
    Ok(files
        .iter()
        .filter_map(|file| {
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
                Some(_) => match fsio.canonicalize_path(
                    &lib_path
                        .as_ref()
                        .join(&file.directory)
                        .join(&file.file_name),
                ) {
                    Ok(path) => PlaybackSource::Local(path),
                    Err(_) => return None,
                },
                None if let PlayingItem::IndependentFile(path) = &file.item => {
                    PlaybackSource::Local(fsio.canonicalize_path_str(path).ok()?)
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
