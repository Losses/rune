use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use log::{debug, info, warn};
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
    type Params = (
        Arc<String>,
        Arc<CancellationToken>,
        Arc<Mutex<TaskTokens>>,
        Option<Arc<crate::server::ServerManager>>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.lib_path),
            Arc::clone(&all_params.main_token),
            Arc::clone(&all_params.task_tokens),
            all_params.server_manager.get().cloned(),
        )
    }
}

impl Signal for CloseLibraryRequest {
    type Params = (
        Arc<String>,
        Arc<CancellationToken>,
        Arc<Mutex<TaskTokens>>,
        Option<Arc<crate::server::ServerManager>>,
    );
    type Response = CloseLibraryResponse;

    async fn handle(
        &self,
        (lib_path, main_token, task_tokens, server_manager): Self::Params,
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

        drop(tokens);
        if let Some(server) = server_manager
            && server.is_running()
            && let Err(error) = server.stop().await
        {
            log::error!("Server shutdown failed: {error}");
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
        Option<database::sync::SeaOrmSyncStore>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.fsio),
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.node_id),
            Arc::clone(&all_params.task_tokens),
            Arc::clone(&all_params.broadcaster),
            all_params
                .library_sync
                .as_ref()
                .map(|library| library.record_store()),
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
        Option<database::sync::SeaOrmSyncStore>,
    );
    type Response = ();

    async fn handle(
        &self,
        (fsio, main_db, node_id, task_tokens, broadcaster, record_store): Self::Params,
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
        tokens.scan_id = Some(dart_signal.scan_id.clone());
        drop(tokens); // Release the lock

        // Clone all the data we need before spawning the task
        let request_path = dart_signal.path.clone();
        // FsIo on Android is rooted at the granted SAF tree; the content://
        // URI Dart passes is not a path inside that tree. Scans always start
        // at the tree root there.
        #[cfg(target_os = "android")]
        let scan_path = if request_path.starts_with("content://") {
            String::new()
        } else {
            request_path.clone()
        };
        #[cfg(not(target_os = "android"))]
        let scan_path = request_path.clone();
        let request_force = dart_signal.force;
        let main_db_clone = Arc::clone(&main_db);
        let broadcaster_clone = Arc::clone(&broadcaster);

        let scan_id = dart_signal.scan_id.clone();
        let worker_id = scan_id.clone();
        let supervisor_token = new_token.clone();
        let supervisor_path = request_path.clone();
        let supervisor_broadcaster = broadcaster.clone();
        let supervisor_tokens = task_tokens.clone();
        let worker = task::spawn_blocking(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                let result: Result<usize> = async {
                    let refresh_start = std::time::Instant::now();
                    if let Err(e) = fsio.refresh_cache() {
                        warn!("Failed to refresh filesystem cache before scan: {e:#?}");
                    } else {
                        info!(
                            "Filesystem cache refreshed in {:?}",
                            refresh_start.elapsed()
                        );
                    }

                    let file_processed = scan_audio_library(
                        &fsio,
                        &main_db_clone,
                        &node_id,
                        Path::new(&scan_path),
                        true,
                        request_force,
                        |progress| {
                            broadcaster_clone.broadcast(&ScanAudioLibraryProgress {
                                scan_id: worker_id.clone(),
                                task: ScanTaskType::IndexFiles,
                                path: request_path.clone(),
                                progress: progress.try_into().unwrap(),
                                total: 0,
                            });
                        },
                        Some(new_token.clone()),
                        record_store
                            .as_ref()
                            .context("library sync store unavailable")?,
                    )
                    .await?;

                    if new_token.is_cancelled() {
                        info!("Operation cancelled during artist processing.");

                        return Ok(file_processed);
                    }

                    let batch_size = determine_batch_size(0.75);
                    let cloned_broadcaster = Arc::clone(&broadcaster_clone);
                    let path_for_closure = request_path.clone();
                    let id_for_closure = worker_id.clone();

                    scan_cover_arts(
                        fsio,
                        &main_db_clone,
                        Path::new(&scan_path),
                        batch_size,
                        move |now, total| {
                            cloned_broadcaster.broadcast(&ScanAudioLibraryProgress {
                                scan_id: id_for_closure.clone(),
                                task: ScanTaskType::ScanCoverArts,
                                path: path_for_closure.clone(),
                                progress: now.try_into().unwrap(),
                                total: total.try_into().unwrap(),
                            });
                        },
                        Some(new_token.clone()),
                        record_store
                            .as_ref()
                            .context("library sync store unavailable")?,
                    )
                    .await?;

                    Ok(file_processed)
                }
                .await;

                result
            })
        });

        task::spawn(supervise_scan(
            worker,
            supervisor_token,
            supervisor_path,
            scan_id,
            supervisor_broadcaster,
            supervisor_tokens,
        ));

        Ok(None)
    }
}

async fn supervise_scan(
    worker: task::JoinHandle<Result<usize>>,
    supervisor_token: CancellationToken,
    supervisor_path: String,
    scan_id: String,
    supervisor_broadcaster: Arc<dyn Broadcaster>,
    supervisor_tokens: Arc<Mutex<TaskTokens>>,
) {
    let result = tokio::select! {
        result = worker => Some(result),
        _ = supervisor_token.cancelled() => None,
    };
    let mut tokens = supervisor_tokens.lock().await;
    let (status, progress, error) = match result {
        None => ("cancelled", 0, String::new()),
        Some(_) if supervisor_token.is_cancelled() => ("cancelled", 0, String::new()),
        Some(result) => match result {
            Ok(Ok(count)) => ("success", count as i32, String::new()),
            Ok(Err(error)) => ("failed", 0, format!("{error:#}")),
            Err(error) => ("failed", 0, format!("Scan worker failed: {error}")),
        },
    };
    if status == "failed" {
        warn!("Scan failed for {supervisor_path}: {error}");
    }
    if tokens.scan_id.as_ref() == Some(&scan_id) {
        tokens.scan_token = None;
        tokens.scan_id = None;
    }
    drop(tokens);
    supervisor_broadcaster.broadcast(&ScanAudioLibraryResponse {
        scan_id,
        status: status.to_string(),
        error,
        path: supervisor_path,
        progress,
    });
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
        (fsio, main_db, _node_id, recommend_db, task_tokens, broadcaster): Self::Params,
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
        // See ScanAudioLibraryRequest: on Android the content:// URI is not a
        // path inside the SAF tree; analysis starts at the tree root.
        #[cfg(target_os = "android")]
        let analysis_path = if request_path.starts_with("content://") {
            String::new()
        } else {
            request_path.clone()
        };
        #[cfg(not(target_os = "android"))]
        let analysis_path = request_path.clone();
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
                        Path::new(&analysis_path),
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
        Option<database::sync::SeaOrmSyncStore>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.fsio),
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.node_id),
            Arc::clone(&all_params.task_tokens),
            Arc::clone(&all_params.broadcaster),
            all_params
                .library_sync
                .as_ref()
                .map(|library| library.record_store()),
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
        Option<database::sync::SeaOrmSyncStore>,
    );
    type Response = ();

    async fn handle(
        &self,
        (fsio, main_db, node_id, task_tokens, broadcaster, record_store): Self::Params,
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
                let _ = uuid_node_id;

                let record_store = record_store
                    .as_ref()
                    .context("library sync store unavailable")?;
                // Stage 1: Compute fingerprints (0% - 33%)
                let broadcaster_clone = Arc::clone(&broadcaster);
                let progress_path = request_path_clone.to_string();

                let request_path_clone = request_path_clone.to_string();
                compute_file_fingerprints(
                    fsio,
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
                    record_store,
                )
                .await?;

                info!(" Compute fingerprints completed.");

                // Stage 2: Compare all pairs (33% - 66%)
                let broadcaster_clone = Arc::clone(&broadcaster);
                let progress_path = request_path_clone.to_string();

                compare_all_pairs(
                    &main_db,
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
                    record_store,
                )
                .await?;

                info!("Comparing fingerprints completed.");

                // Stage 3: Mark duplicates (66% - 100%)
                if !new_token.is_cancelled() {
                    let broadcaster_clone = Arc::clone(&broadcaster);
                    let progress_path = request_path_clone.to_string();

                    mark_duplicate_files(
                        &main_db,
                        similarity_threshold,
                        move |cur, total| {
                            let progress = 0.66 + cur as f32 / total as f32 * 0.34;

                            broadcaster_clone.broadcast(&DeduplicateAudioLibraryProgress {
                                path: progress_path.clone(),
                                progress: (progress * 100.0) as i32,
                                total: 100,
                            });
                        },
                        record_store,
                    )
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
                if request.scan_id.is_none() || request.scan_id != tokens.scan_id {
                    false
                } else if let Some(token) = tokens.scan_token.take() {
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

#[cfg(test)]
mod scan_supervisor_tests {
    use super::*;
    use crate::utils::RinfRustSignal;

    #[derive(Default)]
    struct Capture(std::sync::Mutex<Vec<Vec<u8>>>);
    impl Broadcaster for Capture {
        fn broadcast(&self, message: &dyn RinfRustSignal) {
            if message.name() == "ScanAudioLibraryResponse" {
                self.0
                    .lock()
                    .unwrap()
                    .push(message.encode_to_vec().unwrap());
            }
        }
    }

    #[tokio::test]
    async fn terminal_events_and_replacement_protection() {
        for outcome in ["success", "failed", "panic", "cancelled", "replacement"] {
            let token = CancellationToken::new();
            let replacement = CancellationToken::new();
            let tokens = Arc::new(Mutex::new(TaskTokens {
                scan_id: Some(
                    if outcome == "replacement" {
                        "new"
                    } else {
                        "old"
                    }
                    .into(),
                ),
                scan_token: Some(if outcome == "replacement" {
                    replacement.clone()
                } else {
                    token.clone()
                }),
                ..Default::default()
            }));
            if outcome == "cancelled" || outcome == "replacement" {
                token.cancel();
            }
            let worker = task::spawn(async move {
                match outcome {
                    "failed" => anyhow::bail!("injected failure"),
                    "panic" => panic!("injected panic"),
                    _ => Ok(7),
                }
            });
            let capture = Arc::new(Capture::default());
            supervise_scan(
                worker,
                token,
                "library".into(),
                "old".into(),
                capture.clone(),
                tokens.clone(),
            )
            .await;
            let response: ScanAudioLibraryResponse = {
                let events = capture.0.lock().unwrap();
                assert_eq!(events.len(), 1);
                rinf::deserialize(&events[0]).unwrap()
            };
            let expected = match outcome {
                "failed" | "panic" => "failed",
                "replacement" => "cancelled",
                other => other,
            };
            assert_eq!(response.status, expected);
            if expected == "failed" {
                assert!(!response.error.is_empty());
            }
            let tokens = tokens.lock().await;
            if outcome == "replacement" {
                assert_eq!(tokens.scan_id.as_deref(), Some("new"));
                assert!(tokens.scan_token.is_some());
                assert!(!replacement.is_cancelled());
            } else {
                assert!(tokens.scan_token.is_none());
            }
        }
    }
    #[tokio::test]
    async fn cancellation_reports_while_worker_is_blocked() {
        let token = CancellationToken::new();
        let tokens = Arc::new(Mutex::new(TaskTokens {
            scan_id: Some("old".into()),
            scan_token: Some(token.clone()),
            ..Default::default()
        }));
        let (release, blocked) = tokio::sync::oneshot::channel::<()>();
        let worker = task::spawn(async move {
            let _ = blocked.await;
            Ok(1)
        });
        let capture = Arc::new(Capture::default());
        let supervisor = task::spawn(supervise_scan(
            worker,
            token.clone(),
            "library".into(),
            "old".into(),
            capture.clone(),
            tokens,
        ));
        token.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(1), supervisor)
            .await
            .unwrap()
            .unwrap();
        let events = capture.0.lock().unwrap();
        let response: ScanAudioLibraryResponse = rinf::deserialize(&events[0]).unwrap();
        assert_eq!(response.status, "cancelled");
        drop(events);
        let _ = release.send(());
    }
    #[tokio::test]
    async fn old_cancel_request_does_not_cancel_replacement() {
        let replacement = CancellationToken::new();
        let tokens = Arc::new(Mutex::new(TaskTokens {
            scan_id: Some("new".into()),
            scan_token: Some(replacement.clone()),
            ..Default::default()
        }));
        let request = CancelTaskRequest {
            path: "library".into(),
            scan_id: Some("old".into()),
            r#type: CancelTaskType::ScanAudioLibrary,
        };
        let response = request
            .handle((tokens.clone(),), None, &request)
            .await
            .unwrap()
            .unwrap();
        assert!(!response.success);
        assert!(!replacement.is_cancelled());
        assert_eq!(tokens.lock().await.scan_id.as_deref(), Some("new"));
    }
    #[tokio::test]
    async fn handler_broadcasts_missing_store_error() {
        let capture = Arc::new(Capture::default());
        let tokens = Arc::new(Mutex::new(TaskTokens::default()));
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        let request = ScanAudioLibraryRequest {
            scan_id: "handler".into(),
            path: "/tmp".into(),
            force: false,
        };
        request
            .handle(
                (
                    Arc::new(FsIo::new()),
                    Arc::new(db),
                    Arc::new(uuid::Uuid::new_v4().to_string()),
                    tokens,
                    capture.clone(),
                    None,
                ),
                None,
                &request,
            )
            .await
            .unwrap();
        let response = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(bytes) = capture.0.lock().unwrap().first().cloned() {
                    break rinf::deserialize::<ScanAudioLibraryResponse>(&bytes).unwrap();
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(response.scan_id, "handler");
        assert_eq!(response.status, "failed");
        assert!(response.error.contains("library sync store unavailable"));
    }
}
