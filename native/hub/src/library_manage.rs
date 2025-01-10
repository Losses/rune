use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use log::{debug, info, warn};
use tokio::sync::Mutex;
use tokio::task;
use tokio_util::sync::CancellationToken;

use database::actions::analysis::analysis_audio_library;
use database::actions::cover_art::scan_cover_arts;
use database::actions::metadata::scan_audio_library;
use database::actions::recommendation::sync_recommendation;
use database::connection::MainDbConnection;
use database::connection::RecommendationDbConnection;

use crate::utils::determine_batch_size;
use crate::utils::GlobalParams;
use crate::utils::ParamsExtractor;
use crate::TaskTokens;
use crate::{messages::*, Signal};

impl ParamsExtractor for CloseLibraryRequest {
    type Params = (
        Arc<String>,
        Arc<CancellationToken>,
        Arc<Mutex<TaskTokens>>,
    );

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
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        if request.path != *lib_path {
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
    type Params = (Arc<MainDbConnection>, Arc<Mutex<TaskTokens>>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.task_tokens),
        )
    }
}

impl Signal for ScanAudioLibraryRequest {
    type Params = (Arc<MainDbConnection>, Arc<Mutex<TaskTokens>>);
    type Response = ();

    async fn handle(
        &self,
        (main_db, task_tokens): Self::Params,
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

        Ok(None)
    }
}

impl ParamsExtractor for AnalyzeAudioLibraryRequest {
    type Params = (
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
        Arc<Mutex<TaskTokens>>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.recommend_db),
            Arc::clone(&all_params.task_tokens),
        )
    }
}

impl Signal for AnalyzeAudioLibraryRequest {
    type Params = (
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
        Arc<Mutex<TaskTokens>>,
    );
    type Response = ();

    async fn handle(
        &self,
        (main_db, recommend_db, task_tokens): Self::Params,
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
                let result = async {
                    let total_files = analysis_audio_library(
                        &main_db,
                        Path::new(&request_path),
                        batch_size,
                        request.computing_device.into(),
                        move |progress, total| {
                            AnalyzeAudioLibraryProgress {
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

                    AnalyzeAudioLibraryResponse {
                        path: request_path.clone(),
                        total: total_files as i32,
                    }
                    .send_signal_to_dart();

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
