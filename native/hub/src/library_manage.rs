use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use log::{debug, info, warn};
use rinf::DartSignal;
use tokio::sync::Mutex;
use tokio::task;
use tokio_util::sync::CancellationToken;

use database::actions::analysis::analysis_audio_library;
use database::actions::cover_art::scan_cover_arts;
use database::actions::metadata::scan_audio_library;
use database::actions::recommendation::sync_recommendation;
use database::connection::MainDbConnection;
use database::connection::RecommendationDbConnection;

use crate::messages::*;
use crate::TaskTokens;

pub async fn close_library_request(
    lib_path: Arc<String>,
    main_token: Arc<CancellationToken>,
    task_tokens: Arc<Mutex<TaskTokens>>,
    dart_signal: DartSignal<CloseLibraryRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    if request.path != *lib_path {
        return Ok(());
    }

    let mut tokens = task_tokens.lock().await;
    if let Some(token) = tokens.scan_token.take() {
        token.cancel();
    }
    if let Some(token) = tokens.analyse_token.take() {
        token.cancel();
    }

    main_token.cancel();

    CloseLibraryResponse {
        path: request.path.clone(),
    }
    .send_signal_to_dart();

    Ok(())
}

pub async fn scan_audio_library_request(
    main_db: Arc<MainDbConnection>,
    task_tokens: Arc<Mutex<TaskTokens>>,
    dart_signal: DartSignal<ScanAudioLibraryRequest>,
) -> Result<()> {
    let mut tokens = task_tokens.lock().await;
    // If there is scanning task
    if let Some(token) = tokens.scan_token.take() {
        token.cancel();
    }

    // Create a new cancel token
    let new_token = CancellationToken::new();
    tokens.scan_token = Some(new_token.clone());
    drop(tokens); // Release the lock

    let request = dart_signal.message.clone();
    let main_db_clone = Arc::clone(&main_db);

    task::spawn_blocking(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let path = request.path.clone();
            let result: Result<()> = async {
                let file_processed = scan_audio_library(
                    &main_db_clone,
                    Path::new(&path.clone()),
                    true,
                    |progress| {
                        ScanAudioLibraryProgress {
                            task: 0,
                            path: path.clone(),
                            progress: progress.try_into().unwrap(),
                            total: 0,
                        }
                        .send_signal_to_dart()
                    },
                    Some(new_token.clone()),
                )
                .await?;

                if new_token.is_cancelled() {
                    info!("Operation cancelled during artist processing.");

                    ScanAudioLibraryResponse {
                        path: request.path.clone(),
                        progress: -1,
                    }
                    .send_signal_to_dart();

                    return Ok(());
                }

                let batch_size = determine_batch_size(0.75);

                scan_cover_arts(
                    &main_db_clone,
                    Path::new(&path.clone()),
                    batch_size,
                    move |now, total| {
                        ScanAudioLibraryProgress {
                            task: 1,
                            path: path.clone(),
                            progress: now.try_into().unwrap(),
                            total: total.try_into().unwrap(),
                        }
                        .send_signal_to_dart()
                    },
                    Some(new_token.clone()),
                )
                .await?;

                ScanAudioLibraryResponse {
                    path: request.path.clone(),
                    progress: file_processed as i32,
                }
                .send_signal_to_dart();

                Ok(())
            }
            .await;

            result?;
            Ok::<(), anyhow::Error>(())
        })
    });

    Ok(())
}

pub fn determine_batch_size(workload_factor: f32) -> usize {
    let num_cores = num_cpus::get();
    let batch_size = ((num_cores as f32) * workload_factor).round() as usize;
    let min_batch_size = 1;
    let max_batch_size = 1000;

    std::cmp::min(std::cmp::max(batch_size, min_batch_size), max_batch_size)
}

pub async fn analyse_audio_library_request(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    task_tokens: Arc<Mutex<TaskTokens>>,
    dart_signal: DartSignal<AnalyseAudioLibraryRequest>,
) -> Result<()> {
    let mut tokens = task_tokens.lock().await;
    // If there is scanning task
    if let Some(token) = tokens.scan_token.take() {
        token.cancel();
    }

    // Create a new cancel token
    let new_token = CancellationToken::new();
    tokens.analyse_token = Some(new_token.clone());
    drop(tokens); // Release the lock

    let request = dart_signal.message;

    debug!("Analysing media files: {:#?}", request);

    let request_path = request.path.clone();
    let closure_request_path = request_path.clone();
    let batch_size = determine_batch_size(request.workload_factor);

    task::spawn_blocking(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let result = async {
                let total_files = analysis_audio_library(
                    &main_db,
                    Path::new(&request_path),
                    batch_size,
                    request.computing_device.into(),
                    move |progress, total| {
                        AnalyseAudioLibraryProgress {
                            path: closure_request_path.clone(),
                            progress: progress.try_into().unwrap(),
                            total: total.try_into().unwrap(),
                        }
                        .send_signal_to_dart()
                    },
                    Some(new_token.clone()),
                )
                .await
                .with_context(|| "Audio analysis failed")?;

                sync_recommendation(&main_db, &recommend_db)
                    .await
                    .with_context(|| "Recommendation synchronization failed")?;

                AnalyseAudioLibraryResponse {
                    path: request_path.clone(),
                    total: total_files as i32,
                }
                .send_signal_to_dart();

                Ok::<(), anyhow::Error>(())
            }
            .await;

            if let Err(e) = result {
                // Handle the error, e.g., log it
                eprintln!("Error: {:?}", e);
            }
        })
    });

    Ok(())
}

pub async fn cancel_task_request(
    task_tokens: Arc<Mutex<TaskTokens>>,
    dart_signal: DartSignal<CancelTaskRequest>,
) -> Result<()> {
    let request = dart_signal.message;
    let mut tokens = task_tokens.lock().await;

    let success = match request.r#type {
        0 => {
            if let Some(token) = tokens.analyse_token.take() {
                warn!("Cancelling analyse task");
                token.cancel();
                true
            } else {
                false
            }
        }
        1 => {
            if let Some(token) = tokens.scan_token.take() {
                warn!("Cancelling scan task");
                token.cancel();
                true
            } else {
                false
            }
        }
        _ => false,
    };

    CancelTaskResponse {
        path: request.path.clone(),
        r#type: request.r#type,
        success,
    }
    .send_signal_to_dart();

    Ok(())
}
