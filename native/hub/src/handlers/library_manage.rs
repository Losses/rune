use crate::utils::sync_settings::SyncSettings;
use database::actions::metadata::ScanOptions;
use std::time::Instant;
use std::{
    path::Path,
    sync::{Arc, Mutex as StdMutex, PoisonError},
};
use tokio::runtime::{Handle, Runtime};

use anyhow::Result;
use log::{info, warn};
use tokio::{
    sync::{Mutex, RwLock},
    task,
};
use tokio_util::sync::CancellationToken;

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
    utils::{
        Broadcaster, GlobalParams, LibraryTaskOwner, ParamsExtractor, ScanTask,
        determine_batch_size, require_library,
    },
};

pub struct CloseLibraryParams {
    main_token: Arc<CancellationToken>,
    node: Arc<NodeContext>,
}

impl ParamsExtractor for CloseLibraryRequest {
    type Params = CloseLibraryParams;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        CloseLibraryParams {
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
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let CloseLibraryParams { main_token, node } = params;
        info!("Closing open library");
        node.close_ui(&main_token).await;
        Ok(Some(CloseLibraryResponse { call_id: None }))
    }
}

impl ParamsExtractor for ScanAudioLibraryRequest {
    type Params = (
        Option<LibraryHandle>,
        Arc<String>,
        Arc<Mutex<TaskTokens>>,
        Arc<dyn Broadcaster>,
        Arc<RwLock<SyncSettings>>,
        Arc<StdMutex<LibraryTaskOwner>>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library.clone(),
            Arc::clone(&all_params.node_id),
            Arc::clone(&all_params.task_tokens),
            Arc::clone(&all_params.broadcaster),
            all_params.sync_settings.clone(),
            all_params.library_tasks.clone(),
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
        Arc<StdMutex<LibraryTaskOwner>>,
    );
    type Response = ();

    async fn handle(
        &self,
        (library, node_id, task_tokens, broadcaster, sync_settings, owner): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<()>> {
        let library = require_library(library)?;
        let record_id = library.descriptor.record_id.0.to_string();
        let new_token = {
            let mut tokens = task_tokens.lock().await;
            let owner_guard = owner.lock().unwrap_or_else(PoisonError::into_inner);
            owner_guard.ensure_can_start(
                &record_id,
                LibraryTaskKind::Scan,
                running_scan_token(&tokens, &record_id),
            )?;
            let token = CancellationToken::new();
            if let Some(previous) = tokens.scans.insert(
                record_id.clone(),
                ScanTask {
                    token: token.clone(),
                    scan_id: dart_signal.scan_id.clone(),
                },
            ) {
                previous.token.cancel();
            }
            token
        };

        // Clone all the data we need before spawning the task
        let fs_path = library.media_root.clone();
        // FsIo on Android is rooted at the granted SAF tree, so an empty path
        // addresses the tree root.
        #[cfg(target_os = "android")]
        let scan_path = if fs_path.starts_with("content://") {
            String::new()
        } else {
            fs_path
        };
        #[cfg(not(target_os = "android"))]
        let scan_path = fs_path;
        let request_force = dart_signal.force;
        let missing_host_file = sync_settings.read().await.missing_host_file;
        let main_db_clone = Arc::clone(&library.main_db);
        let broadcaster_clone = Arc::clone(&broadcaster);

        let scan_id = dart_signal.scan_id.clone();
        let worker_id = scan_id.clone();
        let supervisor_token = new_token.clone();
        let supervisor_record_id = record_id.clone();
        let supervisor_broadcaster = broadcaster.clone();
        let supervisor_tokens = task_tokens.clone();
        let scan_runtime = Handle::current();
        let worker = task::spawn_blocking(move || {
            let _library = library;
            scan_runtime.block_on(async move {
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
                                record_id: record_id.clone(),
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
                    let record_id_for_closure = record_id.clone();
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
                                record_id: record_id_for_closure.clone(),
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
            supervisor_record_id,
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
    record_id: String,
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
        warn!("Scan failed for library record {record_id}: {error}");
    }
    if tokens
        .scans
        .get(&record_id)
        .is_some_and(|scan| scan.scan_id == scan_id)
    {
        tokens.scans.remove(&record_id);
    }
    drop(tokens);
    supervisor_broadcaster.broadcast(&ScanAudioLibraryResponse {
        scan_id,
        status: status.to_string(),
        error,
        record_id,
        progress,
    });
}

fn running_scan_token<'a>(
    tokens: &'a TaskTokens,
    record_id: &str,
) -> Option<&'a CancellationToken> {
    tokens
        .scans
        .get(record_id)
        .map(|scan| &scan.token)
        .filter(|token| !token.is_cancelled())
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
        Arc<Mutex<TaskTokens>>,
        Arc<StdMutex<LibraryTaskOwner>>,
        Arc<dyn Broadcaster>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library.clone(),
            Arc::clone(&all_params.task_tokens),
            all_params.library_tasks.clone(),
            Arc::clone(&all_params.broadcaster),
        )
    }
}

impl Signal for AnalyzeAudioLibraryRequest {
    type Params = (
        Option<LibraryHandle>,
        Arc<Mutex<TaskTokens>>,
        Arc<StdMutex<LibraryTaskOwner>>,
        Arc<dyn Broadcaster>,
    );
    type Response = ();

    async fn handle(
        &self,
        (library, task_tokens, owner, broadcaster): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<()>> {
        let library = require_library(library)?;
        let record_id = library.descriptor.record_id.0.to_string();
        let task_tokens = task_tokens.lock().await;
        let mut owner_guard = owner.lock().unwrap_or_else(PoisonError::into_inner);
        let (state, token) = owner_guard.start(
            &record_id,
            LibraryTaskKind::Analyze,
            running_scan_token(&task_tokens, &record_id),
        )?;
        drop(task_tokens);
        broadcaster.broadcast(&LibraryTaskStateChanged {
            state: state.clone(),
        });
        let run_id = match &state {
            LibraryTaskState::Running { run_id, .. } => run_id.clone(),
            _ => anyhow::bail!("Task owner did not return a running analysis"),
        };
        let fs_path = library.media_root.clone();
        #[cfg(target_os = "android")]
        let analysis_path = if fs_path.starts_with("content://") {
            String::new()
        } else {
            fs_path
        };
        #[cfg(not(target_os = "android"))]
        let analysis_path = fs_path;
        let batch_size = determine_batch_size(dart_signal.workload_factor);
        let computing_device = dart_signal.computing_device;
        let worker_owner = owner.clone();
        let worker_broadcaster = broadcaster.clone();
        let worker_record = record_id.clone();
        let worker_run = run_id.clone();
        let worker = task::spawn(async move {
            let callback_owner = worker_owner.clone();
            let callback_broadcaster = worker_broadcaster.clone();
            let callback_record = worker_record.clone();
            let callback_run = worker_run.clone();
            let joined = task::spawn_blocking(move || {
                let runtime = Runtime::new().map_err(|_| LibraryTaskError::WorkerFailed)?;
                runtime.block_on(async move {
                    let total = analysis_audio_library(
                        Arc::clone(&library.fsio),
                        &library.main_db,
                        Path::new(&analysis_path),
                        batch_size,
                        computing_device.into(),
                        move |progress, total| {
                            let state = callback_owner
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .progress(
                                    &callback_record,
                                    LibraryTaskKind::Analyze,
                                    &callback_run,
                                    progress as i32,
                                    total as i32,
                                );
                            if let Some(state) = state {
                                callback_broadcaster.broadcast(&LibraryTaskStateChanged { state });
                            }
                        },
                        Some(token.clone()),
                    )
                    .await
                    .map_err(|_| LibraryTaskError::AnalysisFailed)?;
                    sync_recommendation(&library.main_db, &library.recommend_db)
                        .await
                        .map_err(|_| LibraryTaskError::RecommendationSyncFailed)?;
                    Ok(total as i32)
                })
            })
            .await;
            let result = match joined {
                Ok(result) => result,
                Err(_) => Err(LibraryTaskError::WorkerFailed),
            };
            let (outcome, progress, total) = match result {
                Ok(total) => (LibraryTaskOutcome::Finished, total, total),
                Err(error) => (LibraryTaskOutcome::Failed(error), 0, 0),
            };
            let state = worker_owner
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .end(
                    &worker_record,
                    LibraryTaskKind::Analyze,
                    &worker_run,
                    outcome,
                    progress,
                    total,
                );
            if let Some(state) = state {
                worker_broadcaster.broadcast(&LibraryTaskStateChanged { state });
            }
        });
        owner_guard.attach_worker(&record_id, LibraryTaskKind::Analyze, &run_id, worker);
        drop(owner_guard);
        Ok(Some(()))
    }
}

impl ParamsExtractor for DeduplicateAudioLibraryRequest {
    type Params = (
        Option<LibraryHandle>,
        Arc<Mutex<TaskTokens>>,
        Arc<StdMutex<LibraryTaskOwner>>,
        Arc<dyn Broadcaster>,
    );
    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library.clone(),
            Arc::clone(&all_params.task_tokens),
            all_params.library_tasks.clone(),
            Arc::clone(&all_params.broadcaster),
        )
    }
}

impl Signal for DeduplicateAudioLibraryRequest {
    type Params = (
        Option<LibraryHandle>,
        Arc<Mutex<TaskTokens>>,
        Arc<StdMutex<LibraryTaskOwner>>,
        Arc<dyn Broadcaster>,
    );
    type Response = ();
    async fn handle(
        &self,
        (library, task_tokens, owner, broadcaster): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<()>> {
        let library = require_library(library)?;
        let record_id = library.descriptor.record_id.0.to_string();
        let task_tokens = task_tokens.lock().await;
        let mut owner_guard = owner.lock().unwrap_or_else(PoisonError::into_inner);
        let (state, token) = owner_guard.start(
            &record_id,
            LibraryTaskKind::Deduplicate,
            running_scan_token(&task_tokens, &record_id),
        )?;
        drop(task_tokens);
        broadcaster.broadcast(&LibraryTaskStateChanged {
            state: state.clone(),
        });
        let run_id = match &state {
            LibraryTaskState::Running { run_id, .. } => run_id.clone(),
            _ => return Ok(None),
        };
        let fs_path = library.media_root.clone();
        #[cfg(target_os = "android")]
        let deduplicate_path = if fs_path.starts_with("content://") {
            String::new()
        } else {
            fs_path
        };
        #[cfg(not(target_os = "android"))]
        let deduplicate_path = fs_path;
        let batch_size = determine_batch_size(dart_signal.workload_factor);
        let similarity_threshold = dart_signal.similarity_threshold;
        let config = Configuration::default();
        let record_store = library.library_sync.record_store();
        let worker_owner = owner.clone();
        let worker_broadcaster = broadcaster.clone();
        let worker_record = record_id.clone();
        let worker_run = run_id.clone();
        let worker = task::spawn(async move {
            let worker_owner_progress = worker_owner.clone();
            let worker_broadcaster_progress = worker_broadcaster.clone();
            let record_progress = worker_record.clone();
            let run_progress = worker_run.clone();
            let result = task::spawn_blocking(move || {
                let runtime = Runtime::new().map_err(|_| LibraryTaskError::WorkerFailed)?;
                runtime.block_on(async move {
                    let progress_owner = worker_owner_progress;
                    let progress_broadcaster = worker_broadcaster_progress;
                    let progress_record = record_progress;
                    let progress_run = run_progress;
                    let progress = move |value: i32| {
                        let state = progress_owner
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .progress(
                                &progress_record,
                                LibraryTaskKind::Deduplicate,
                                &progress_run,
                                value,
                                100,
                            );
                        if let Some(state) = state {
                            progress_broadcaster.broadcast(&LibraryTaskStateChanged { state });
                        }
                    };
                    compute_file_fingerprints(
                        Arc::clone(&library.fsio),
                        &library.main_db,
                        Path::new(&deduplicate_path),
                        batch_size,
                        {
                            let progress = progress.clone();
                            move |cur, total| progress((cur as f32 / total as f32 * 33.0) as i32)
                        },
                        Some(token.clone()),
                        &record_store,
                    )
                    .await
                    .map_err(|_| LibraryTaskError::FingerprintingFailed)?;
                    compare_all_pairs(
                        &library.main_db,
                        {
                            let progress = progress.clone();
                            move |cur, total| {
                                progress(33 + (cur as f32 / total as f32 * 33.0) as i32)
                            }
                        },
                        &config,
                        Some(Arc::new(token.clone())),
                        &record_store,
                    )
                    .await
                    .map_err(|_| LibraryTaskError::ComparisonFailed)?;
                    if !token.is_cancelled() {
                        mark_duplicate_files(
                            &library.main_db,
                            similarity_threshold,
                            move |cur, total| {
                                progress(66 + (cur as f32 / total as f32 * 34.0) as i32)
                            },
                            &record_store,
                        )
                        .await
                        .map_err(|_| LibraryTaskError::MarkingDuplicatesFailed)?;
                    }
                    Ok::<(), LibraryTaskError>(())
                })
            })
            .await;
            let (outcome, progress) = match result {
                Ok(Ok(())) => (LibraryTaskOutcome::Finished, 100),
                Ok(Err(error)) => (LibraryTaskOutcome::Failed(error), 0),
                Err(error) => {
                    log::error!("Deduplication worker failed: {error}");
                    (
                        LibraryTaskOutcome::Failed(LibraryTaskError::WorkerFailed),
                        0,
                    )
                }
            };
            let state = worker_owner
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .end(
                    &worker_record,
                    LibraryTaskKind::Deduplicate,
                    &worker_run,
                    outcome,
                    progress,
                    100,
                );
            if let Some(state) = state {
                worker_broadcaster.broadcast(&LibraryTaskStateChanged { state });
            }
        });
        owner_guard.attach_worker(&record_id, LibraryTaskKind::Deduplicate, &run_id, worker);
        drop(owner_guard);
        Ok(Some(()))
    }
}

impl ParamsExtractor for CancelTaskRequest {
    type Params = (
        Arc<Mutex<TaskTokens>>,
        Arc<StdMutex<LibraryTaskOwner>>,
        Arc<dyn Broadcaster>,
    );
    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.task_tokens),
            all_params.library_tasks.clone(),
            Arc::clone(&all_params.broadcaster),
        )
    }
}

impl Signal for CancelTaskRequest {
    type Params = (
        Arc<Mutex<TaskTokens>>,
        Arc<StdMutex<LibraryTaskOwner>>,
        Arc<dyn Broadcaster>,
    );
    type Response = CancelTaskResponse;
    async fn handle(
        &self,
        (task_tokens, owner, broadcaster): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let result = match request.r#type {
            CancelTaskType::AnalyzeAudioLibrary => {
                let state =
                    LibraryTaskOwner::cancel(&owner, &request.record_id, LibraryTaskKind::Analyze)
                        .await
                        .unwrap_or_else(|| {
                            owner
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .current(&request.record_id, LibraryTaskKind::Analyze)
                        });
                broadcaster.broadcast(&LibraryTaskStateChanged {
                    state: state.clone(),
                });
                CancelTaskResult::Library(state)
            }
            CancelTaskType::DeduplicateAudioLibrary => {
                let state = LibraryTaskOwner::cancel(
                    &owner,
                    &request.record_id,
                    LibraryTaskKind::Deduplicate,
                )
                .await
                .unwrap_or_else(|| {
                    owner
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .current(&request.record_id, LibraryTaskKind::Deduplicate)
                });
                broadcaster.broadcast(&LibraryTaskStateChanged {
                    state: state.clone(),
                });
                CancelTaskResult::Library(state)
            }
            CancelTaskType::ScanAudioLibrary => {
                let mut tokens = task_tokens.lock().await;
                let cancelled = request.scan_id.as_ref().is_some_and(|scan_id| {
                    tokens
                        .scans
                        .get(&request.record_id)
                        .is_some_and(|scan| &scan.scan_id == scan_id)
                });
                if cancelled && let Some(scan) = tokens.scans.remove(&request.record_id) {
                    scan.token.cancel();
                }
                if cancelled {
                    CancelTaskResult::ScanCancelled
                } else {
                    CancelTaskResult::ScanUnchanged
                }
            }
        };
        Ok(Some(CancelTaskResponse {
            call_id: None,
            record_id: request.record_id.clone(),
            r#type: request.r#type,
            result,
        }))
    }
}

#[cfg(test)]
mod scan_supervisor_tests {

    use super::*;
    use crate::library_pool::LibraryPool;
    use crate::request_error::ExpectedRequestError;
    use crate::utils::RinfRustSignal;
    use database::library_registry::{LibraryRegistry, create_replica};
    use std::collections::HashMap;
    use std::time::Duration;
    use tokio::sync::oneshot;
    use uuid::Uuid;

    async fn open_test_library() -> Result<(tempfile::TempDir, String, LibraryHandle)> {
        let config = tempfile::TempDir::new()?;
        let id = Uuid::new_v4();
        let mut registry = LibraryRegistry::load(config.path())?;
        create_replica(config.path(), &mut registry, id, "Replica", "owner").await?;
        let node_id = Uuid::new_v4().to_string();
        let pool = LibraryPool::new(config.path(), config.path(), &node_id)?;
        let library = pool.acquire_ui(id).await?;
        Ok((config, node_id, library))
    }

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
            let scan_id = if outcome == "replacement" {
                "new"
            } else {
                "old"
            };
            let tokens = Arc::new(Mutex::new(TaskTokens {
                scans: HashMap::from([(
                    "library".to_owned(),
                    ScanTask {
                        token: if outcome == "replacement" {
                            replacement.clone()
                        } else {
                            token.clone()
                        },
                        scan_id: scan_id.into(),
                    },
                )]),
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
                let scan = tokens.scans.get("library").unwrap();
                assert_eq!(scan.scan_id, "new");
                assert!(!replacement.is_cancelled());
            } else {
                assert!(!tokens.scans.contains_key("library"));
            }
        }
    }
    #[tokio::test]
    async fn cancellation_reports_while_worker_is_blocked() {
        let token = CancellationToken::new();
        let tokens = Arc::new(Mutex::new(TaskTokens {
            scans: HashMap::from([(
                "library".into(),
                ScanTask {
                    scan_id: "old".into(),
                    token: token.clone(),
                },
            )]),
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
            scans: HashMap::from([(
                "library".into(),
                ScanTask {
                    scan_id: "new".into(),
                    token: replacement.clone(),
                },
            )]),
        }));
        let request = CancelTaskRequest {
            call_id: None,
            record_id: "library".into(),
            scan_id: Some("old".into()),
            r#type: CancelTaskType::ScanAudioLibrary,
        };
        let owner = Arc::new(StdMutex::new(LibraryTaskOwner::default()));
        let capture = Arc::new(Capture::default());
        let response = request
            .handle((tokens.clone(), owner, capture), None, &request)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.result, CancelTaskResult::ScanUnchanged);
        assert!(!replacement.is_cancelled());
        assert_eq!(tokens.lock().await.scans["library"].scan_id, "new");
    }
    #[tokio::test]
    async fn handler_rejects_request_without_open_library() {
        let capture = Arc::new(Capture::default());
        let tokens = Arc::new(Mutex::new(TaskTokens::default()));
        let request = ScanAudioLibraryRequest {
            call_id: None,
            scan_id: "handler".into(),
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
                    Arc::new(StdMutex::new(LibraryTaskOwner::default())),
                ),
                None,
                &request,
            )
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("no library is open on this device"));
    }

    #[tokio::test]
    async fn analysis_start_while_scanning_is_refused_without_cancelling_scan() -> Result<()> {
        let (_config, _node_id, library) = open_test_library().await?;
        let record_id = library.descriptor.record_id.0.to_string();
        let scan_token = CancellationToken::new();
        let tokens = Arc::new(Mutex::new(TaskTokens {
            scans: HashMap::from([(
                record_id.clone(),
                ScanTask {
                    scan_id: "active-scan".into(),
                    token: scan_token.clone(),
                },
            )]),
        }));
        let owner = Arc::new(StdMutex::new(LibraryTaskOwner::default()));
        let request = AnalyzeAudioLibraryRequest {
            call_id: None,
            computing_device: ComputingDeviceRequest::Cpu,
            workload_factor: 0.5,
        };
        let error = request
            .handle(
                (
                    Some(library),
                    tokens.clone(),
                    owner.clone(),
                    Arc::new(Capture::default()),
                ),
                None,
                &request,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::LibraryTaskAlreadyRunning {
                task: LibraryTaskKind::Scan
            })
        ));
        assert!(!scan_token.is_cancelled());
        let tokens = tokens.lock().await;
        assert_eq!(tokens.scans[&record_id].scan_id, "active-scan");
        drop(tokens);
        assert!(matches!(
            owner
                .lock()
                .unwrap()
                .current(&record_id, LibraryTaskKind::Analyze),
            LibraryTaskState::NotRunning { .. }
        ));
        Ok(())
    }

    #[tokio::test]
    async fn scan_start_while_deduplicating_is_refused_without_changing_deduplication() -> Result<()>
    {
        let (_config, node_id, library) = open_test_library().await?;
        let record_id = library.descriptor.record_id.0.to_string();
        let owner = Arc::new(StdMutex::new(LibraryTaskOwner::default()));
        let (running_state, dedup_token) =
            owner
                .lock()
                .unwrap()
                .start(&record_id, LibraryTaskKind::Deduplicate, None)?;
        let tokens = Arc::new(Mutex::new(TaskTokens::default()));
        let request = ScanAudioLibraryRequest {
            call_id: None,
            scan_id: "refused-scan".into(),
            force: false,
        };
        let error = request
            .handle(
                (
                    Some(library),
                    Arc::new(node_id),
                    tokens.clone(),
                    Arc::new(Capture::default()),
                    Arc::new(RwLock::new(SyncSettings::default())),
                    owner.clone(),
                ),
                None,
                &request,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::LibraryTaskAlreadyRunning {
                task: LibraryTaskKind::Deduplicate
            })
        ));
        assert!(!dedup_token.is_cancelled());
        assert_eq!(
            owner
                .lock()
                .unwrap()
                .current(&record_id, LibraryTaskKind::Deduplicate),
            running_state
        );
        let tokens = tokens.lock().await;
        assert!(tokens.scans.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn scanning_an_open_replica_scans_its_folder() -> Result<()> {
        let (_config, node_id, library) = open_test_library().await?;
        let audio = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");
        std::fs::copy(audio, Path::new(&library.media_root).join("track.ogg"))?;
        let tokens = Arc::new(Mutex::new(TaskTokens::default()));
        let capture = Arc::new(Capture::default());
        let params = || {
            (
                Some(library.clone()),
                Arc::new(node_id.clone()),
                tokens.clone(),
                capture.clone() as Arc<dyn Broadcaster>,
                Arc::new(RwLock::new(SyncSettings::default())),
                Arc::new(StdMutex::new(LibraryTaskOwner::default())),
            )
        };
        let session = || Session {
            public_key: String::new(),
            fingerprint: "controller".into(),
            host: "127.0.0.1".into(),
        };

        let request = ScanAudioLibraryRequest {
            call_id: None,
            scan_id: "replica".into(),
            force: false,
        };
        request.handle(params(), Some(session()), &request).await?;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if !capture.0.lock().unwrap().is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        let events = capture.0.lock().unwrap();
        let response: ScanAudioLibraryResponse = rinf::deserialize(&events[0])?;
        assert_eq!(response.status, "success");
        assert_eq!(response.progress, 1);
        Ok(())
    }
}
