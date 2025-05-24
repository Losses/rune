use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use log::{debug, info, warn};
use tokio::{sync::Mutex, task};
use tokio_util::sync::CancellationToken;

use database::{
    actions::{
        analysis::analysis_audio_library,
        cover_art::scan_cover_arts,
        fingerprint::{
            compare_all_pairs, compute_file_fingerprints, mark_duplicate_files, Configuration,
        },
        metadata::scan_audio_library,
        recommendation::sync_recommendation,
    },
    connection::{MainDbConnection, RecommendationDbConnection},
};

use crate::{
    messages::*,
    utils::{determine_batch_size, Broadcaster, GlobalParams, ParamsExtractor},
    Session, Signal, TaskTokens,
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
        Arc<MainDbConnection>,
        Arc<String>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.node_id),
            Arc::clone(&all_params.task_tokens),
            Arc::clone(&all_params.broadcaster),
        )
    }
}

impl Signal for ScanAudioLibraryRequest {
    type Params = (
        Arc<MainDbConnection>,
        Arc<String>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
    );
    type Response = ();

    async fn handle(
        &self,
        (main_db, node_id, task_tokens, broadcaster): Self::Params,
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

        let request = dart_signal.clone();
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
                        request.force,
                        |progress| {
                            broadcaster.broadcast(&ScanAudioLibraryProgress {
                                task: 0,
                                path: path.clone(),
                                progress: progress.try_into().unwrap(),
                                total: 0,
                            });
                        },
                        Some(new_token.clone()),
                    )
                    .await?;

                    if new_token.is_cancelled() {
                        info!("Operation cancelled during artist processing.");

                        broadcaster.broadcast(&ScanAudioLibraryResponse {
                            path: request.path.clone(),
                            progress: -1,
                        });

                        return Ok(());
                    }

                    let batch_size = determine_batch_size(0.75);
                    let cloned_broadcaster = Arc::clone(&broadcaster);

                    scan_cover_arts(
                        &main_db_clone,
                        Path::new(&path.clone()),
                        &node_id,
                        batch_size,
                        move |now, total| {
                            cloned_broadcaster.broadcast(&ScanAudioLibraryProgress {
                                task: 1,
                                path: path.clone(),
                                progress: now.try_into().unwrap(),
                                total: total.try_into().unwrap(),
                            });
                        },
                        Some(new_token.clone()),
                    )
                    .await?;

                    broadcaster.broadcast(&ScanAudioLibraryResponse {
                        path: request.path.clone(),
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

impl ParamsExtractor for AnalyzeAudioLibraryRequest {
    type Params = (
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.recommend_db),
            Arc::clone(&all_params.task_tokens),
            Arc::clone(&all_params.broadcaster),
        )
    }
}

impl Signal for AnalyzeAudioLibraryRequest {
    type Params = (
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
    );
    type Response = ();

    async fn handle(
        &self,
        (main_db, recommend_db, task_tokens, broadcaster): Self::Params,
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

        let request = dart_signal.clone();
        debug!("Analyzing media files: {:#?}", request);

        let request_path = request.path.clone();
        let closure_request_path = request_path.clone();
        let batch_size = determine_batch_size(request.workload_factor);

        task::spawn_blocking(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                let cloned_broadcaster = Arc::clone(&broadcaster);
                let result = async {
                    let total_files = analysis_audio_library(
                        &main_db,
                        Path::new(&request_path),
                        batch_size,
                        request.computing_device.into(),
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
                    eprintln!("Error: {:?}", e);
                }
            })
        });

        Ok(Some(()))
    }
}

impl ParamsExtractor for DeduplicateAudioLibraryRequest {
    type Params = (
        Arc<MainDbConnection>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.task_tokens),
            Arc::clone(&all_params.broadcaster),
        )
    }
}

impl Signal for DeduplicateAudioLibraryRequest {
    type Params = (
        Arc<MainDbConnection>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
    );
    type Response = ();

    async fn handle(
        &self,
        (main_db, task_tokens, broadcaster): Self::Params,
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

        let request = dart_signal.clone();
        let request_path = Arc::new(request.path.clone());
        let batch_size = determine_batch_size(request.workload_factor);
        let config = Configuration::default();
        let similarity_threshold = request.similarity_threshold;

        let request_path_clone = request_path.clone();
        task::spawn_blocking(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();

            let request_path_clone = request_path_clone.clone();
            rt.block_on(async {
                // Stage 1: Compute fingerprints (0% - 33%)
                let broadcaster_clone = Arc::clone(&broadcaster);
                let progress_path = request_path_clone.to_string();

                let request_path_clone = request_path_clone.to_string();
                compute_file_fingerprints(
                    &main_db,
                    Path::new(&request_path_clone),
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
            0 => {
                if let Some(token) = tokens.analyze_token.take() {
                    warn!("Cancelling analyze task");
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

        Ok(Some(CancelTaskResponse {
            path: request.path.clone(),
            r#type: request.r#type,
            success,
        }))
    }
}
