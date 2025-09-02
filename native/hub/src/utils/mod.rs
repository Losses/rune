pub mod broadcastable;
pub mod nid;
pub mod player;

use std::{
    collections::HashMap,
    fmt::Debug,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use anyhow::{Context, Result};
use fsio::FsIo;
use log::{error, info};
use nid::get_or_create_node_id;
use rinf::DartSignal;
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
        connect_main_db, connect_recommendation_db, create_redirect,
    },
    entities::media_files,
    playing_item::MediaFileHandle,
};
use ::discovery::{client::CertValidator, protocol::DiscoveryService, server::PermissionManager};
use ::playback::{
    player::{Playable, PlayingItem},
    sfx_player::SfxPlayer,
};
use ::scrobbling::manager::ScrobblingManager;

use crate::backends::{local::local_player_loop, remote::server_player_loop};
use crate::messages::*;
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

    let recommend_db = connect_recommendation_db(fsio, path, db_path)
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
    pub analyze_token: Option<CancellationToken>,
    pub deduplicate_token: Option<CancellationToken>,
}

#[derive(Debug, Clone, Copy)]
pub enum RunningMode {
    Server,
    Client,
}

pub struct GlobalParams {
    pub fsio: Arc<FsIo>,
    pub lib_path: Arc<String>,
    pub config_path: Arc<String>,
    pub node_id: Arc<String>,
    pub main_db: Arc<MainDbConnection>,
    pub recommend_db: Arc<RecommendationDbConnection>,
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

impl Debug for GlobalParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlobalParams")
            .field("lib_path", &self.lib_path)
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
            info!("Received media library path message");
            let media_library_path = &dart_signal.message.path;

            info!("Received media library {media_library_path}");

            let config_path = &dart_signal.message.config_path;
            let alias = &dart_signal.message.alias;
            #[cfg(not(target_os = "android"))]
            let fsio = Arc::new(FsIo::new());
            #[cfg(target_os = "android")]
            let fsio = Arc::new(FsIo::new(
                Path::new(".rune/.android-fs.db"),
                &dart_signal.message.path,
            )?);
            let node_id = get_or_create_node_id(&fsio, config_path).await?.to_string();

            match &dart_signal.message.hosted_on {
                OperationDestination::Local => {
                    let database_path = dart_signal.message.db_path;
                    let database_mode = dart_signal.message.mode;
                    info!("Received path: {media_library_path}");

                    let library_test = match check_library_state(media_library_path) {
                        Ok(x) => x,
                        Err(e) => {
                            error!("Failed to check library state: {e:#?}");
                            broadcaster.broadcast(&SetMediaLibraryPathResponse {
                                path: media_library_path.to_string(),
                                success: false,
                                error: Some(format!("{e:#?}")),
                                not_ready: false,
                            });
                            continue;
                        }
                    };

                    if database_mode.is_none() {
                        match &library_test {
                            LibraryState::Uninitialized => {
                                broadcaster.broadcast(&SetMediaLibraryPathResponse {
                                    path: media_library_path.to_string(),
                                    success: true,
                                    error: None,
                                    not_ready: true,
                                });
                                continue;
                            }
                            LibraryState::Initialized(_) => {}
                        }
                    }

                    if let Some(mode) = database_mode
                        && mode == LibraryInitializeMode::Redirected
                        && let Err(e) = create_redirect(media_library_path)
                    {
                        broadcaster.broadcast(&SetMediaLibraryPathResponse {
                            path: media_library_path.to_string(),
                            success: false,
                            error: Some(format!("{e:#?}")),
                            not_ready: false,
                        });
                        continue;
                    }

                    // Initialize databases
                    match initialize_databases(
                        &fsio,
                        media_library_path,
                        Some(&database_path),
                        &node_id,
                    )
                    .await
                    {
                        Ok(db_connections) => {
                            // Send success response to Dart
                            broadcaster.broadcast(&SetMediaLibraryPathResponse {
                                path: media_library_path.to_string(),
                                success: true,
                                error: None,
                                not_ready: false,
                            });

                            // Clone the Arc for this iteration
                            let scrobbler_clone = Arc::clone(&scrobbler);

                            // Continue with main loop
                            local_player_loop(
                                fsio,
                                media_library_path.to_string(),
                                config_path.to_string(),
                                db_connections,
                                scrobbler_clone,
                                broadcaster.clone(),
                            )
                            .await;
                        }
                        Err(e) => {
                            error!("Database initialization failed: {e:#?}");
                            // Send error response to Dart
                            broadcaster.broadcast(&SetMediaLibraryPathResponse {
                                path: media_library_path.to_string(),
                                success: false,
                                error: Some(format!("{e:#?}")),
                                not_ready: false,
                            });
                        }
                    }
                }
                OperationDestination::Remote => {
                    let config_path = &dart_signal.message.config_path;
                    match server_player_loop(fsio, media_library_path, config_path, alias).await {
                        Ok(_) => {
                            broadcaster.broadcast(&SetMediaLibraryPathResponse {
                                path: media_library_path.to_string(),
                                success: true,
                                error: None,
                                not_ready: false,
                            });
                        }
                        Err(e) => {
                            error!("Failed to server player loop: {e:#?}");
                            broadcaster.broadcast(&SetMediaLibraryPathResponse {
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

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
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
    media_summaries: Vec<MetadataSummary>,
    lib_path: Arc<String>,
) -> Result<Vec<MediaFile>> {
    let mut media_files = Vec::with_capacity(media_summaries.len());

    for file in media_summaries {
        let media_path = fsio.canonicalize_path(
            &Path::new(lib_path.as_ref())
                .join(&file.directory)
                .join(&file.file_name),
        );

        match media_path {
            Ok(media_path) => {
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
                };

                media_files.push(media_file);
            }
            Err(e) => {
                error!("{e:?}");
            }
        }
    }

    Ok(media_files)
}

pub fn files_to_playback_request<P: AsRef<Path>>(
    fsio: &FsIo,
    lib_path: &P,
    files: &[MediaFileHandle],
) -> Vec<(PlayingItem, PathBuf)> {
    files
        .iter()
        .filter_map(|file| {
            let path_buf = match &file.item {
                PlayingItem::InLibrary(_) => match fsio.canonicalize_path(
                    &lib_path
                        .as_ref()
                        .join(&file.directory)
                        .join(&file.file_name),
                ) {
                    Ok(x) => x,
                    Err(_) => return None,
                },
                PlayingItem::IndependentFile(raw_path) => {
                    match fsio.canonicalize_path_str(raw_path) {
                        Ok(x) => x,
                        Err(_) => return None,
                    }
                }
                PlayingItem::Online(_, _) => PathBuf::new(),
                PlayingItem::Unknown => return None,
            };

            Some((file.item.clone(), path_buf))
        })
        .collect()
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
    let writer = LogcatMakeWriter::new(tag).expect("Failed to initialize logcat writer");

    tracing_subscriber::fmt()
        .event_format(Format::default().with_level(false).without_time())
        .with_writer(writer)
        .with_ansi(false)
        .init();
}
