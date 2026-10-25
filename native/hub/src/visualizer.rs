//! The process-wide visualizer and the covers its scenes read.
//!
//! Its frames reach the screen through [`crate::flutter_bridge`], which registers a texture with
//! the engine from Rust, so the pixels never travel through Dart.

use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

use ::database::{
    actions::{cover_art::bake_cover_art_by_media_files, mixes::query_mix_media_files},
    connection::{MainDbConnection, RecommendationDbConnection},
};
use ::fsio::FsIo;
use anyhow::Result;
use log::warn;
use visualizer::Visualizer;

pub static VISUALIZER: LazyLock<Visualizer> = LazyLock::new(Visualizer::new);

/// How many random covers a scene that reads covers gets besides the one of the track playing.
const RANDOM_COVERS: usize = 63;

/// What finding the covers for scenes needs.
struct CoverSource {
    runtime: tokio::runtime::Handle,
    fsio: Arc<FsIo>,
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
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
) {
    *COVER_SOURCE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(CoverSource {
        runtime: tokio::runtime::Handle::current(),
        fsio,
        main_db,
        recommend_db,
    }));
    VISUALIZER.set_covers_requester(refresh_covers);
}

/// The player moved to another track, whose cover file is `cover` (none if it has none): a scene
/// that reads covers gets it in cell 0, the random covers stay.
pub fn playing_cover_changed(cover: Option<&str>) {
    let cover = cover.filter(|path| !path.is_empty()).map(PathBuf::from);
    *PLAYING_COVER.lock().unwrap_or_else(|e| e.into_inner()) = cover.clone();
    if VISUALIZER.wants_covers() {
        VISUALIZER.set_playing_cover(cover);
    }
}

/// Find 63 random covers in the background and give them to the visualizer with the playing
/// track's.
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
        match random_covers(&source).await {
            Ok(random) => {
                let current = PLAYING_COVER
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                VISUALIZER.set_covers(current, random);
            }
            Err(e) => warn!("Failed to find covers for the visualizer: {e:#}"),
        }
    });
}

/// The same query as the cover wall's, for the cover files of random tracks.
async fn random_covers(source: &CoverSource) -> Result<Vec<PathBuf>> {
    let files = query_mix_media_files(
        &source.main_db,
        &source.recommend_db,
        vec![
            ("lib::random".to_string(), RANDOM_COVERS.to_string()),
            ("filter::with_cover_art".to_string(), "true".to_string()),
        ],
        0,
        RANDOM_COVERS,
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
