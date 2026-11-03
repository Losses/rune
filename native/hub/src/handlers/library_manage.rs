use crate::utils::sync_settings::SyncSettings;
use database::actions::metadata::ScanOptions;
use std::time::Instant;
use std::{path::Path, sync::Arc};
use tokio::runtime::Runtime;

use anyhow::{Context, Result};
use log::{debug, info, warn};
use tokio::{
    sync::{Mutex, RwLock},
    task,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use ::analysis::utils::computing_device::ComputingDevice;
use ::database::actions::{
    analysis::analysis_audio_library,
    cover_art::scan_cover_arts,
    fingerprint::{
        Configuration, compare_all_pairs, compute_file_fingerprints, mark_duplicate_files,
    },
    metadata::scan_audio_library,
    recommendation::sync_recommendation,
};

use crate::{
    Session, Signal, TaskTokens,
    library_pool::LibraryHandle,
    messages::*,
    node::NodeContext,
    utils::{Broadcaster, GlobalParams, ParamsExtractor, determine_batch_size, require_library},
};

pub struct CloseLibraryParams {
    library: Option<LibraryHandle>,
    main_token: Arc<CancellationToken>,
    node: Arc<NodeContext>,
}

impl ParamsExtractor for CloseLibraryRequest {
    type Params = CloseLibraryParams;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        CloseLibraryParams {
            library: all_params.library.clone(),
            main_token: Arc::clone(&all_params.main_token),
            node: all_params.node.clone(),
        }
    }
}

impl Signal for CloseLibraryRequest {
    type Params = CloseLibraryParams;
    type Response = CloseLibraryResponse;

    async fn handle(
        &self,
        params: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let CloseLibraryParams {
            library,
            main_token,
            node,
        } = params;
        let library = require_library(library)?;
        let request = dart_signal;

        info!("Closing library: {:#?}", request.path);

        if request.path != library.media_root {
            warn!(
                "Library path mismatch: {:#?} != {:#?}",
                request.path, library.media_root
            );
            return Ok(None);
        }

        node.close_ui(&main_token).await;

        Ok(Some(CloseLibraryResponse {
            path: request.path.clone(),
        }))
    }
}

impl ParamsExtractor for ScanAudioLibraryRequest {
    type Params = (
        Option<LibraryHandle>,
        Arc<String>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
        Arc<RwLock<SyncSettings>>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library.clone(),
            Arc::clone(&all_params.node_id),
            Arc::clone(&all_params.task_tokens),
            Arc::clone(&all_params.broadcaster),
            all_params.sync_settings.clone(),
        )
    }
}

impl Signal for ScanAudioLibraryRequest {
    type Params = (
        Option<LibraryHandle>,
        Arc<String>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
        Arc<RwLock<SyncSettings>>,
    );
    type Response = ();

    async fn handle(
        &self,
        (library, node_id, task_tokens, broadcaster, sync_settings): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<()>> {
        let library = require_library(library)?;
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
        let missing_host_file = sync_settings.read().await.missing_host_file;
        let main_db_clone = Arc::clone(&library.main_db);
        let broadcaster_clone = Arc::clone(&broadcaster);

        let scan_id = dart_signal.scan_id.clone();
        let worker_id = scan_id.clone();
        let supervisor_token = new_token.clone();
        let supervisor_path = request_path.clone();
        let supervisor_broadcaster = broadcaster.clone();
        let supervisor_tokens = task_tokens.clone();
        let worker = task::spawn_blocking(move || {
            let _library = library;
            let runtime = Runtime::new().unwrap();
            runtime.block_on(async move {
                let result: Result<usize> = async {
                    let refresh_start = Instant::now();
                    if let Err(e) = _library.fsio.refresh_cache() {
                        warn!("Failed to refresh filesystem cache before scan: {e:#?}");
                    } else {
                        info!(
                            "Filesystem cache refreshed in {:?}",
                            refresh_start.elapsed()
                        );
                    }

                    let file_processed = scan_audio_library(
                        &_library.fsio,
                        &main_db_clone,
                        &node_id,
                        Path::new(&scan_path),
                        ScanOptions {
                            cleanup: true,
                            force: request_force,
                            missing_host_file,
                            cancel_token: Some(new_token.clone()),
                        },
                        |progress| {
                            broadcaster_clone.broadcast(&ScanAudioLibraryProgress {
                                scan_id: worker_id.clone(),
                                task: ScanTaskType::IndexFiles,
                                path: request_path.clone(),
                                progress: progress.try_into().unwrap(),
                                total: 0,
                            });
                        },
                        &_library.library_sync.record_store(),
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
                        Arc::clone(&_library.fsio),
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
                        &_library.library_sync.record_store(),
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
        Option<LibraryHandle>,
        Arc<String>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library.clone(),
            Arc::clone(&all_params.node_id),
            Arc::clone(&all_params.task_tokens),
            Arc::clone(&all_params.broadcaster),
        )
    }
}

impl Signal for AnalyzeAudioLibraryRequest {
    type Params = (
        Option<LibraryHandle>,
        Arc<String>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
    );
    type Response = ();

    async fn handle(
        &self,
        (library, _node_id, task_tokens, broadcaster): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
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
            let _library = library;
            let runtime = Runtime::new().unwrap();
            runtime.block_on(async move {
                let cloned_broadcaster = Arc::clone(&broadcaster);
                let result = async {
                    let total_files = analysis_audio_library(
                        Arc::clone(&_library.fsio),
                        &_library.main_db,
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

                    sync_recommendation(&_library.main_db, &_library.recommend_db)
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
        Option<LibraryHandle>,
        Arc<String>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library.clone(),
            Arc::clone(&all_params.node_id),
            Arc::clone(&all_params.task_tokens),
            Arc::clone(&all_params.broadcaster),
        )
    }
}

impl Signal for DeduplicateAudioLibraryRequest {
    type Params = (
        Option<LibraryHandle>,
        Arc<String>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
    );
    type Response = ();

    async fn handle(
        &self,
        (library, node_id, task_tokens, broadcaster): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
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
            let _library = library;
            let rt = Runtime::new().unwrap();

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

                let record_store = _library.library_sync.record_store();
                // Stage 1: Compute fingerprints (0% - 33%)
                let broadcaster_clone = Arc::clone(&broadcaster);
                let progress_path = request_path_clone.to_string();

                let request_path_clone = request_path_clone.to_string();
                compute_file_fingerprints(
                    Arc::clone(&_library.fsio),
                    &_library.main_db,
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
                    &record_store,
                )
                .await?;

                info!(" Compute fingerprints completed.");

                // Stage 2: Compare all pairs (33% - 66%)
                let broadcaster_clone = Arc::clone(&broadcaster);
                let progress_path = request_path_clone.to_string();

                compare_all_pairs(
                    &_library.main_db,
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
                    &record_store,
                )
                .await?;

                info!("Comparing fingerprints completed.");

                // Stage 3: Mark duplicates (66% - 100%)
                if !new_token.is_cancelled() {
                    let broadcaster_clone = Arc::clone(&broadcaster);
                    let progress_path = request_path_clone.to_string();

                    mark_duplicate_files(
                        &_library.main_db,
                        similarity_threshold,
                        move |cur, total| {
                            let progress = 0.66 + cur as f32 / total as f32 * 0.34;

                            broadcaster_clone.broadcast(&DeduplicateAudioLibraryProgress {
                                path: progress_path.clone(),
                                progress: (progress * 100.0) as i32,
                                total: 100,
                            });
                        },
                        &record_store,
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
    use std::time::Duration;
    use tokio::sync::oneshot;

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
        let (release, blocked) = oneshot::channel::<()>();
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
        tokio::time::timeout(Duration::from_secs(1), supervisor)
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
    async fn handler_rejects_request_without_open_library() {
        let capture = Arc::new(Capture::default());
        let tokens = Arc::new(Mutex::new(TaskTokens::default()));
        let request = ScanAudioLibraryRequest {
            scan_id: "handler".into(),
            path: "/tmp".into(),
            force: false,
        };
        let error = request
            .handle(
                (
                    None,
                    Arc::new(uuid::Uuid::new_v4().to_string()),
                    tokens,
                    capture,
                    Arc::new(RwLock::new(SyncSettings::default())),
                ),
                None,
                &request,
            )
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("no library is open on this device"));
    }
}
