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
use tokio::{sync::RwLock, task};
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
    Session, Signal,
    library_pool::LibraryHandle,
    messages::*,
    node::NodeContext,
    utils::{
        Broadcaster, GlobalParams, LibraryTaskOwner, ParamsExtractor, determine_batch_size,
        require_library,
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
        Arc<dyn Broadcaster>,
        Arc<RwLock<SyncSettings>>,
        Arc<StdMutex<LibraryTaskOwner>>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library.clone(),
            Arc::clone(&all_params.node_id),
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
        Arc<dyn Broadcaster>,
        Arc<RwLock<SyncSettings>>,
        Arc<StdMutex<LibraryTaskOwner>>,
    );
    type Response = ();

    async fn handle(
        &self,
        (library, node_id, broadcaster, sync_settings, owner): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<()>> {
        let library = require_library(library)?;
        let record_id = library.descriptor.record_id.0.to_string();
        let (state, new_token) = owner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .start(&record_id, LibraryTaskKind::Scan)?;
        broadcaster.broadcast(&LibraryTaskStateChanged {
            state: state.clone(),
        });
        let LibraryTaskState::ScanRunning { run_id, .. } = state else {
            anyhow::bail!("Task owner did not return a running scan");
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

        let worker_owner = owner.clone();
        let worker_record_id = record_id.clone();
        let worker_run_id = run_id.clone();
        let blocking_record_id = record_id.clone();
        let blocking_run_id = run_id.clone();
        let worker_broadcaster = broadcaster.clone();
        let scan_runtime = Handle::current();
        let worker = task::spawn(async move {
            let blocking_owner = worker_owner.clone();
            let result = task::spawn_blocking(move || {
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
                                let state = blocking_owner
                                    .lock()
                                    .unwrap_or_else(PoisonError::into_inner)
                                    .progress(
                                        &blocking_record_id,
                                        LibraryTaskKind::Scan,
                                        &blocking_run_id,
                                        progress as i32,
                                        0,
                                    );
                                if let Some(state) = state {
                                    broadcaster_clone.broadcast(&LibraryTaskStateChanged { state });
                                }
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
                        let record_id_for_closure = blocking_record_id.clone();
                        let owner_for_closure = blocking_owner.clone();
                        let run_for_closure = blocking_run_id.clone();

                        let phase = owner_for_closure
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .set_scan_phase(
                                &record_id_for_closure,
                                &run_for_closure,
                                ScanTaskPhase::ScanCoverArts,
                            );
                        if let Some(state) = phase {
                            cloned_broadcaster.broadcast(&LibraryTaskStateChanged { state });
                        }

                        scan_cover_arts(
                            Arc::clone(&_library.fsio),
                            &main_db_clone,
                            Path::new(&scan_path),
                            batch_size,
                            move |now, total| {
                                let state = owner_for_closure
                                    .lock()
                                    .unwrap_or_else(PoisonError::into_inner)
                                    .progress(
                                        &record_id_for_closure,
                                        LibraryTaskKind::Scan,
                                        &run_for_closure,
                                        now as i32,
                                        total as i32,
                                    );
                                if let Some(state) = state {
                                    cloned_broadcaster
                                        .broadcast(&LibraryTaskStateChanged { state });
                                }
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
            })
            .await;
            let (outcome, progress, total) = match result {
                Ok(Ok(count)) => (ScanOutcome::Finished, count as i32, count as i32),
                Ok(Err(error)) => {
                    warn!("Scan failed for library record {worker_record_id}: {error:#}");
                    (ScanOutcome::Failed(ScanFailure::ScanFailed), 0, 0)
                }
                Err(error) => {
                    warn!("Scan worker failed for library record {worker_record_id}: {error}");
                    (ScanOutcome::Failed(ScanFailure::WorkerFailed), 0, 0)
                }
            };
            let state = worker_owner
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .end(
                    &worker_record_id,
                    &worker_run_id,
                    LibraryTaskEnd::Scan(outcome),
                    progress,
                    total,
                );
            if let Some(state) = state {
                worker_broadcaster.broadcast(&LibraryTaskStateChanged { state });
            }
        });
        owner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .attach_worker(&record_id, LibraryTaskKind::Scan, &run_id, worker);

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
        Option<LibraryHandle>,
        Arc<StdMutex<LibraryTaskOwner>>,
        Arc<dyn Broadcaster>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library.clone(),
            all_params.library_tasks.clone(),
            Arc::clone(&all_params.broadcaster),
        )
    }
}

impl Signal for AnalyzeAudioLibraryRequest {
    type Params = (
        Option<LibraryHandle>,
        Arc<StdMutex<LibraryTaskOwner>>,
        Arc<dyn Broadcaster>,
    );
    type Response = ();

    async fn handle(
        &self,
        (library, owner, broadcaster): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<()>> {
        let library = require_library(library)?;
        let record_id = library.descriptor.record_id.0.to_string();
        let mut owner_guard = owner.lock().unwrap_or_else(PoisonError::into_inner);
        let (state, token) = owner_guard.start(&record_id, LibraryTaskKind::Analyze)?;
        broadcaster.broadcast(&LibraryTaskStateChanged {
            state: state.clone(),
        });
        let LibraryTaskState::AnalysisRunning { run_id, .. } = &state else {
            anyhow::bail!("Task owner did not return a running analysis");
        };
        let run_id = run_id.clone();
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
                let runtime = Runtime::new().map_err(|_| AnalysisFailure::WorkerFailed)?;
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
                    .map_err(|_| AnalysisFailure::AnalysisFailed)?;
                    sync_recommendation(&library.main_db, &library.recommend_db)
                        .await
                        .map_err(|_| AnalysisFailure::RecommendationSyncFailed)?;
                    Ok(total as i32)
                })
            })
            .await;
            let result = match joined {
                Ok(result) => result,
                Err(_) => Err(AnalysisFailure::WorkerFailed),
            };
            let (outcome, progress, total) = match result {
                Ok(total) => (AnalysisOutcome::Finished, total, total),
                Err(error) => (AnalysisOutcome::Failed(error), 0, 0),
            };
            let state = worker_owner
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .end(
                    &worker_record,
                    &worker_run,
                    LibraryTaskEnd::Analysis(outcome),
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
        Arc<StdMutex<LibraryTaskOwner>>,
        Arc<dyn Broadcaster>,
    );
    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library.clone(),
            all_params.library_tasks.clone(),
            Arc::clone(&all_params.broadcaster),
        )
    }
}

impl Signal for DeduplicateAudioLibraryRequest {
    type Params = (
        Option<LibraryHandle>,
        Arc<StdMutex<LibraryTaskOwner>>,
        Arc<dyn Broadcaster>,
    );
    type Response = ();
    async fn handle(
        &self,
        (library, owner, broadcaster): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<()>> {
        let library = require_library(library)?;
        let record_id = library.descriptor.record_id.0.to_string();
        let mut owner_guard = owner.lock().unwrap_or_else(PoisonError::into_inner);
        let (state, token) = owner_guard.start(&record_id, LibraryTaskKind::Deduplicate)?;
        broadcaster.broadcast(&LibraryTaskStateChanged {
            state: state.clone(),
        });
        let LibraryTaskState::DeduplicationRunning { run_id, .. } = &state else {
            return Ok(None);
        };
        let run_id = run_id.clone();
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
                let runtime = Runtime::new().map_err(|_| DeduplicationFailure::WorkerFailed)?;
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
                    .map_err(|_| DeduplicationFailure::FingerprintingFailed)?;
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
                    .map_err(|_| DeduplicationFailure::ComparisonFailed)?;
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
                        .map_err(|_| DeduplicationFailure::MarkingDuplicatesFailed)?;
                    }
                    Ok::<(), DeduplicationFailure>(())
                })
            })
            .await;
            let (outcome, progress) = match result {
                Ok(Ok(())) => (DeduplicationOutcome::Finished, 100),
                Ok(Err(error)) => (DeduplicationOutcome::Failed(error), 0),
                Err(error) => {
                    log::error!("Deduplication worker failed: {error}");
                    (
                        DeduplicationOutcome::Failed(DeduplicationFailure::WorkerFailed),
                        0,
                    )
                }
            };
            let state = worker_owner
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .end(
                    &worker_record,
                    &worker_run,
                    LibraryTaskEnd::Deduplication(outcome),
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
    type Params = (Arc<StdMutex<LibraryTaskOwner>>, Arc<dyn Broadcaster>);
    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library_tasks.clone(),
            Arc::clone(&all_params.broadcaster),
        )
    }
}

impl Signal for CancelTaskRequest {
    type Params = (Arc<StdMutex<LibraryTaskOwner>>, Arc<dyn Broadcaster>);
    type Response = CancelTaskResponse;
    async fn handle(
        &self,
        (owner, broadcaster): Self::Params,
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
                state
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
                state
            }
            CancelTaskType::ScanAudioLibrary => {
                let state =
                    LibraryTaskOwner::cancel(&owner, &request.record_id, LibraryTaskKind::Scan)
                        .await
                        .unwrap_or_else(|| {
                            owner
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .current(&request.record_id, LibraryTaskKind::Scan)
                        });
                broadcaster.broadcast(&LibraryTaskStateChanged {
                    state: state.clone(),
                });
                state
            }
        };
        Ok(Some(CancelTaskResponse {
            call_id: None,
            record_id: request.record_id.clone(),
            r#type: request.r#type,
            state: result,
        }))
    }
}

#[cfg(test)]
mod scan_task_tests {
    use super::*;
    use crate::library_pool::LibraryPool;
    use crate::request_error::ExpectedRequestError;
    use crate::test_wait::{TEST_DEADLINE, wait_until};
    use crate::utils::RinfRustSignal;
    use database::actions::metadata::{MissingHostFilePolicy, ScanOptions, scan_audio_library};
    use database::library_registry::{LibraryRegistry, create_replica};
    use sea_orm::ConnectionTrait;
    use tokio::{
        sync::{Notify, mpsc::unbounded_channel, oneshot},
        time::timeout,
    };
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

    async fn open_test_library_with_audio_files(
        names: &[&str],
    ) -> Result<(tempfile::TempDir, String, LibraryHandle)> {
        let (config, node_id, library) = open_test_library().await?;
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");
        for name in names {
            std::fs::copy(&source, Path::new(&library.media_root).join(name))?;
        }
        let store = library.library_sync.record_store();
        scan_audio_library(
            &library.fsio,
            &library.main_db,
            &node_id,
            Path::new(&library.media_root),
            ScanOptions {
                cleanup: false,
                force: false,
                missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
                cancel_token: None,
            },
            |_| {},
            &store,
        )
        .await?;
        Ok((config, node_id, library))
    }

    #[derive(Default)]
    struct Capture {
        states: std::sync::Mutex<Vec<LibraryTaskState>>,
        changed: Notify,
    }

    impl Broadcaster for Capture {
        fn broadcast(&self, message: &dyn RinfRustSignal) {
            if message.name() == "LibraryTaskStateChanged"
                && let Ok(bytes) = message.encode_to_vec()
                && let Ok(event) = rinf::deserialize::<LibraryTaskStateChanged>(&bytes)
            {
                self.states.lock().unwrap().push(event.state);
                self.changed.notify_one();
            }
        }
    }

    impl Capture {
        fn states(&self) -> Vec<LibraryTaskState> {
            self.states.lock().unwrap().clone()
        }
    }

    async fn wait_for_task_outcome(
        capture: &Capture,
        task: LibraryTaskKind,
    ) -> Result<(LibraryTaskEnd, i32)> {
        wait_until("library task outcome", || async {
            Ok(capture.states().into_iter().find_map(|state| {
                let ended = match state {
                    LibraryTaskState::ScanEnded {
                        outcome, progress, ..
                    } => (LibraryTaskEnd::Scan(outcome), progress),
                    LibraryTaskState::AnalysisEnded {
                        outcome, progress, ..
                    } => (LibraryTaskEnd::Analysis(outcome), progress),
                    LibraryTaskState::DeduplicationEnded {
                        outcome, progress, ..
                    } => (LibraryTaskEnd::Deduplication(outcome), progress),
                    _ => return None,
                };
                (ended.0.kind() == task).then_some(ended)
            }))
        })
        .await
    }

    #[tokio::test]
    async fn analysis_write_failure_ends_failed_with_analysis_error() -> Result<()> {
        let (_config, _node_id, library) =
            open_test_library_with_audio_files(&["track.ogg"]).await?;
        library
            .main_db
            .execute_unprepared(
                "CREATE TRIGGER reject_analysis BEFORE INSERT ON media_analysis BEGIN SELECT RAISE(ABORT, 'test failure'); END",
            )
            .await?;
        let owner = Arc::new(StdMutex::new(LibraryTaskOwner::default()));
        let capture = Arc::new(Capture::default());
        let request = AnalyzeAudioLibraryRequest {
            call_id: None,
            computing_device: ComputingDeviceRequest::Cpu,
            workload_factor: 1.0,
        };
        request
            .handle((Some(library), owner, capture.clone()), None, &request)
            .await?;

        assert_eq!(
            wait_for_task_outcome(&capture, LibraryTaskKind::Analyze)
                .await?
                .0,
            LibraryTaskEnd::Analysis(AnalysisOutcome::Failed(AnalysisFailure::AnalysisFailed))
        );
        Ok(())
    }

    #[tokio::test]
    async fn fingerprint_write_failure_ends_failed_with_fingerprinting_error() -> Result<()> {
        let (_config, _node_id, library) =
            open_test_library_with_audio_files(&["track.ogg"]).await?;
        library
            .main_db
            .execute_unprepared(
                "CREATE TRIGGER reject_fingerprint BEFORE INSERT ON media_file_fingerprint BEGIN SELECT RAISE(ABORT, 'test failure'); END",
            )
            .await?;
        let owner = Arc::new(StdMutex::new(LibraryTaskOwner::default()));
        let capture = Arc::new(Capture::default());
        let request = DeduplicateAudioLibraryRequest {
            call_id: None,
            workload_factor: 1.0,
            similarity_threshold: 0.8,
        };
        request
            .handle((Some(library), owner, capture.clone()), None, &request)
            .await?;

        assert_eq!(
            wait_for_task_outcome(&capture, LibraryTaskKind::Deduplicate)
                .await?
                .0,
            LibraryTaskEnd::Deduplication(DeduplicationOutcome::Failed(
                DeduplicationFailure::FingerprintingFailed
            ))
        );
        Ok(())
    }

    #[tokio::test]
    async fn undecodable_audio_finishes_analysis_and_is_retried() -> Result<()> {
        use database::entities::{media_analysis, media_files};
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

        let (_config, _node_id, library) =
            open_test_library_with_audio_files(&["bad.ogg", "good.ogg"]).await?;
        std::fs::write(
            Path::new(&library.media_root).join("bad.ogg"),
            b"not an audio file",
        )?;
        let bad = media_files::Entity::find()
            .filter(media_files::Column::FileName.eq("bad.ogg"))
            .one(library.main_db.as_ref())
            .await?
            .expect("bad audio file was indexed");
        let good = media_files::Entity::find()
            .filter(media_files::Column::FileName.eq("good.ogg"))
            .one(library.main_db.as_ref())
            .await?
            .expect("good audio file was indexed");

        for expected_progress in [2, 1] {
            let owner = Arc::new(StdMutex::new(LibraryTaskOwner::default()));
            let capture = Arc::new(Capture::default());
            let request = AnalyzeAudioLibraryRequest {
                call_id: None,
                computing_device: ComputingDeviceRequest::Cpu,
                workload_factor: 1.0,
            };
            request
                .handle(
                    (Some(library.clone()), owner, capture.clone()),
                    None,
                    &request,
                )
                .await?;
            let (outcome, progress) =
                wait_for_task_outcome(&capture, LibraryTaskKind::Analyze).await?;
            assert_eq!(outcome, LibraryTaskEnd::Analysis(AnalysisOutcome::Finished));
            assert_eq!(progress, expected_progress);
            let analyses = media_analysis::Entity::find()
                .all(library.main_db.as_ref())
                .await?;
            assert_eq!(analyses.len(), 1);
            assert_eq!(analyses[0].file_id, good.id);
            assert!(analyses.iter().all(|analysis| analysis.file_id != bad.id));
        }
        Ok(())
    }

    async fn start_scan(
        library: LibraryHandle,
        node_id: String,
        owner: Arc<StdMutex<LibraryTaskOwner>>,
        force: bool,
    ) -> Result<Option<()>> {
        let request = ScanAudioLibraryRequest {
            call_id: None,
            force,
        };
        request
            .handle(
                (
                    Some(library),
                    Arc::new(node_id),
                    Arc::new(Capture::default()),
                    Arc::new(RwLock::new(SyncSettings::default())),
                    owner,
                ),
                None,
                &request,
            )
            .await
    }

    #[tokio::test]
    async fn scan_lifecycle_force_scan_while_analysis_runs_is_refused_without_changing_analysis()
    -> Result<()> {
        let (_config, node_id, library) = open_test_library().await?;
        let record_id = library.descriptor.record_id.0.to_string();
        let owner = Arc::new(StdMutex::new(LibraryTaskOwner::default()));
        let (running, token) = owner
            .lock()
            .unwrap()
            .start(&record_id, LibraryTaskKind::Analyze)?;
        let error = start_scan(library, node_id, owner.clone(), true)
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::LibraryTaskAlreadyRunning {
                task: LibraryTaskKind::Analyze
            })
        ));
        assert!(!token.is_cancelled());
        assert_eq!(
            owner
                .lock()
                .unwrap()
                .current(&record_id, LibraryTaskKind::Analyze),
            running
        );
        assert!(matches!(
            owner
                .lock()
                .unwrap()
                .current(&record_id, LibraryTaskKind::Scan),
            LibraryTaskState::NotRunning { .. }
        ));
        Ok(())
    }

    #[tokio::test]
    async fn scan_lifecycle_second_start_while_scanning_is_refused_without_changing_the_scan()
    -> Result<()> {
        let (_config, node_id, library) = open_test_library().await?;
        let record_id = library.descriptor.record_id.0.to_string();
        let owner = Arc::new(StdMutex::new(LibraryTaskOwner::default()));
        let (running, token) = owner
            .lock()
            .unwrap()
            .start(&record_id, LibraryTaskKind::Scan)?;
        let error = start_scan(library, node_id, owner.clone(), false)
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::LibraryTaskAlreadyRunning {
                task: LibraryTaskKind::Scan
            })
        ));
        assert!(!token.is_cancelled());
        assert_eq!(
            owner
                .lock()
                .unwrap()
                .current(&record_id, LibraryTaskKind::Scan),
            running
        );
        Ok(())
    }

    #[tokio::test]
    async fn scan_lifecycle_analysis_start_while_scanning_is_refused_without_cancelling_the_scan()
    -> Result<()> {
        let (_config, _node_id, library) = open_test_library().await?;
        let record_id = library.descriptor.record_id.0.to_string();
        let owner = Arc::new(StdMutex::new(LibraryTaskOwner::default()));
        let (running, token) = owner
            .lock()
            .unwrap()
            .start(&record_id, LibraryTaskKind::Scan)?;
        let request = AnalyzeAudioLibraryRequest {
            call_id: None,
            computing_device: ComputingDeviceRequest::Cpu,
            workload_factor: 0.5,
        };
        let error = request
            .handle(
                (Some(library), owner.clone(), Arc::new(Capture::default())),
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
        assert!(!token.is_cancelled());
        assert_eq!(
            owner
                .lock()
                .unwrap()
                .current(&record_id, LibraryTaskKind::Scan),
            running
        );
        Ok(())
    }

    #[tokio::test]
    async fn scan_lifecycle_start_while_deduplicating_is_refused_without_changing_deduplication()
    -> Result<()> {
        let (_config, node_id, library) = open_test_library().await?;
        let record_id = library.descriptor.record_id.0.to_string();
        let owner = Arc::new(StdMutex::new(LibraryTaskOwner::default()));
        let (running, token) = owner
            .lock()
            .unwrap()
            .start(&record_id, LibraryTaskKind::Deduplicate)?;
        let error = start_scan(library, node_id, owner.clone(), false)
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::LibraryTaskAlreadyRunning {
                task: LibraryTaskKind::Deduplicate
            })
        ));
        assert!(!token.is_cancelled());
        assert_eq!(
            owner
                .lock()
                .unwrap()
                .current(&record_id, LibraryTaskKind::Deduplicate),
            running
        );
        Ok(())
    }

    #[tokio::test]
    async fn scan_lifecycle_handler_rejects_request_without_open_library() {
        let request = ScanAudioLibraryRequest {
            call_id: None,
            force: false,
        };
        let error = request
            .handle(
                (
                    None,
                    Arc::new(Uuid::new_v4().to_string()),
                    Arc::new(Capture::default()),
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
    async fn scan_lifecycle_scanning_an_open_replica_scans_its_folder() -> Result<()> {
        let (_config, node_id, library) = open_test_library().await?;
        let audio = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");
        std::fs::copy(audio, Path::new(&library.media_root).join("track.ogg"))?;
        let record_id = library.descriptor.record_id.0.to_string();
        let owner = Arc::new(StdMutex::new(LibraryTaskOwner::default()));
        let capture = Arc::new(Capture::default());
        let request = ScanAudioLibraryRequest {
            call_id: None,
            force: false,
        };
        request
            .handle(
                (
                    Some(library),
                    Arc::new(node_id),
                    capture.clone(),
                    Arc::new(RwLock::new(SyncSettings::default())),
                    owner.clone(),
                ),
                None,
                &request,
            )
            .await?;

        loop {
            let changed = capture.changed.notified();
            let states = capture.states();
            if states.iter().any(|state| {
                matches!(
                    state,
                    LibraryTaskState::ScanEnded {
                        record_id: ended_record,
                        outcome: ScanOutcome::Finished,
                        ..
                    } if ended_record == &record_id
                )
            }) {
                break;
            }
            timeout(TEST_DEADLINE, changed).await?;
        }
        assert!(matches!(
            owner
                .lock()
                .unwrap()
                .current(&record_id, LibraryTaskKind::Scan),
            LibraryTaskState::ScanEnded {
                outcome: ScanOutcome::Finished,
                progress: 1,
                ..
            }
        ));
        Ok(())
    }

    #[tokio::test]
    async fn a_scan_reports_its_phases_and_ends_in_the_phase_it_reached() -> Result<()> {
        let (_config, node_id, library) = open_test_library().await?;
        let audio = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");
        std::fs::copy(audio, Path::new(&library.media_root).join("track.ogg"))?;
        let capture = Arc::new(Capture::default());
        let request = ScanAudioLibraryRequest {
            call_id: None,
            force: false,
        };
        request
            .handle(
                (
                    Some(library),
                    Arc::new(node_id),
                    capture.clone(),
                    Arc::new(RwLock::new(SyncSettings::default())),
                    Arc::new(StdMutex::new(LibraryTaskOwner::default())),
                ),
                None,
                &request,
            )
            .await?;
        wait_for_task_outcome(&capture, LibraryTaskKind::Scan).await?;

        let states = capture.states();
        assert!(matches!(
            states.first(),
            Some(LibraryTaskState::ScanRunning {
                phase: ScanTaskPhase::IndexFiles,
                ..
            })
        ));
        assert!(states.iter().any(|state| matches!(
            state,
            LibraryTaskState::ScanRunning {
                phase: ScanTaskPhase::ScanCoverArts,
                ..
            }
        )));
        assert!(matches!(
            states.last(),
            Some(LibraryTaskState::ScanEnded {
                phase: ScanTaskPhase::ScanCoverArts,
                outcome: ScanOutcome::Finished,
                ..
            })
        ));
        assert!(states.iter().all(|state| !matches!(
            state,
            LibraryTaskState::AnalysisRunning { .. }
                | LibraryTaskState::AnalysisEnded { .. }
                | LibraryTaskState::DeduplicationRunning { .. }
                | LibraryTaskState::DeduplicationEnded { .. }
        )));
        Ok(())
    }

    #[tokio::test]
    async fn scan_lifecycle_cancel_response_waits_for_worker_and_contains_cancelled_state()
    -> Result<()> {
        let owner = Arc::new(StdMutex::new(LibraryTaskOwner::default()));
        let (running, token) = owner
            .lock()
            .unwrap()
            .start("library", LibraryTaskKind::Scan)?;
        let LibraryTaskState::ScanRunning { run_id, .. } = running else {
            anyhow::bail!("expected a running scan");
        };
        let (stopped_tx, stopped_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let worker = task::spawn(async move {
            token.cancelled().await;
            let _ = stopped_tx.send(());
            let _ = release_rx.await;
        });
        owner
            .lock()
            .unwrap()
            .attach_worker("library", LibraryTaskKind::Scan, &run_id, worker);
        let request = CancelTaskRequest {
            call_id: None,
            record_id: "library".into(),
            r#type: CancelTaskType::ScanAudioLibrary,
        };
        let (response_tx, mut response_rx) = unbounded_channel();
        let cancel_owner = owner.clone();
        let cancel = task::spawn(async move {
            let response = request
                .handle((cancel_owner, Arc::new(Capture::default())), None, &request)
                .await
                .unwrap()
                .unwrap();
            let _ = response_tx.send(response);
        });
        timeout(TEST_DEADLINE, stopped_rx).await??;
        assert!(response_rx.try_recv().is_err());
        let _ = release_tx.send(());
        let response = timeout(TEST_DEADLINE, response_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            response.state,
            LibraryTaskState::ScanEnded {
                outcome: ScanOutcome::Cancelled,
                ..
            }
        ));
        cancel.await?;
        Ok(())
    }
}
