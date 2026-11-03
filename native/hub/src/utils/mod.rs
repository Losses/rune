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
    collections::HashMap,
    fmt::Debug,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use crate::node::{NodeContext, initialize_node};
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
    library_registry::{LibraryRegistry, resolve_library},
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
    remote::{RemoteConnectError, server_player_loop},
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

pub trait RinfRustSignal {
    fn name(&self) -> String;

    /// Encodes the message into a byte vector.
    fn encode_to_vec(&self) -> Result<Vec<u8>>;

    /// Sends the message to Dart.
    fn send_to_dart(&self);
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
        #[cfg(not(target_os = "android"))]
        let fsio = Arc::new(FsIo::new());
        #[cfg(target_os = "android")]
        let fsio = match FsIo::new(Path::new(".rune/.android-fs.db"), &dart_signal.message.path) {
            Ok(fsio) => Arc::new(fsio),
            Err(e) => {
                error!("Failed to initialize Android SAF filesystem: {e:#?}");
                broadcaster.broadcast(&SetMediaLibraryPathResponse {
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

                let library_test = match check_library_state(&fsio, media_library_path) {
                    Ok(x) => x,
                    Err(e) => {
                        error!("Failed to check library state: {e:#?}");
                        broadcaster.broadcast(&SetMediaLibraryPathResponse {
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
                    node.library_pool.acquire_ui(descriptor.library_id).await
                }
                .await;
                match opened {
                    Ok(library) => {
                        info!("Resolved library id: {}", library.descriptor.library_id);
                        let result = local_player_loop(library, node, Arc::clone(&scrobbler)).await;
                        broadcaster.broadcast(&SetMediaLibraryPathResponse {
                            error_kind: None,
                            path: media_library_path.to_string(),
                            success: result.is_ok(),
                            error: result.err().map(|error| format!("{error:#}")),
                            not_ready: false,
                        });
                    }
                    Err(e) => {
                        error!("Library initialization failed: {e:#?}");
                        broadcaster.broadcast(&SetMediaLibraryPathResponse {
                            error_kind: None,
                            path: media_library_path.to_string(),
                            success: false,
                            error: Some(format!("{e:#?}")),
                            not_ready: false,
                        });
                    }
                }
            }
            OperationDestination::Remote => {
                match server_player_loop(fsio, media_library_path, node, alias).await {
                    Ok(_) => {
                        broadcaster.broadcast(&SetMediaLibraryPathResponse {
                            error_kind: None,
                            path: media_library_path.to_string(),
                            success: true,
                            error: None,
                            not_ready: false,
                        });
                    }
                    Err(e) => {
                        error!("Failed to server player loop: {e:#?}");
                        broadcaster.broadcast(&SetMediaLibraryPathResponse {
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

pub async fn inject_cover_art_map(
    fsio: &FsIo,
    main_db: &MainDbConnection,
    recommend_db: Arc<RecommendationDbConnection>,
    collection: Collection,
    n: Option<i32>,
    running_mode: &RunningMode,
    remote_host: &Option<String>,
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
    )
    .await?;

    // Get the base cover art paths
    let raw_cover_art_map = bake_cover_art_by_media_files(fsio, main_db, files).await?;

    // Process the cover art paths based on the running mode
    let cover_art_map: HashMap<i32, String> = raw_cover_art_map
        .into_iter()
        .map(|(id, path)| (id, process_cover_art_path(&path, running_mode, remote_host)))
        .collect();

    Ok(Collection {
        id: collection.id,
        name: collection.name,
        queries: collection.queries,
        collection_type: collection.collection_type,
        cover_art_map,
        readonly: collection.readonly,
    })
}

pub async fn query_cover_arts(
    main_db: &MainDbConnection,
    recommend_db: Arc<RecommendationDbConnection>,
    queries: Vec<MixQuery>,
    n: Option<i32>,
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
                    format!("{host}/files/cache/{file_name}")
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
    _node_id: &str,
    lib_path: &P,
    files: &[MediaFileHandle],
) -> Result<Vec<(PlayingItem, PlaybackSource)>> {
    let ids: Vec<i32> = files
        .iter()
        .filter_map(|file| match file.item {
            PlayingItem::InLibrary(id) => Some(id),
            _ => None,
        })
        .collect();
    let availability = audio_availability(db, fsio, lib_path.as_ref(), &ids).await?;
    let remote_content = remote_content_ids(db, &ids).await?;
    Ok(files
        .iter()
        .filter_map(|file| {
            let source = match &file.item {
                PlayingItem::InLibrary(id)
                    if matches!(
                        availability.get(id),
                        Some(AudioAvailability::RemoteOnly)
                            | Some(AudioAvailability::ImportPending)
                    ) && remote_content.contains(id) =>
                {
                    PlaybackSource::LibraryStream
                }
                PlayingItem::InLibrary(id)
                    if !matches!(availability.get(id), Some(AudioAvailability::Local)) =>
                {
                    return None;
                }
                PlayingItem::InLibrary(_) => match fsio.canonicalize_path(
                    &lib_path
                        .as_ref()
                        .join(&file.directory)
                        .join(&file.file_name),
                ) {
                    Ok(path) => PlaybackSource::Local(path),
                    Err(_) => return None,
                },
                PlayingItem::IndependentFile(path) => {
                    PlaybackSource::Local(fsio.canonicalize_path_str(path).ok()?)
                }
                PlayingItem::Online(_, _) => PlaybackSource::Local(PathBuf::new()),
                PlayingItem::Unknown => return None,
            };
            Some((file.item.clone(), source))
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
