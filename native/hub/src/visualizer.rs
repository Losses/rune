//! The process-wide visualizer and the covers its scenes read.
//!
//! Its frames reach the screen through [`crate::flutter_bridge`], which registers a texture with
//! the engine from Rust, so the pixels never travel through Dart.

use crate::{node::NodeContext, utils::playback_implementation_for_item};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use tokio::runtime::Handle;
use uuid::Uuid;

use ::database::{
    actions::{cover_art::bake_cover_art_by_media_files, mixes::query_mix_media_files},
    connection::{MainDbConnection, RecommendationDbConnection},
};
use ::fsio::FsIo;
use ::playback::player::PlayingItem;
pub use ::visualizer::ChartItem;
use anyhow::Result;
use log::warn;
use visualizer::Visualizer;

/// The app's cover wall is one of its scenes, and its errors go to the Log page.
pub static VISUALIZER: LazyLock<Visualizer> = LazyLock::new(|| {
    let visualizer = Visualizer::new();
    visualizer.offer_cover_wall();
    visualizer.set_error_listener(Some(Arc::new(|operation, message| {
        crate::visualizer_log::error(operation, message.to_string())
    })));
    visualizer
});

/// How many random covers a scene that reads covers gets besides the one of the track playing.
pub(crate) const RANDOM_COVERS: usize = 63;
const MAX_FEED_COVER_BYTES: usize = 4 * 1024 * 1024;
const MAX_FEED_BYTES: usize = 16 * 1024 * 1024;

/// What finding the covers for scenes needs.
struct CoverSource {
    runtime: tokio::runtime::Handle,
    fsio: Arc<FsIo>,
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    node: Arc<NodeContext>,
    library_id: Uuid,
    item: Arc<Mutex<Option<PlayingItem>>>,
}

static COVER_SOURCE: Mutex<Option<Arc<CoverSource>>> = Mutex::new(None);
/// The cover file of the track playing, once its cover is baked.
static PLAYING_COVER: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Let the visualizer's scenes read covers from this library: whenever a scene that asks for them
/// loads they are found and handed over, and whenever the playing track changes while one is on
/// screen, so is its cover. Call it within the runtime.
pub fn provide_covers(
    fsio: Arc<FsIo>,
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    node: Arc<NodeContext>,
    library_id: Uuid,
    item: Option<PlayingItem>,
) {
    *COVER_SOURCE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(CoverSource {
        runtime: Handle::current(),
        fsio,
        main_db,
        recommend_db,
        node,
        library_id,
        item: Arc::new(Mutex::new(item)),
    }));
    VISUALIZER.set_covers_requester(refresh_covers);
}

/// The player moved to another track, whose cover file is `cover` (none if it has none): a scene
/// that reads covers gets it in cell 0, the random covers stay.
pub fn playing_cover_changed(item: Option<&PlayingItem>, cover: Option<&str>) {
    let cover = cover.filter(|path| !path.is_empty()).map(PathBuf::from);
    let refresh = item.is_some() && cover.is_none();
    *PLAYING_COVER.lock().unwrap_or_else(|e| e.into_inner()) = cover.clone();
    if let Some(source) = COVER_SOURCE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        *source.item.lock().unwrap_or_else(|e| e.into_inner()) = item.cloned();
    }
    if VISUALIZER.wants_covers() {
        VISUALIZER.set_playing_cover(cover);
        if refresh {
            refresh_covers();
        }
    }
}

/// Ask the playing item's owning library for covers and give them to the visualizer.
fn refresh_covers() {
    // The cell of the track playing too: it may have changed while no such scene was on screen,
    // and the one the visualizer kept would cover the new atlas's.
    VISUALIZER.set_playing_cover(
        PLAYING_COVER
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
    );
    let source = COVER_SOURCE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some(source) = source else {
        return;
    };
    let runtime = source.runtime.clone();
    runtime.spawn(async move {
        let item = source
            .item
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(item) = item else {
            VISUALIZER.set_covers(None, Vec::new());
            return;
        };
        let library_id = match &item {
            PlayingItem::LibraryTrack { library_id, .. } => *library_id,
            _ => source.library_id,
        };
        let node = Arc::clone(&source.node);
        match async {
            let implementation = playback_implementation_for_item(&node, library_id).await?;
            implementation.visualizer_covers(&item, &node).await
        }
        .await
        {
            Ok((playing, random)) => {
                if source
                    .item
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    != Some(&item)
                {
                    return;
                }
                let playing = playing.map(PathBuf::from).or_else(|| {
                    PLAYING_COVER
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone()
                });
                VISUALIZER.set_covers(playing, random.into_iter().map(PathBuf::from).collect());
            }
            Err(e) => warn!("Failed to find covers for the visualizer: {e:#}"),
        }
    });
}

/// The same query as the cover wall's, for the cover files of random tracks.
async fn random_covers(source: &CoverSource) -> Result<Vec<PathBuf>> {
    let queue = source
        .node
        .session_queue_for_library(source.library_id)
        .await?;
    let files = query_mix_media_files(
        &source.main_db,
        &source.recommend_db,
        vec![
            ("lib::random".to_string(), RANDOM_COVERS.to_string()),
            ("filter::with_cover_art".to_string(), "true".to_string()),
        ],
        0,
        RANDOM_COVERS,
        &queue,
    )
    .await?;
    let ids: Vec<i32> = files.iter().map(|file| file.id).collect();
    let paths = bake_cover_art_by_media_files(&source.fsio, &source.main_db, files).await?;
    Ok(ids
        .iter()
        .filter_map(|id| paths.get(id))
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect())
}

/// Read the playing cover and the same bounded random selection as the cover wall for a feed.
pub async fn feed_cover_bytes(
    fsio: Arc<FsIo>,
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    node: Arc<NodeContext>,
    library_id: Uuid,
    playing_path: Option<PathBuf>,
) -> Result<(Option<Vec<u8>>, Vec<Vec<u8>>)> {
    let source = CoverSource {
        runtime: Handle::current(),
        fsio,
        main_db,
        recommend_db,
        node,
        library_id,
        item: Arc::new(Mutex::new(None)),
    };
    let playing = read_cover_bytes(playing_path.as_deref()).await?;
    let mut used_bytes = playing.as_ref().map_or(0, Vec::len);
    let mut random = Vec::with_capacity(RANDOM_COVERS);
    for path in random_covers(&source)
        .await?
        .into_iter()
        .take(RANDOM_COVERS)
    {
        if used_bytes >= MAX_FEED_BYTES {
            break;
        }
        if let Some(bytes) = read_cover_bytes(Some(&path)).await?
            && bytes.len() <= MAX_FEED_COVER_BYTES
            && used_bytes.saturating_add(bytes.len()) <= MAX_FEED_BYTES
        {
            used_bytes += bytes.len();
            random.push(bytes);
        }
    }
    Ok((playing, random))
}

async fn read_cover_bytes(path: Option<&std::path::Path>) -> Result<Option<Vec<u8>>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let metadata = tokio::fs::metadata(path).await?;
    if usize::try_from(metadata.len()).unwrap_or(usize::MAX) > MAX_FEED_COVER_BYTES {
        return Ok(None);
    }
    Ok(Some(tokio::fs::read(path).await?))
}
