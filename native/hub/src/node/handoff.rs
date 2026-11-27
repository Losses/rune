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
    next: Option<(Uuid, Arc<CancellationToken>)>,
    running: Option<RunningHandoff>,
}

struct RunningHandoff {
    ui: Arc<CancellationToken>,
    ticket: Uuid,
    cancel: CancellationToken,
    completion: Arc<HandoffCompletion>,
    committed: bool,
}

impl RunningHandoff {
    fn cancel(&self) -> Arc<HandoffCompletion> {
        if !self.committed {
            self.cancel.cancel();
        }
        self.completion.clone()
    }
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
    pub(crate) fn prepare(
        &self,
        ui: &Arc<CancellationToken>,
    ) -> Result<Uuid, ExpectedRequestError> {
        let ticket = Uuid::new_v4();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if ui.is_cancelled() {
            return Err(ExpectedRequestError::HandoffSuperseded);
        }
        state.next = Some((ticket, ui.clone()));
        Ok(ticket)
    }

    pub(crate) async fn start(
        &self,
        ticket: Option<Uuid>,
        ui: &Arc<CancellationToken>,
    ) -> Result<HandoffRun<'_>, ExpectedRequestError> {
        let run_ticket = ticket.unwrap_or_else(Uuid::new_v4);
        loop {
            let previous = {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                if ui.is_cancelled()
                    || ticket.is_some()
                        && !state.next.as_ref().is_some_and(|(next, owner)| {
                            *next == run_ticket && Arc::ptr_eq(owner, ui)
                        })
                {
                    return Err(ExpectedRequestError::HandoffSuperseded);
                }
                if let Some(running) = &state.running {
                    running.completion.clone()
                } else {
                    let cancel = CancellationToken::new();
                    let completion = Arc::new(HandoffCompletion::default());
                    if ticket.is_some() {
                        state.next = None;
                    }
                    state.running = Some(RunningHandoff {
                        ui: ui.clone(),
                        ticket: run_ticket,
                        cancel: cancel.clone(),
                        completion: completion.clone(),
                        committed: false,
                    });
                    return Ok(HandoffRun {
                        owner: self,
                        ticket: run_ticket,
                        cancel,
                        completion,
                    });
                }
            };
            // A replacement must let the previous operation finish restoring playback.
            if let Err(error) = previous.wait().await {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                if state
                    .next
                    .as_ref()
                    .is_some_and(|(next, _)| *next == run_ticket)
                {
                    state.next = None;
                }
                return Err(error);
            }
        }
    }

    pub(crate) async fn close_ui(
        &self,
        ui: &Arc<CancellationToken>,
    ) -> Result<(), ExpectedRequestError> {
        let completion = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            // Revoke admission under the same lock that installs a running handoff.
            ui.cancel();
            if state
                .next
                .as_ref()
                .is_some_and(|(_, owner)| Arc::ptr_eq(owner, ui))
            {
                state.next = None;
            }
            state
                .running
                .as_ref()
                .filter(|run| Arc::ptr_eq(&run.ui, ui))
                .map(RunningHandoff::cancel)
        };
        if let Some(completion) = completion {
            completion.wait().await?;
        }
        Ok(())
    }

    /// A cancellation of a running handoff settles only after its cleanup has finished.
    pub(crate) async fn cancel(&self, ticket: Uuid) -> Result<(), ExpectedRequestError> {
        let completion = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.next.as_ref().is_some_and(|(next, _)| *next == ticket) {
                state.next = None;
                return Ok(());
            }
            let Some(running) = state.running.as_ref().filter(|run| run.ticket == ticket) else {
                return Ok(());
            };
            running.cancel()
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
    async fn closing_the_old_ui_waits_for_its_handoff_and_keeps_the_new_ticket() {
        let owner = HandoffTickets::default();
        let old_ui = Arc::new(CancellationToken::new());
        let new_ui = Arc::new(CancellationToken::new());
        let run = owner.start(None, &old_ui).await.unwrap();
        let next = owner.prepare(&new_ui).unwrap();

        let close = owner.close_ui(&old_ui);
        tokio::pin!(close);
        assert!(futures::poll!(&mut close).is_pending());
        assert!(run.check_cancelled().is_err());
        assert!(owner.prepare(&old_ui).is_err());
        assert!(owner.start(None, &old_ui).await.is_err());

        run.finish();
        drop(run);
        close.await.unwrap();
        assert!(
            owner
                .start(Some(next), &new_ui)
                .await
                .unwrap()
                .commit()
                .is_ok()
        );
    }

    #[tokio::test]
    async fn cancelling_a_prepared_ticket_does_not_cancel_a_later_handoff() {
        let owner = HandoffTickets::default();
        let ui = Arc::new(CancellationToken::new());
        let first = owner.prepare(&ui).unwrap();
        owner.cancel(first).await.unwrap();
        owner.cancel(first).await.unwrap();
        assert!(owner.start(Some(first), &ui).await.is_err());

        let next = owner.prepare(&ui).unwrap();
        let run = owner.start(Some(next), &ui).await.unwrap();
        owner.cancel(first).await.unwrap();
        assert!(run.commit().is_ok());
    }

    #[tokio::test]
    async fn retry_waits_for_the_cancelled_handoff_to_finish() {
        let owner = HandoffTickets::default();
        let ui = Arc::new(CancellationToken::new());
        let ticket = owner.prepare(&ui).unwrap();
        let run = owner.start(Some(ticket), &ui).await.unwrap();
        let cancel = owner.cancel(ticket);
        tokio::pin!(cancel);
        assert!(futures::poll!(&mut cancel).is_pending());

        let next = owner.prepare(&ui).unwrap();
        let retry = owner.start(Some(next), &ui);
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
        let ui = Arc::new(CancellationToken::new());
        let ticket = owner.prepare(&ui).unwrap();
        let run = owner.start(Some(ticket), &ui).await.unwrap();
        let cancel = owner.cancel(ticket);
        tokio::pin!(cancel);
        assert!(futures::poll!(&mut cancel).is_pending());

        let next = owner.prepare(&ui).unwrap();
        let retry = owner.start(Some(next), &ui);
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
        assert!(owner.start(Some(next), &ui).await.is_err());
    }

    #[tokio::test]
    async fn an_interrupted_handoff_does_not_report_settled() {
        let owner = HandoffTickets::default();
        let ui = Arc::new(CancellationToken::new());
        let ticket = owner.prepare(&ui).unwrap();
        let run = owner.start(Some(ticket), &ui).await.unwrap();
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
        let ui = Arc::new(CancellationToken::new());
        let ticket = owner.prepare(&ui).unwrap();
        let run = owner.start(Some(ticket), &ui).await.unwrap();
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
        let ui = Arc::new(CancellationToken::new());
        let ticket = owner.prepare(&ui).unwrap();
        let run = owner.start(Some(ticket), &ui).await.unwrap();
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
