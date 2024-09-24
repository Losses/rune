use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use log::{debug, info};
use rinf::DartSignal;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use database::actions::analysis::analysis_audio_library;
use database::actions::cover_art::scan_cover_arts;
use database::actions::metadata::scan_audio_library;
use database::actions::recommendation::sync_recommendation;
use database::connection::MainDbConnection;
use database::connection::RecommendationDbConnection;
use database::connection::SearchDbConnection;

use crate::messages::library_manage::{
    ScanAudioLibraryProgress, ScanAudioLibraryRequest, ScanAudioLibraryResponse,
};
use crate::{
    AnalyseAudioLibraryProgress, AnalyseAudioLibraryRequest, AnalyseAudioLibraryResponse,
    CloseLibraryRequest, CloseLibraryResponse,
};

pub async fn close_library_request(
    lib_path: Arc<String>,
    cancel_token: Arc<CancellationToken>,
    dart_signal: DartSignal<CloseLibraryRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    if request.path != *lib_path {
        return Ok(());
    }

    cancel_token.cancel();

    CloseLibraryResponse {
        path: request.path.clone(),
    }
    .send_signal_to_dart();

    Ok(())
}

pub async fn scan_audio_library_request(
    main_db: Arc<MainDbConnection>,
    search_db: Arc<Mutex<SearchDbConnection>>,
    cancel_token: Arc<CancellationToken>,
    dart_signal: DartSignal<ScanAudioLibraryRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    let mut search_db = search_db.lock().await;

    let file_processed = scan_audio_library(
        &main_db,
        &mut search_db,
        Path::new(&request.path),
        true,
        |progress| {
            ScanAudioLibraryProgress {
                path: request.path.clone(),
                progress: progress.try_into().unwrap(),
            }
            .send_signal_to_dart()
        },
        Some((*cancel_token).clone()),
    )
    .await
    .unwrap();

    let batch_size = determine_batch_size();

    scan_cover_arts(
        &main_db,
        Path::new(&request.path),
        batch_size,
        |now, total| info!("Scanning cover art: {}/{}", now, total),
        None,
    )
    .await?;

    ScanAudioLibraryResponse {
        path: request.path.clone(),
        progress: file_processed as i32,
    }
    .send_signal_to_dart();

    Ok(())
}

pub fn determine_batch_size() -> usize {
    let num_cores = num_cpus::get();
    let batch_size = num_cores / 4 * 3;
    let min_batch_size = 1;
    let max_batch_size = 1000;

    std::cmp::min(std::cmp::max(batch_size, min_batch_size), max_batch_size)
}

pub async fn analyse_audio_library_request(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    cancel_token: Arc<CancellationToken>,
    dart_signal: DartSignal<AnalyseAudioLibraryRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    debug!("Analysing media files: {:#?}", request);

    let request_path = request.path.clone();
    let closure_request_path = request_path.clone();
    let batch_size = determine_batch_size();

    let total_files = analysis_audio_library(
        &main_db,
        Path::new(&request_path),
        batch_size,
        move |progress, total| {
            AnalyseAudioLibraryProgress {
                path: closure_request_path.clone(), // Use the cloned path here
                progress: progress.try_into().unwrap(),
                total: total.try_into().unwrap(),
            }
            .send_signal_to_dart()
        },
        Some((*cancel_token).clone()),
    )
    .await
    .with_context(|| "Audio analysis failed")?;

    sync_recommendation(&main_db, &recommend_db)
        .await
        .with_context(|| "Recommendation synchronization failed")?;

    AnalyseAudioLibraryResponse {
        path: request_path.clone(), // Use the original cloned path here
        total: total_files as i32,
    }
    .send_signal_to_dart();

    Ok(())
}
