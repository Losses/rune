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
    messages::{LibraryTaskKind, LibraryTaskOutcome, LibraryTaskState, LibraryTaskStateChanged},
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
        scan_token: Option<&CancellationToken>,
    ) -> Result<(LibraryTaskState, CancellationToken), ExpectedRequestError> {
        self.ensure_can_start(record_id, task, scan_token)?;
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
        let state = LibraryTaskState::Running {
            record_id: record_id.to_owned(),
            task,
            run_id,
            progress: 0,
            total: 0,
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

    pub fn ensure_can_start(
        &self,
        record_id: &str,
        requested: LibraryTaskKind,
        scan_token: Option<&CancellationToken>,
    ) -> Result<(), ExpectedRequestError> {
        if requested != LibraryTaskKind::Scan
            && scan_token.is_some_and(|token| !token.is_cancelled())
        {
            return Err(ExpectedRequestError::LibraryTaskAlreadyRunning {
                task: LibraryTaskKind::Scan,
            });
        }
        for task in [LibraryTaskKind::Analyze, LibraryTaskKind::Deduplicate] {
            if let Some(entry) = self.tasks.get(&(record_id.to_owned(), task))
                && (matches!(entry.state, LibraryTaskState::Running { .. })
                    || !*entry.worker.done.borrow())
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
            && state_run_id(&entry.state) == Some(run_id)
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
        if state_run_id(&entry.state) != Some(run_id)
            || !matches!(entry.state, LibraryTaskState::Running { .. })
        {
            return None;
        }
        entry.state = LibraryTaskState::Running {
            record_id: record_id.to_owned(),
            task,
            run_id: run_id.to_owned(),
            progress,
            total,
        };
        Some(entry.state.clone())
    }

    pub fn end(
        &mut self,
        record_id: &str,
        task: LibraryTaskKind,
        run_id: &str,
        outcome: LibraryTaskOutcome,
        progress: i32,
        total: i32,
    ) -> Option<LibraryTaskState> {
        let entry = self.tasks.get_mut(&(record_id.to_owned(), task))?;
        if state_run_id(&entry.state) != Some(run_id)
            || !matches!(entry.state, LibraryTaskState::Running { .. })
        {
            return None;
        }
        entry.state = LibraryTaskState::Ended {
            record_id: record_id.to_owned(),
            task,
            run_id: run_id.to_owned(),
            progress,
            total,
            outcome,
        };
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
            if let LibraryTaskState::Running {
                record_id,
                task,
                run_id,
                progress,
                total,
            } = entry.state.clone()
            {
                entry.worker.token.cancel();
                let state = LibraryTaskState::Ended {
                    record_id,
                    task,
                    run_id,
                    progress,
                    total,
                    outcome: LibraryTaskOutcome::Cancelled,
                };
                entry.state = state.clone();
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
                if matches!(entry.state, LibraryTaskState::Running { .. }) {
                    entry.worker.token.cancel();
                    if let LibraryTaskState::Running {
                        record_id,
                        task,
                        run_id,
                        progress,
                        total,
                    } = entry.state.clone()
                    {
                        entry.state = LibraryTaskState::Ended {
                            record_id,
                            task,
                            run_id,
                            progress,
                            total,
                            outcome: LibraryTaskOutcome::Cancelled,
                        };
                        states.push(entry.state.clone());
                    }
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

fn state_run_id(state: &LibraryTaskState) -> Option<&str> {
    match state {
        LibraryTaskState::NotRunning { .. } => None,
        LibraryTaskState::Running { run_id, .. } | LibraryTaskState::Ended { run_id, .. } => {
            Some(run_id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::RinfRustSignal;
    use std::sync::Arc;
    use tokio::sync::{mpsc::unbounded_channel, oneshot};

    fn running(state: LibraryTaskState) -> (String, String) {
        match state {
            LibraryTaskState::Running {
                record_id, run_id, ..
            } => (record_id, run_id),
            state => panic!("expected running state, got {state:?}"),
        }
    }

    #[tokio::test]
    async fn a_second_analysis_start_is_refused_and_keeps_the_running_state() {
        let owner = Mutex::new(LibraryTaskOwner::default());
        let (first, _first_token) = owner
            .lock()
            .unwrap()
            .start("library", LibraryTaskKind::Analyze, None)
            .unwrap();
        let error = owner
            .lock()
            .unwrap()
            .start("library", LibraryTaskKind::Analyze, None)
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
    async fn analysis_start_is_refused_while_a_scan_is_running() {
        let owner = LibraryTaskOwner::default();
        let scan_token = CancellationToken::new();
        let error = owner
            .ensure_can_start("library", LibraryTaskKind::Analyze, Some(&scan_token))
            .unwrap_err();
        assert!(matches!(
            error,
            ExpectedRequestError::LibraryTaskAlreadyRunning {
                task: LibraryTaskKind::Scan
            }
        ));
    }

    #[tokio::test]
    async fn scan_start_is_refused_while_deduplication_is_running() {
        let mut owner = LibraryTaskOwner::default();
        owner
            .start("library", LibraryTaskKind::Deduplicate, None)
            .unwrap();
        let error = owner
            .ensure_can_start("library", LibraryTaskKind::Scan, None)
            .unwrap_err();
        assert!(matches!(
            error,
            ExpectedRequestError::LibraryTaskAlreadyRunning {
                task: LibraryTaskKind::Deduplicate
            }
        ));
    }

    #[tokio::test]
    async fn deduplicate_start_is_refused_while_analysis_is_running() {
        let mut owner = LibraryTaskOwner::default();
        owner
            .start("library", LibraryTaskKind::Analyze, None)
            .unwrap();
        let error = owner
            .ensure_can_start("library", LibraryTaskKind::Deduplicate, None)
            .unwrap_err();
        assert!(matches!(
            error,
            ExpectedRequestError::LibraryTaskAlreadyRunning {
                task: LibraryTaskKind::Analyze
            }
        ));
    }

    #[tokio::test]
    async fn cancel_returns_only_after_the_worker_stops() {
        let owner = Arc::new(Mutex::new(LibraryTaskOwner::default()));
        let (state, token) = owner
            .lock()
            .unwrap()
            .start("library", LibraryTaskKind::Analyze, None)
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
            .attach_worker(&record_id, LibraryTaskKind::Analyze, &run_id, worker);
        let (response_tx, mut response_rx) = unbounded_channel();
        let cancel_owner = owner.clone();
        let cancel = tokio::spawn(async move {
            let state =
                LibraryTaskOwner::cancel(&cancel_owner, "library", LibraryTaskKind::Analyze).await;
            let _ = response_tx.send(state.unwrap());
        });
        stopped_rx.await.unwrap();
        assert!(response_rx.try_recv().is_err());
        let _ = release_tx.send(());
        assert!(matches!(
            response_rx.recv().await.unwrap(),
            LibraryTaskState::Ended {
                outcome: LibraryTaskOutcome::Cancelled,
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
            .start("library", LibraryTaskKind::Analyze, None)
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
                LibraryTaskKind::Analyze,
                &run_id,
                LibraryTaskOutcome::Finished,
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
            .start("library", LibraryTaskKind::Analyze, None)
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
            LibraryTaskKind::Analyze,
            &first_run,
            LibraryTaskOutcome::Cancelled,
            1,
            4,
        );
        LibraryTaskOwner::cancel(&owner, "library", LibraryTaskKind::Analyze)
            .await
            .unwrap();
        let (second, _) = owner
            .lock()
            .unwrap()
            .start("library", LibraryTaskKind::Analyze, None)
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
            .start("library", LibraryTaskKind::Deduplicate, None)
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
        stopped_rx.await.unwrap();
        assert!(done_rx.try_recv().is_err());
        let _ = release_tx.send(());
        done_rx.recv().await.unwrap();
        shutdown.await.unwrap();
        assert!(matches!(
            owner
                .lock()
                .unwrap()
                .current("library", LibraryTaskKind::Deduplicate),
            LibraryTaskState::Ended {
                outcome: LibraryTaskOutcome::Cancelled,
                ..
            }
        ));
    }

    struct NoopBroadcaster;
    impl Broadcaster for NoopBroadcaster {
        fn broadcast(&self, _message: &dyn RinfRustSignal) {}
    }
}
