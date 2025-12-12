use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use log::{debug, info, warn};
use sync::hlc::SyncTaskContext;
use tokio::{sync::Mutex, task};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use ::analysis::utils::computing_device::ComputingDevice;
use ::database::{
    actions::{
        analysis::analysis_audio_library,
        cover_art::scan_cover_arts,
        fingerprint::{
            Configuration, compare_all_pairs, compute_file_fingerprints, mark_duplicate_files,
        },
        metadata::scan_audio_library,
        recommendation::sync_recommendation,
    },
    connection::{MainDbConnection, RecommendationDbConnection},
};
use ::fsio::FsIo;

use crate::{
    Session, Signal, TaskTokens,
    messages::*,
    utils::{Broadcaster, GlobalParams, ParamsExtractor, determine_batch_size},
};

impl ParamsExtractor for CloseLibraryRequest {
    type Params = (Arc<String>, Arc<CancellationToken>, Arc<Mutex<TaskTokens>>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.lib_path),
            Arc::clone(&all_params.main_token),
            Arc::clone(&all_params.task_tokens),
        )
    }
}

impl Signal for CloseLibraryRequest {
    type Params = (Arc<String>, Arc<CancellationToken>, Arc<Mutex<TaskTokens>>);
    type Response = CloseLibraryResponse;

    async fn handle(
        &self,
        (lib_path, main_token, task_tokens): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        info!("Closing library: {:#?}", request.path);

        if request.path != *lib_path {
            warn!(
                "Library path mismatch: {:#?} != {:#?}",
                request.path, *lib_path
            );
            return Ok(None);
        }

        let mut tokens = task_tokens.lock().await;
        if let Some(token) = tokens.scan_token.take() {
            token.cancel();
        }
        if let Some(token) = tokens.analyze_token.take() {
            token.cancel();
        }

        main_token.cancel();

        Ok(Some(CloseLibraryResponse {
            path: request.path.clone(),
        }))
    }
}

impl ParamsExtractor for ScanAudioLibraryRequest {
    type Params = (
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<String>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.fsio),
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.node_id),
            Arc::clone(&all_params.task_tokens),
            Arc::clone(&all_params.broadcaster),
        )
    }
}

impl Signal for ScanAudioLibraryRequest {
    type Params = (
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<String>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
    );
    type Response = ();

    async fn handle(
        &self,
        (fsio, main_db, node_id, task_tokens, broadcaster): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<()>> {
        let mut tokens = task_tokens.lock().await;
        // If there is scanning task
        if let Some(token) = tokens.scan_token.take() {
            token.cancel();
        }

        // Create a new cancel token
        let new_token = CancellationToken::new();
        tokens.scan_token = Some(new_token.clone());
        drop(tokens); // Release the lock

        // Clone all the data we need before spawning the task
        let request_path = dart_signal.path.clone();
        let request_force = dart_signal.force;
        let main_db_clone = Arc::clone(&main_db);
        let node_id_clone = Arc::clone(&node_id);
        let broadcaster_clone = Arc::clone(&broadcaster);

        task::spawn_blocking(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                let result: Result<()> = async {
                    let file_processed = scan_audio_library(
                        &fsio,
                        &main_db_clone,
                        &node_id,
                        Path::new(&request_path),
                        true,
                        request_force,
                        |progress| {
                            broadcaster_clone.broadcast(&ScanAudioLibraryProgress {
                                task: ScanTaskType::IndexFiles,
                                path: request_path.clone(),
                                progress: progress.try_into().unwrap(),
                                total: 0,
                            });
                        },
                        Some(new_token.clone()),
                    )
                    .await?;

                    if new_token.is_cancelled() {
                        info!("Operation cancelled during artist processing.");

                        broadcaster_clone.broadcast(&ScanAudioLibraryResponse {
                            path: request_path.clone(),
                            progress: -1,
                        });

                        return Ok(());
                    }

                    let batch_size = determine_batch_size(0.75);
                    let cloned_broadcaster = Arc::clone(&broadcaster_clone);
                    let path_for_closure = request_path.clone();

                    scan_cover_arts(
                        fsio,
                        &main_db_clone,
                        Path::new(&request_path),
                        &node_id_clone,
                        batch_size,
                        move |now, total| {
                            cloned_broadcaster.broadcast(&ScanAudioLibraryProgress {
                                task: ScanTaskType::ScanCoverArts,
                                path: path_for_closure.clone(),
                                progress: now.try_into().unwrap(),
                                total: total.try_into().unwrap(),
                            });
                        },
                        Some(new_token.clone()),
                    )
                    .await?;

                    broadcaster_clone.broadcast(&ScanAudioLibraryResponse {
                        path: request_path.clone(),
                        progress: file_processed as i32,
                    });

                    Ok(())
                }
                .await;

                result?;
                Ok::<(), anyhow::Error>(())
            })
        });

        Ok(None)
    }
}

impl From<ComputingDeviceRequest> for ComputingDevice {
    fn from(value: ComputingDeviceRequest) -> Self {
        match value {
            ComputingDeviceRequest::Cpu => ComputingDevice::Cpu,
            ComputingDeviceRequest::Gpu => ComputingDevice::Gpu,
        }
    }
}

impl ParamsExtractor for AnalyzeAudioLibraryRequest {
    type Params = (
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<String>,
        Arc<RecommendationDbConnection>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.fsio),
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.node_id),
            Arc::clone(&all_params.recommend_db),
            Arc::clone(&all_params.task_tokens),
            Arc::clone(&all_params.broadcaster),
        )
    }
}

impl Signal for AnalyzeAudioLibraryRequest {
    type Params = (
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<String>,
        Arc<RecommendationDbConnection>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
    );
    type Response = ();

    async fn handle(
        &self,
        (fsio, main_db, node_id, recommend_db, task_tokens, broadcaster): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let mut tokens = task_tokens.lock().await;
        if let Some(token) = tokens.scan_token.take() {
            token.cancel();
        }

        let new_token = CancellationToken::new();
        tokens.analyze_token = Some(new_token.clone());
        drop(tokens);

        // Clone the data from dart_signal before spawning the task
        let request = dart_signal;
        debug!("Analyzing media files: {request:#?}");

        let request_path = request.path.clone();
        let closure_request_path = request_path.clone();
        let batch_size = determine_batch_size(request.workload_factor);
        let computing_device = request.computing_device;

        task::spawn_blocking(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                let cloned_broadcaster = Arc::clone(&broadcaster);
                let result = async {
                    let total_files = analysis_audio_library(
                        fsio,
                        &main_db,
                        Path::new(&request_path),
                        &node_id,
                        batch_size,
                        computing_device.into(),
                        move |progress, total| {
                            cloned_broadcaster.broadcast(&AnalyzeAudioLibraryProgress {
                                path: closure_request_path.clone(),
                                progress: progress.try_into().unwrap(),
                                total: total.try_into().unwrap(),
                            });
                        },
                        Some(new_token.clone()),
                    )
                    .await
                    .with_context(|| "Audio analysis failed")?;

                    sync_recommendation(&main_db, &recommend_db)
                        .await
                        .with_context(|| "Recommendation synchronization failed")?;

                    broadcaster.broadcast(&AnalyzeAudioLibraryResponse {
                        path: request_path.clone(),
                        total: total_files as i32,
                    });

                    Ok::<(), anyhow::Error>(())
                }
                .await;

                if let Err(e) = result {
                    eprintln!("Error: {e:?}");
                }
            })
        });

        Ok(Some(()))
    }
}

impl ParamsExtractor for DeduplicateAudioLibraryRequest {
    type Params = (
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<String>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.fsio),
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.node_id),
            Arc::clone(&all_params.task_tokens),
            Arc::clone(&all_params.broadcaster),
        )
    }
}

impl Signal for DeduplicateAudioLibraryRequest {
    type Params = (
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<String>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
    );
    type Response = ();

    async fn handle(
        &self,
        (fsio, main_db, node_id, task_tokens, broadcaster): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let mut tokens = task_tokens.lock().await;
        if let Some(token) = tokens.deduplicate_token.take() {
            token.cancel();
        }

        let new_token = CancellationToken::new();
        tokens.deduplicate_token = Some(new_token.clone());
        drop(tokens);

        let request = dart_signal;
        let request_path = Arc::new(request.path.clone());
        let batch_size = determine_batch_size(request.workload_factor);
        let config = Configuration::default();
        let similarity_threshold = request.similarity_threshold;

        let request_path_clone = request_path.clone();
        task::spawn_blocking(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();

            let request_path_clone = request_path_clone.clone();
            rt.block_on(async {
                let uuid_node_id = match Uuid::parse_str(&node_id) {
                    Ok(id) => id,
                    Err(e) => {
                         let broadcaster_clone = Arc::clone(&broadcaster);
                         // Ideally we should send an error message back.
                         // But for now logging it and returning.
                         log::error!("Invalid node ID {node_id}: {e}");
                         broadcaster_clone.broadcast(&DeduplicateAudioLibraryResponse {
                             path: request_path_clone.to_string(),
                         });
                         return Ok(());
                    }
                };
                let hlc_context = Arc::new(SyncTaskContext::new(uuid_node_id));

                // Stage 1: Compute fingerprints (0% - 33%)
                let broadcaster_clone = Arc::clone(&broadcaster);
                let progress_path = request_path_clone.to_string();

                let request_path_clone = request_path_clone.to_string();
                compute_file_fingerprints(
                    fsio,
                    &main_db,
                    Path::new(&request_path_clone),
                    hlc_context.clone(),
                    batch_size,
                    move |cur, total| {
                        let progress = cur as f32 / total as f32 * 0.33;

                        broadcaster_clone.broadcast(&DeduplicateAudioLibraryProgress {
                            path: progress_path.clone(),
                            progress: (progress * 100.0) as i32,
                            total: 100,
                        })
                    },
                    Some(new_token.clone()),
                )
                .await?;

                info!(" Compute fingerprints completed.");

                // Stage 2: Compare all pairs (33% - 66%)
                let broadcaster_clone = Arc::clone(&broadcaster);
                let progress_path = request_path_clone.to_string();

                compare_all_pairs(
                    &main_db,
                    hlc_context,
                    batch_size,
                    move |cur, total| {
                        let progress = 0.33 + cur as f32 / total as f32 * 0.33;

                        broadcaster_clone.broadcast(&DeduplicateAudioLibraryProgress {
                            path: progress_path.clone(),
                            progress: (progress * 100.0) as i32,
                            total: 100,
                        });
                    },
                    &config,
                    Some(Arc::new(new_token.clone())),
                    1000,
                )
                .await?;

                info!("Comparing fingerprints completed.");

                // Stage 3: Mark duplicates (66% - 100%)
                if !new_token.is_cancelled() {
                    let broadcaster_clone = Arc::clone(&broadcaster);
                    let progress_path = request_path_clone.to_string();

                    mark_duplicate_files(&main_db, similarity_threshold, move |cur, total| {
                        let progress = 0.66 + cur as f32 / total as f32 * 0.34;

                        broadcaster_clone.broadcast(&DeduplicateAudioLibraryProgress {
                            path: progress_path.clone(),
                            progress: (progress * 100.0) as i32,
                            total: 100,
                        });
                    })
                    .await?;
                }

                let request_path_clone = request_path_clone.to_string();
                broadcaster.broadcast(&DeduplicateAudioLibraryResponse {
                    path: request_path_clone,
                });

                Ok::<(), anyhow::Error>(())
            })
        });

        Ok(Some(()))
    }
}

impl ParamsExtractor for CancelTaskRequest {
    type Params = (Arc<Mutex<TaskTokens>>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.task_tokens),)
    }
}

impl Signal for CancelTaskRequest {
    type Params = (Arc<Mutex<TaskTokens>>,);
    type Response = CancelTaskResponse;

    async fn handle(
        &self,
        (task_tokens,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let mut tokens = task_tokens.lock().await;

        let success = match request.r#type {
            CancelTaskType::AnalyzeAudioLibrary => {
                if let Some(token) = tokens.analyze_token.take() {
                    warn!("Cancelling analyze task");
                    token.cancel();
                    true
                } else {
                    false
                }
            }
            CancelTaskType::ScanAudioLibrary => {
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

        Ok(Some(CancelTaskResponse {
            path: request.path.clone(),
            r#type: request.r#type,
            success,
        }))
    }
}
