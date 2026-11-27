use std::sync::{Mutex, PoisonError};

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
    done: CancellationToken,
    committed: bool,
}

pub(crate) struct HandoffRun<'a> {
    owner: &'a HandoffTickets,
    ticket: Uuid,
    cancel: CancellationToken,
}

impl HandoffTickets {
    pub(crate) fn prepare(&self) -> Uuid {
        let ticket = Uuid::new_v4();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.next = Some(ticket);
        ticket
    }

    pub(crate) fn start(&self, ticket: Uuid) -> Result<HandoffRun<'_>, ExpectedRequestError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.next != Some(ticket) || state.running.is_some() {
            return Err(ExpectedRequestError::HandoffSuperseded);
        }
        let cancel = CancellationToken::new();
        state.next = None;
        state.running = Some(RunningHandoff {
            ticket,
            cancel: cancel.clone(),
            done: CancellationToken::new(),
            committed: false,
        });
        Ok(HandoffRun {
            owner: self,
            ticket,
            cancel,
        })
    }

    /// A cancellation of a running handoff settles only after its cleanup has finished.
    pub(crate) async fn cancel(&self, ticket: Uuid) {
        let done = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.next == Some(ticket) {
                state.next = None;
                return;
            }
            let Some(running) = state.running.as_ref().filter(|run| run.ticket == ticket) else {
                return;
            };
            if !running.committed {
                running.cancel.cancel();
            }
            running.done.clone()
        };
        done.cancelled().await;
    }
}

impl HandoffRun<'_> {
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
            running.done.cancel();
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
        owner.cancel(first).await;
        owner.cancel(first).await;
        assert!(owner.start(first).is_err());

        let next = owner.prepare();
        let run = owner.start(next).unwrap();
        owner.cancel(first).await;
        assert!(run.commit().is_ok());
    }

    #[tokio::test]
    async fn closing_during_playback_confirmation_prevents_commit() {
        let owner = HandoffTickets::default();
        let ticket = owner.prepare();
        let run = owner.start(ticket).unwrap();
        let cancel = owner.cancel(ticket);
        tokio::pin!(cancel);
        assert!(futures::poll!(&mut cancel).is_pending());
        let outcome = run.confirm(std::future::pending()).await;
        assert!(matches!(
            outcome.unwrap_err().downcast_ref(),
            Some(ExpectedRequestError::HandoffSuperseded)
        ));
        drop(run);
        cancel.await;
    }

    #[tokio::test]
    async fn late_cancellation_waits_for_a_committed_handoff_without_undoing_it() {
        let owner = HandoffTickets::default();
        let ticket = owner.prepare();
        let run = owner.start(ticket).unwrap();
        run.commit().unwrap();
        let cancel = owner.cancel(ticket);
        tokio::pin!(cancel);
        assert!(futures::poll!(&mut cancel).is_pending());
        assert!(run.check_cancelled().is_ok());
        drop(run);
        cancel.await;
    }
}
