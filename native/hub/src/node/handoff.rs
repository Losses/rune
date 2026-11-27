use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::request_error::ExpectedRequestError;

/// The UI reserves a ticket before sending a handoff. Closing before dispatch revokes the
/// reservation without building up a list of cancellation requests for unknown operations.
#[derive(Default)]
pub(crate) struct HandoffTickets {
    state: Mutex<Tickets>,
}

#[derive(Default)]
struct Tickets {
    next: Option<Uuid>,
    running: Option<RunningHandoff>,
}

struct RunningHandoff {
    ticket: Uuid,
    cancel: CancellationToken,
    completion: Arc<HandoffCompletion>,
    committed: bool,
}

#[derive(Default)]
struct HandoffCompletion {
    done: CancellationToken,
    finished: AtomicBool,
    unsettled: AtomicBool,
}

impl HandoffCompletion {
    async fn wait(&self) -> Result<(), ExpectedRequestError> {
        self.done.cancelled().await;
        if self.unsettled.load(Ordering::Acquire) {
            return Err(ExpectedRequestError::Unavailable {
                what: "the target renderer's unresolved handoff rollback",
            });
        }
        if !self.finished.load(Ordering::Acquire) {
            return Err(ExpectedRequestError::Unavailable {
                what: "completion of the interrupted handoff",
            });
        }
        Ok(())
    }
}

pub(crate) struct HandoffRun<'a> {
    owner: &'a HandoffTickets,
    ticket: Uuid,
    cancel: CancellationToken,
    completion: Arc<HandoffCompletion>,
}

impl HandoffTickets {
    pub(crate) fn prepare(&self) -> Uuid {
        let ticket = Uuid::new_v4();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.next = Some(ticket);
        ticket
    }

    pub(crate) async fn start(&self, ticket: Uuid) -> Result<HandoffRun<'_>, ExpectedRequestError> {
        loop {
            let previous = {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                if state.next != Some(ticket) {
                    return Err(ExpectedRequestError::HandoffSuperseded);
                }
                if let Some(running) = &state.running {
                    running.completion.clone()
                } else {
                    let cancel = CancellationToken::new();
                    let completion = Arc::new(HandoffCompletion::default());
                    state.next = None;
                    state.running = Some(RunningHandoff {
                        ticket,
                        cancel: cancel.clone(),
                        completion: completion.clone(),
                        committed: false,
                    });
                    return Ok(HandoffRun {
                        owner: self,
                        ticket,
                        cancel,
                        completion,
                    });
                }
            };
            // Close is immediate. A replacement must wait for the old operation to finish
            // restoring playback, then check that its own ticket is still reserved.
            if let Err(error) = previous.wait().await {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                if state.next == Some(ticket) {
                    state.next = None;
                }
                return Err(error);
            }
        }
    }

    /// A cancellation of a running handoff settles only after its cleanup has finished.
    pub(crate) async fn cancel(&self, ticket: Uuid) -> Result<(), ExpectedRequestError> {
        let completion = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.next == Some(ticket) {
                state.next = None;
                return Ok(());
            }
            let Some(running) = state.running.as_ref().filter(|run| run.ticket == ticket) else {
                return Ok(());
            };
            if !running.committed {
                running.cancel.cancel();
            }
            running.completion.clone()
        };
        completion.wait().await
    }
}

impl HandoffRun<'_> {
    /// Record that the worker reached a terminal result rather than being dropped mid-operation.
    pub(crate) fn finish(&self) {
        self.completion.finished.store(true, Ordering::Release);
    }

    pub(crate) fn mark_rollback_unsettled(&self) {
        self.completion.unsettled.store(true, Ordering::Release);
    }

    pub(crate) fn check_cancelled(&self) -> Result<(), ExpectedRequestError> {
        if self.cancel.is_cancelled() {
            return Err(ExpectedRequestError::HandoffSuperseded);
        }
        Ok(())
    }

    pub(crate) async fn confirm(
        &self,
        confirmation: impl Future<Output = anyhow::Result<()>>,
    ) -> anyhow::Result<()> {
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => return Err(ExpectedRequestError::HandoffSuperseded.into()),
            outcome = confirmation => outcome?,
        }
        self.commit()?;
        Ok(())
    }

    /// Once committed, a late dismissal waits for completion without undoing this operation.
    pub(crate) fn commit(&self) -> Result<(), ExpectedRequestError> {
        let mut state = self
            .owner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let running = state
            .running
            .as_mut()
            .filter(|run| run.ticket == self.ticket)
            .ok_or(ExpectedRequestError::HandoffSuperseded)?;
        self.check_cancelled()?;
        running.committed = true;
        Ok(())
    }
}

impl Drop for HandoffRun<'_> {
    fn drop(&mut self) {
        let mut state = self
            .owner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state
            .running
            .as_ref()
            .is_some_and(|run| run.ticket == self.ticket)
            && let Some(running) = state.running.take()
        {
            running.completion.done.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelling_a_prepared_ticket_does_not_cancel_a_later_handoff() {
        let owner = HandoffTickets::default();
        let first = owner.prepare();
        owner.cancel(first).await.unwrap();
        owner.cancel(first).await.unwrap();
        assert!(owner.start(first).await.is_err());

        let next = owner.prepare();
        let run = owner.start(next).await.unwrap();
        owner.cancel(first).await.unwrap();
        assert!(run.commit().is_ok());
    }

    #[tokio::test]
    async fn retry_waits_for_the_cancelled_handoff_to_finish() {
        let owner = HandoffTickets::default();
        let ticket = owner.prepare();
        let run = owner.start(ticket).await.unwrap();
        let cancel = owner.cancel(ticket);
        tokio::pin!(cancel);
        assert!(futures::poll!(&mut cancel).is_pending());

        let next = owner.prepare();
        let retry = owner.start(next);
        tokio::pin!(retry);
        assert!(futures::poll!(&mut retry).is_pending());
        run.finish();
        drop(run);
        cancel.await.unwrap();
        assert!(retry.await.unwrap().commit().is_ok());
    }

    #[tokio::test]
    async fn an_unsettled_rollback_fails_cancellation_and_a_waiting_retry() {
        let owner = HandoffTickets::default();
        let ticket = owner.prepare();
        let run = owner.start(ticket).await.unwrap();
        let cancel = owner.cancel(ticket);
        tokio::pin!(cancel);
        assert!(futures::poll!(&mut cancel).is_pending());

        let next = owner.prepare();
        let retry = owner.start(next);
        tokio::pin!(retry);
        assert!(futures::poll!(&mut retry).is_pending());
        run.mark_rollback_unsettled();
        run.finish();
        drop(run);

        assert!(matches!(
            cancel.await,
            Err(ExpectedRequestError::Unavailable { .. })
        ));
        assert!(matches!(
            retry.await,
            Err(ExpectedRequestError::Unavailable { .. })
        ));
        assert!(owner.start(next).await.is_err());
    }

    #[tokio::test]
    async fn an_interrupted_handoff_does_not_report_settled() {
        let owner = HandoffTickets::default();
        let ticket = owner.prepare();
        let run = owner.start(ticket).await.unwrap();
        let cancel = owner.cancel(ticket);
        tokio::pin!(cancel);
        assert!(futures::poll!(&mut cancel).is_pending());
        drop(run);

        assert!(matches!(
            cancel.await,
            Err(ExpectedRequestError::Unavailable { .. })
        ));
    }

    #[tokio::test]
    async fn closing_during_playback_confirmation_prevents_commit() {
        let owner = HandoffTickets::default();
        let ticket = owner.prepare();
        let run = owner.start(ticket).await.unwrap();
        let cancel = owner.cancel(ticket);
        tokio::pin!(cancel);
        assert!(futures::poll!(&mut cancel).is_pending());
        let outcome = run.confirm(std::future::pending()).await;
        assert!(matches!(
            outcome.unwrap_err().downcast_ref(),
            Some(ExpectedRequestError::HandoffSuperseded)
        ));
        run.finish();
        drop(run);
        cancel.await.unwrap();
    }

    #[tokio::test]
    async fn late_cancellation_waits_for_a_committed_handoff_without_undoing_it() {
        let owner = HandoffTickets::default();
        let ticket = owner.prepare();
        let run = owner.start(ticket).await.unwrap();
        run.commit().unwrap();
        let cancel = owner.cancel(ticket);
        tokio::pin!(cancel);
        assert!(futures::poll!(&mut cancel).is_pending());
        assert!(run.check_cancelled().is_ok());
        run.finish();
        drop(run);
        cancel.await.unwrap();
    }
}
