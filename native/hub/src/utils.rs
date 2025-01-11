use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use dunce::canonicalize;
use log::{error, info};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use ::database::actions::{
    collection::CollectionQueryType, cover_art::bake_cover_art_by_media_files,
    metadata::MetadataSummary, mixes::query_mix_media_files,
};
use ::database::connection::{
    check_library_state, connect_main_db, connect_recommendation_db, create_redirect, LibraryState,
    MainDbConnection, RecommendationDbConnection,
};
use ::database::entities::media_files;
use ::database::playing_item::MediaFileHandle;
use ::playback::player::{Player, PlayingItem};
use ::playback::sfx_player::SfxPlayer;
use ::scrobbling::manager::ScrobblingManager;

use crate::messages::*;

#[cfg(target_os = "android")]
use tracing_logcat::{LogcatMakeWriter, LogcatTag};
#[cfg(target_os = "android")]
use tracing_subscriber::fmt::format::Format;
use tracing_subscriber::EnvFilter;

pub struct DatabaseConnections {
    pub main_db: Arc<MainDbConnection>,
    pub recommend_db: Arc<RecommendationDbConnection>,
}

async fn initialize_databases(path: &str, db_path: Option<&str>) -> Result<DatabaseConnections> {
    info!("Initializing databases");

    let main_db = connect_main_db(path, db_path)
        .await
        .with_context(|| "Failed to connect to main DB")?;

    let recommend_db = connect_recommendation_db(path, db_path)
        .with_context(|| "Failed to connect to recommendation DB")?;

    Ok(DatabaseConnections {
        main_db: Arc::new(main_db),
        recommend_db: Arc::new(recommend_db),
    })
}

#[derive(Default)]
pub struct TaskTokens {
    pub scan_token: Option<CancellationToken>,
    pub analyze_token: Option<CancellationToken>,
}

pub struct GlobalParams {
    pub lib_path: Arc<String>,
    pub main_db: Arc<MainDbConnection>,
    pub recommend_db: Arc<RecommendationDbConnection>,
    pub main_token: Arc<CancellationToken>,
    pub task_tokens: Arc<Mutex<TaskTokens>>,
    pub player: Arc<Mutex<Player>>,
    pub sfx_player: Arc<Mutex<SfxPlayer>>,
    pub scrobbler: Arc<Mutex<ScrobblingManager>>,
    pub broadcaster: Arc<dyn Broadcaster>,
}

pub trait ParamsExtractor {
    type Params;
    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params;
}

pub trait RinfRustSignal {
    fn send(&self);
}

#[macro_export]
macro_rules! impl_rinf_rust_signal {
    ($($t:ty),*) => {
        $(
            impl RinfRustSignal for $t {
                fn send(&self) {
                    self.send_signal_to_dart()
                }
            }
        )*
    };
}

pub trait Broadcaster: Send + Sync {
    fn broadcast(&self, message: &dyn RinfRustSignal);
}

pub struct LocalGuiBroadcaster;

impl Broadcaster for LocalGuiBroadcaster {
    fn broadcast(&self, message: &dyn RinfRustSignal) {
        message.send();
    }
}

impl_rinf_rust_signal!(SetMediaLibraryPathResponse);

pub async fn receive_media_library_path<F, Fut>(
    main_loop: F,
    scrobbler: Arc<Mutex<ScrobblingManager>>,
) -> Result<()>
where
    F: Fn(String, DatabaseConnections, Arc<Mutex<ScrobblingManager>>, Arc<dyn Broadcaster>) -> Fut
        + Send
        + Sync,
    Fut: std::future::Future<Output = ()> + Send,
{
    let receiver = SetMediaLibraryPathRequest::get_dart_signal_receiver();
    let broadcaster: Arc<dyn Broadcaster> = Arc::new(LocalGuiBroadcaster);

    loop {
        while let Some(dart_signal) = receiver.recv().await {
            let media_library_path = dart_signal.message.path;
            let database_path = dart_signal.message.db_path;
            let database_mode = dart_signal.message.mode;
            info!("Received path: {}", media_library_path);

            let library_test = match check_library_state(&media_library_path) {
                Ok(x) => x,
                Err(e) => {
                    broadcaster.broadcast(&SetMediaLibraryPathResponse {
                        path: media_library_path.clone(),
                        success: false,
                        error: Some(format!("{:#?}", e)),
                        not_ready: false,
                    });
                    continue;
                }
            };

            if database_mode.is_none() {
                match &library_test {
                    LibraryState::Uninitialized => {
                        broadcaster.broadcast(&SetMediaLibraryPathResponse {
                            path: media_library_path.clone(),
                            success: false,
                            error: None,
                            not_ready: true,
                        });
                        continue;
                    }
                    LibraryState::Initialized(_) => {}
                }
            }

            if let Some(mode) = database_mode {
                if mode == 1 {
                    if let Err(e) = create_redirect(&media_library_path) {
                        broadcaster.broadcast(&SetMediaLibraryPathResponse {
                            path: media_library_path.clone(),
                            success: false,
                            error: Some(format!("{:#?}", e)),
                            not_ready: false,
                        });
                        continue;
                    }
                }
            }

            // Initialize databases
            match initialize_databases(&media_library_path, Some(&database_path)).await {
                Ok(db_connections) => {
                    // Send success response to Dart
                    broadcaster.broadcast(&SetMediaLibraryPathResponse {
                        path: media_library_path.clone(),
                        success: true,
                        error: None,
                        not_ready: false,
                    });

                    // Clone the Arc for this iteration
                    let scrobbler_clone = Arc::clone(&scrobbler);

                    // Continue with main loop
                    main_loop(
                        media_library_path,
                        db_connections,
                        scrobbler_clone,
                        broadcaster.clone(),
                    )
                    .await;
                }
                Err(e) => {
                    error!("Database initialization failed: {:#?}", e);
                    // Send error response to Dart
                    broadcaster.broadcast(&SetMediaLibraryPathResponse {
                        path: media_library_path,
                        success: false,
                        error: Some(format!("{:#?}", e)),
                        not_ready: false,
                    });
                }
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}

pub async fn inject_cover_art_map(
    main_db: &MainDbConnection,
    recommend_db: Arc<RecommendationDbConnection>,
    collection: Collection,
    n: Option<i32>,
) -> Result<Collection> {
    let files = query_cover_arts(
        main_db,
        recommend_db,
        if collection.collection_type == i32::from(CollectionQueryType::Track) {
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
    let cover_art_map = bake_cover_art_by_media_files(main_db, files).await?;
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

pub async fn parse_media_files(
    media_summaries: Vec<MetadataSummary>,
    lib_path: Arc<String>,
) -> Result<Vec<MediaFile>> {
    let mut media_files = Vec::with_capacity(media_summaries.len());

    for file in media_summaries {
        let media_path = canonicalize(
            Path::new(lib_path.as_ref())
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
                error!("{:?}", e);
            }
        }
    }

    Ok(media_files)
}

pub fn files_to_playback_request(
    lib_path: &String,
    files: &[MediaFileHandle],
) -> Vec<(PlayingItem, PathBuf)> {
    files
        .iter()
        .filter_map(|file| {
            let file_path = match &file.item {
                PlayingItem::InLibrary(_) => Path::new(lib_path)
                    .join(&file.directory)
                    .join(&file.file_name),
                PlayingItem::IndependentFile(path_buf) => path_buf.to_path_buf(),
                PlayingItem::Unknown => Path::new("/").to_path_buf(),
            };

            match canonicalize(&file_path) {
                Ok(canonical_path) => Some((file.item.clone(), canonical_path)),
                Err(_) => None,
            }
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
