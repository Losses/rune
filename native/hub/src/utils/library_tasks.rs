use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
};

use tokio::{
    sync::{oneshot, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    messages::{
        LibraryTaskEnd, LibraryTaskKind, LibraryTaskState, LibraryTaskStateChanged, ScanTaskPhase,
    },
    request_error::ExpectedRequestError,
    utils::Broadcaster,
};

#[derive(Default)]
pub struct LibraryTaskOwner {
    tasks: HashMap<(String, LibraryTaskKind), TaskEntry>,
}

struct TaskEntry {
    state: LibraryTaskState,
    worker: RunningWorker,
}

struct RunningWorker {
    token: CancellationToken,
    attach: Option<oneshot::Sender<JoinHandle<()>>>,
    done: watch::Receiver<bool>,
}

impl LibraryTaskOwner {
    pub fn start(
        &mut self,
        record_id: &str,
        task: LibraryTaskKind,
    ) -> Result<(LibraryTaskState, CancellationToken), ExpectedRequestError> {
        self.ensure_can_start(record_id)?;
        let key = (record_id.to_owned(), task);
        let run_id = Uuid::new_v4().to_string();
        let token = CancellationToken::new();
        let (attach, worker_rx) = oneshot::channel();
        let (done_tx, done) = watch::channel(false);
        tokio::spawn(async move {
            if let Ok(worker) = worker_rx.await {
                let _ = worker.await;
            }
            let _ = done_tx.send(true);
        });
        let record_id = record_id.to_owned();
        let state = match task {
            LibraryTaskKind::Scan => LibraryTaskState::ScanRunning {
                record_id,
                run_id,
                progress: 0,
                total: 0,
                phase: ScanTaskPhase::IndexFiles,
            },
            LibraryTaskKind::Analyze => LibraryTaskState::AnalysisRunning {
                record_id,
                run_id,
                progress: 0,
                total: 0,
            },
            LibraryTaskKind::Deduplicate => LibraryTaskState::DeduplicationRunning {
                record_id,
                run_id,
                progress: 0,
                total: 0,
            },
        };
        self.tasks.insert(
            key,
            TaskEntry {
                state: state.clone(),
                worker: RunningWorker {
                    token: token.clone(),
                    attach: Some(attach),
                    done,
                },
            },
        );
        Ok((state, token))
    }

    pub fn ensure_can_start(&self, record_id: &str) -> Result<(), ExpectedRequestError> {
        for task in [
            LibraryTaskKind::Scan,
            LibraryTaskKind::Analyze,
            LibraryTaskKind::Deduplicate,
        ] {
            if let Some(entry) = self.tasks.get(&(record_id.to_owned(), task))
                && (entry.state.is_running() || !*entry.worker.done.borrow())
            {
                return Err(ExpectedRequestError::LibraryTaskAlreadyRunning { task });
            }
        }
        Ok(())
    }

    pub fn attach_worker(
        &mut self,
        record_id: &str,
        task: LibraryTaskKind,
        run_id: &str,
        worker: JoinHandle<()>,
    ) {
        if let Some(entry) = self.tasks.get_mut(&(record_id.to_owned(), task))
            && entry.state.run_id() == Some(run_id)
            && let Some(attach) = entry.worker.attach.take()
        {
            let _ = attach.send(worker);
        }
    }

    pub fn progress(
        &mut self,
        record_id: &str,
        task: LibraryTaskKind,
        run_id: &str,
        progress: i32,
        total: i32,
    ) -> Option<LibraryTaskState> {
        let entry = self.tasks.get_mut(&(record_id.to_owned(), task))?;
        if entry.state.run_id() != Some(run_id) {
            return None;
        }
        match &mut entry.state {
            LibraryTaskState::ScanRunning {
                progress: now,
                total: all,
                ..
            }
            | LibraryTaskState::AnalysisRunning {
                progress: now,
                total: all,
                ..
            }
            | LibraryTaskState::DeduplicationRunning {
                progress: now,
                total: all,
                ..
            } => {
                *now = progress;
                *all = total;
            }
            _ => return None,
        }
        Some(entry.state.clone())
    }

    pub fn set_scan_phase(
        &mut self,
        record_id: &str,
        run_id: &str,
        scan_phase: ScanTaskPhase,
    ) -> Option<LibraryTaskState> {
        let entry = self
            .tasks
            .get_mut(&(record_id.to_owned(), LibraryTaskKind::Scan))?;
        if entry.state.run_id() != Some(run_id) {
            return None;
        }
        let LibraryTaskState::ScanRunning { phase, .. } = &mut entry.state else {
            return None;
        };
        *phase = scan_phase;
        Some(entry.state.clone())
    }

    pub fn end(
        &mut self,
        record_id: &str,
        run_id: &str,
        end: LibraryTaskEnd,
        progress: i32,
        total: i32,
    ) -> Option<LibraryTaskState> {
        let entry = self.tasks.get_mut(&(record_id.to_owned(), end.kind()))?;
        if entry.state.run_id() != Some(run_id) || !entry.state.is_running() {
            return None;
        }
        entry.state = entry.state.clone().ended(end, Some((progress, total)));
        Some(entry.state.clone())
    }

    pub async fn cancel(
        owner: &Mutex<Self>,
        record_id: &str,
        task: LibraryTaskKind,
    ) -> Option<LibraryTaskState> {
        let (state, mut worker_done) = {
            let mut owner = owner.lock().unwrap_or_else(PoisonError::into_inner);
            let entry = owner.tasks.get_mut(&(record_id.to_owned(), task))?;
            if entry.state.is_running() {
                entry.worker.token.cancel();
                entry.state = entry.state.clone().cancelled();
            }
            (entry.state.clone(), Some(entry.worker.done.clone()))
        };
        if let Some(worker_done) = &mut worker_done {
            wait_for_worker(worker_done).await;
        }
        Some(state)
    }

    pub fn current(&self, record_id: &str, task: LibraryTaskKind) -> LibraryTaskState {
        self.tasks
            .get(&(record_id.to_owned(), task))
            .map(|entry| entry.state.clone())
            .unwrap_or_else(|| LibraryTaskState::NotRunning {
                record_id: record_id.to_owned(),
                task,
            })
    }

    pub async fn shutdown(owner: &Mutex<Self>, broadcaster: &dyn Broadcaster) {
        let (mut workers, states) = {
            let mut owner = owner.lock().unwrap_or_else(PoisonError::into_inner);
            let mut states = Vec::new();
            let mut workers = Vec::new();
            for entry in owner.tasks.values_mut() {
                if entry.state.is_running() {
                    entry.worker.token.cancel();
                    entry.state = entry.state.clone().cancelled();
                    states.push(entry.state.clone());
                }
                if !*entry.worker.done.borrow() {
                    workers.push(entry.worker.done.clone());
                }
            }
            (workers, states)
        };
        for state in states {
            broadcaster.broadcast(&LibraryTaskStateChanged { state });
        }
        for worker in &mut workers {
            wait_for_worker(worker).await;
        }
    }
}

async fn wait_for_worker(worker_done: &mut watch::Receiver<bool>) {
    while !*worker_done.borrow() {
        if worker_done.changed().await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{AnalysisOutcome, DeduplicationOutcome, ScanFailure, ScanOutcome};
    use crate::test_wait::TEST_DEADLINE;
    use crate::utils::RinfRustSignal;
    use std::sync::Arc;
    use tokio::{
        sync::{mpsc::unbounded_channel, oneshot},
        time::timeout,
    };

    fn running(state: LibraryTaskState) -> (String, String) {
        match state {
            LibraryTaskState::ScanRunning {
                record_id, run_id, ..
            }
            | LibraryTaskState::AnalysisRunning {
                record_id, run_id, ..
            }
            | LibraryTaskState::DeduplicationRunning {
                record_id, run_id, ..
            } => (record_id, run_id),
            state => panic!("expected running state, got {state:?}"),
        }
    }

    #[tokio::test]
    async fn each_task_kind_starts_in_its_own_variant() {
        let mut owner = LibraryTaskOwner::default();
        let (scan, _) = owner.start("scan-library", LibraryTaskKind::Scan).unwrap();
        let (analysis, _) = owner
            .start("analysis-library", LibraryTaskKind::Analyze)
            .unwrap();
        let (deduplication, _) = owner
            .start("dedup-library", LibraryTaskKind::Deduplicate)
            .unwrap();
        assert!(matches!(
            scan,
            LibraryTaskState::ScanRunning {
                phase: ScanTaskPhase::IndexFiles,
                ..
            }
        ));
        assert!(matches!(analysis, LibraryTaskState::AnalysisRunning { .. }));
        assert!(matches!(
            deduplication,
            LibraryTaskState::DeduplicationRunning { .. }
        ));
    }

    #[tokio::test]
    async fn a_scan_phase_is_set_only_on_a_running_scan() {
        let mut owner = LibraryTaskOwner::default();
        let (analysis, _) = owner.start("library", LibraryTaskKind::Analyze).unwrap();
        let (_, run_id) = running(analysis);
        assert!(
            owner
                .set_scan_phase("library", &run_id, ScanTaskPhase::ScanCoverArts)
                .is_none()
        );
    }

    #[tokio::test]
    async fn cancelling_a_task_ends_it_in_its_own_variant() {
        let owner = Mutex::new(LibraryTaskOwner::default());
        let (state, token) = owner
            .lock()
            .unwrap()
            .start("library", LibraryTaskKind::Analyze)
            .unwrap();
        let (_, run_id) = running(state);
        let worker = tokio::spawn(async move { token.cancelled().await });
        owner
            .lock()
            .unwrap()
            .attach_worker("library", LibraryTaskKind::Analyze, &run_id, worker);
        let cancelled = LibraryTaskOwner::cancel(&owner, "library", LibraryTaskKind::Analyze)
            .await
            .unwrap();
        assert!(matches!(
            cancelled,
            LibraryTaskState::AnalysisEnded {
                outcome: AnalysisOutcome::Cancelled,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn a_second_analysis_start_is_refused_and_keeps_the_running_state() {
        let owner = Mutex::new(LibraryTaskOwner::default());
        let (first, _first_token) = owner
            .lock()
            .unwrap()
            .start("library", LibraryTaskKind::Analyze)
            .unwrap();
        let error = owner
            .lock()
            .unwrap()
            .start("library", LibraryTaskKind::Analyze)
            .unwrap_err();
        assert!(matches!(
            error,
            ExpectedRequestError::LibraryTaskAlreadyRunning {
                task: LibraryTaskKind::Analyze
            }
        ));
        assert_eq!(
            owner
                .lock()
                .unwrap()
                .current("library", LibraryTaskKind::Analyze),
            first
        );
    }

    #[tokio::test]
    async fn scan_lifecycle_analysis_start_is_refused_while_a_scan_is_running() {
        let mut owner = LibraryTaskOwner::default();
        owner.start("library", LibraryTaskKind::Scan).unwrap();
        let error = owner.ensure_can_start("library").unwrap_err();
        assert!(matches!(
            error,
            ExpectedRequestError::LibraryTaskAlreadyRunning {
                task: LibraryTaskKind::Scan
            }
        ));
    }

    #[tokio::test]
    async fn scan_lifecycle_scan_start_is_refused_while_deduplication_is_running() {
        let mut owner = LibraryTaskOwner::default();
        owner
            .start("library", LibraryTaskKind::Deduplicate)
            .unwrap();
        let error = owner.ensure_can_start("library").unwrap_err();
        assert!(matches!(
            error,
            ExpectedRequestError::LibraryTaskAlreadyRunning {
                task: LibraryTaskKind::Deduplicate
            }
        ));
    }

    #[tokio::test]
    async fn scan_lifecycle_second_scan_start_is_refused_and_keeps_the_running_state() {
        let mut owner = LibraryTaskOwner::default();
        let (first, _first_token) = owner.start("library", LibraryTaskKind::Scan).unwrap();
        let error = owner.start("library", LibraryTaskKind::Scan).unwrap_err();
        assert!(matches!(
            error,
            ExpectedRequestError::LibraryTaskAlreadyRunning {
                task: LibraryTaskKind::Scan
            }
        ));
        assert_eq!(owner.current("library", LibraryTaskKind::Scan), first);
    }

    #[tokio::test]
    async fn scan_lifecycle_worker_failure_ends_with_a_typed_outcome() {
        let mut owner = LibraryTaskOwner::default();
        let (state, _) = owner.start("library", LibraryTaskKind::Scan).unwrap();
        let (_, run_id) = running(state);
        let ended = owner
            .end(
                "library",
                &run_id,
                LibraryTaskEnd::Scan(ScanOutcome::Failed(ScanFailure::ScanFailed)),
                2,
                4,
            )
            .unwrap();
        assert!(matches!(
            ended,
            LibraryTaskState::ScanEnded {
                outcome: ScanOutcome::Failed(ScanFailure::ScanFailed),
                progress: 2,
                total: 4,
                ..
            }
        ));
        assert!(
            owner
                .progress("library", LibraryTaskKind::Scan, &run_id, 4, 4)
                .is_none()
        );
    }

    #[tokio::test]
    async fn scan_lifecycle_reports_the_current_scan_phase() {
        let mut owner = LibraryTaskOwner::default();
        let (state, _) = owner.start("library", LibraryTaskKind::Scan).unwrap();
        let (_, run_id) = running(state);
        let state = owner
            .set_scan_phase("library", &run_id, ScanTaskPhase::ScanCoverArts)
            .unwrap();
        assert!(matches!(
            state,
            LibraryTaskState::ScanRunning {
                phase: ScanTaskPhase::ScanCoverArts,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn deduplicate_start_is_refused_while_analysis_is_running() {
        let mut owner = LibraryTaskOwner::default();
        owner.start("library", LibraryTaskKind::Analyze).unwrap();
        let error = owner.ensure_can_start("library").unwrap_err();
        assert!(matches!(
            error,
            ExpectedRequestError::LibraryTaskAlreadyRunning {
                task: LibraryTaskKind::Analyze
            }
        ));
    }

    #[tokio::test]
    async fn scan_lifecycle_cancelling_a_scan_answers_after_its_worker_stops() {
        let owner = Arc::new(Mutex::new(LibraryTaskOwner::default()));
        let (state, token) = owner
            .lock()
            .unwrap()
            .start("library", LibraryTaskKind::Scan)
            .unwrap();
        let (record_id, run_id) = running(state);
        let (stopped_tx, stopped_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let worker = tokio::spawn(async move {
            token.cancelled().await;
            let _ = stopped_tx.send(());
            let _ = release_rx.await;
        });
        owner
            .lock()
            .unwrap()
            .attach_worker(&record_id, LibraryTaskKind::Scan, &run_id, worker);
        let (response_tx, mut response_rx) = unbounded_channel();
        let cancel_owner = owner.clone();
        let cancel = tokio::spawn(async move {
            let state =
                LibraryTaskOwner::cancel(&cancel_owner, "library", LibraryTaskKind::Scan).await;
            let _ = response_tx.send(state.unwrap());
        });
        timeout(TEST_DEADLINE, stopped_rx).await.unwrap().unwrap();
        assert!(response_rx.try_recv().is_err());
        let _ = release_tx.send(());
        assert!(matches!(
            response_rx.recv().await.unwrap(),
            LibraryTaskState::ScanEnded {
                outcome: ScanOutcome::Cancelled,
                ..
            }
        ));
        cancel.await.unwrap();
    }

    #[tokio::test]
    async fn cancel_after_finish_returns_the_finished_state() {
        let owner = Mutex::new(LibraryTaskOwner::default());
        let (state, _) = owner
            .lock()
            .unwrap()
            .start("library", LibraryTaskKind::Analyze)
            .unwrap();
        let (_, run_id) = running(state);
        let worker = tokio::spawn(async {});
        owner
            .lock()
            .unwrap()
            .attach_worker("library", LibraryTaskKind::Analyze, &run_id, worker);
        let finished = owner
            .lock()
            .unwrap()
            .end(
                "library",
                &run_id,
                LibraryTaskEnd::Analysis(AnalysisOutcome::Finished),
                4,
                4,
            )
            .unwrap();
        let response = LibraryTaskOwner::cancel(&owner, "library", LibraryTaskKind::Analyze)
            .await
            .unwrap();
        assert_eq!(response, finished);
    }

    #[tokio::test]
    async fn cancelling_then_restarting_ignores_the_old_runs_events() {
        let owner = Mutex::new(LibraryTaskOwner::default());
        let (first, _) = owner
            .lock()
            .unwrap()
            .start("library", LibraryTaskKind::Analyze)
            .unwrap();
        let (_, first_run) = running(first);
        let worker = tokio::spawn(async {});
        owner.lock().unwrap().attach_worker(
            "library",
            LibraryTaskKind::Analyze,
            &first_run,
            worker,
        );
        owner.lock().unwrap().end(
            "library",
            &first_run,
            LibraryTaskEnd::Analysis(AnalysisOutcome::Cancelled),
            1,
            4,
        );
        LibraryTaskOwner::cancel(&owner, "library", LibraryTaskKind::Analyze)
            .await
            .unwrap();
        let (second, _) = owner
            .lock()
            .unwrap()
            .start("library", LibraryTaskKind::Analyze)
            .unwrap();
        let (_, second_run) = running(second.clone());
        assert_ne!(first_run, second_run);
        assert!(
            owner
                .lock()
                .unwrap()
                .progress("library", LibraryTaskKind::Analyze, &first_run, 4, 4)
                .is_none()
        );
        assert_eq!(
            owner
                .lock()
                .unwrap()
                .current("library", LibraryTaskKind::Analyze),
            second
        );
    }

    #[tokio::test]
    async fn shutdown_waits_for_running_workers() {
        let owner = Arc::new(Mutex::new(LibraryTaskOwner::default()));
        let (state, token) = owner
            .lock()
            .unwrap()
            .start("library", LibraryTaskKind::Deduplicate)
            .unwrap();
        let (record_id, run_id) = running(state);
        let (stopped_tx, stopped_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let worker = tokio::spawn(async move {
            token.cancelled().await;
            let _ = stopped_tx.send(());
            let _ = release_rx.await;
        });
        owner.lock().unwrap().attach_worker(
            &record_id,
            LibraryTaskKind::Deduplicate,
            &run_id,
            worker,
        );
        let (done_tx, mut done_rx) = unbounded_channel();
        let shutdown_owner = owner.clone();
        let shutdown = tokio::spawn(async move {
            LibraryTaskOwner::shutdown(&shutdown_owner, &NoopBroadcaster).await;
            let _ = done_tx.send(());
        });
        timeout(TEST_DEADLINE, stopped_rx).await.unwrap().unwrap();
        assert!(done_rx.try_recv().is_err());
        let _ = release_tx.send(());
        timeout(TEST_DEADLINE, done_rx.recv())
            .await
            .unwrap()
            .unwrap();
        shutdown.await.unwrap();
        assert!(matches!(
            owner
                .lock()
                .unwrap()
                .current("library", LibraryTaskKind::Deduplicate),
            LibraryTaskState::DeduplicationEnded {
                outcome: DeduplicationOutcome::Cancelled,
                ..
            }
        ));
    }

    struct NoopBroadcaster;
    impl Broadcaster for NoopBroadcaster {
        fn broadcast(&self, _message: &dyn RinfRustSignal) {}
    }
}
