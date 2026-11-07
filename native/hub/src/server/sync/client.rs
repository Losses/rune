//! Resumable bidirectional metadata sessions for an authenticated paired peer.
use anyhow::{Result, ensure};
pub use database::actions::cover_art::fetch_missing;
use database::sync::{LibrarySyncStore, model::RuneModel};
use http_request::sync::PageCursor;
use http_request::sync::{BidirectionalSession, MetadataPeer};
use sea_orm::DatabaseConnection;
use std::sync::Arc;
use std::time::Duration;
use std::{future::Future, pin::Pin};
use sync::{SyncModel, SyncPeer};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug)]
pub struct LibrarySyncReport {
    pub pulled: sync::ReceiveProgress,
    pub pushed: sync::ReceiveProgress,
    /// Artwork bodies observed but not attached during this content pass.
    pub artwork_pending: usize,
    /// Whether all artwork records were visited during this run.
    pub artwork_complete: bool,
}
use database::sync::session_limits;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SessionDirection {
    #[default]
    Both,
    Pull,
    Push,
}

/// Keep this value between background passes and network retries.
pub struct LibrarySession {
    local: LibrarySyncStore,
    session: BidirectionalSession<RuneModel>,
    direction: SessionDirection,
}
impl LibrarySession {
    pub fn new(local: LibrarySyncStore) -> Result<Self> {
        let scope = sync::Scope {
            collections: RuneModel::collections().into_iter().collect(),
        };
        Ok(Self {
            local,
            direction: SessionDirection::Both,
            session: BidirectionalSession::new(scope, session_limits())?,
        })
    }
    pub(crate) fn set_direction(&mut self, direction: SessionDirection) {
        self.direction = direction;
    }
    /// One bounded pass; a failed request leaves its continuation available for retry.
    pub async fn advance(
        &mut self,
        remote: &MetadataPeer<RuneModel>,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<bool> {
        self.advance_cancellable(remote, notify, &CancellationToken::new())
            .await
    }
    pub async fn advance_cancellable(
        &mut self,
        remote: &MetadataPeer<RuneModel>,
        notify: Arc<dyn Fn() + Send + Sync>,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        let endpoint = self.local.endpoint().with_notification(notify);
        let mut step = (&mut self.session, &endpoint, remote, self.direction);
        retry_storage_busy(
            cancel,
            &mut step,
            |(session, endpoint, remote, direction)| {
                Box::pin(async move {
                    match direction {
                        SessionDirection::Both => session.advance(*endpoint, *remote).await,
                        SessionDirection::Pull => session.pull_only(*endpoint, *remote).await,
                        SessionDirection::Push => session.push_only(*endpoint, *remote).await,
                    }
                    .map_err(anyhow::Error::new)
                })
            },
        )
        .await
    }
    pub fn report(&self) -> Result<LibrarySyncReport> {
        let (pulled, pushed) = self
            .session
            .progress()
            .ok_or_else(|| anyhow::anyhow!("metadata session incomplete"))?;
        Ok(LibrarySyncReport {
            pulled,
            pushed,
            artwork_pending: 0,
            artwork_complete: false,
        })
    }
}

async fn retry_storage_busy<T, State, F>(
    cancel: &CancellationToken,
    state: &mut State,
    mut step: F,
) -> Result<T>
where
    F: for<'a> FnMut(&'a mut State) -> Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>,
{
    let mut delay = Duration::from_millis(100);
    for attempt in 0..5 {
        match step(state).await {
            Ok(value) => return Ok(value),
            Err(error)
                if error
                    .downcast_ref::<sync::SyncError>()
                    .is_some_and(|error| *error == sync::SyncError::StorageBusy)
                    && attempt < 4 =>
            {
                tokio::select! {
                    _ = cancel.cancelled() => return Err(sync::SyncError::Cancelled.into()),
                    _ = tokio::time::sleep(delay) => {}
                }
                delay = (delay * 2).min(Duration::from_millis(800));
            }
            Err(error) => return Err(error),
        }
    }
    Err(sync::SyncError::StorageBusy.into())
}

pub async fn sync_library(
    db: DatabaseConnection,
    node_id: Uuid,
    server: &str,
    client: reqwest::Client,
) -> Result<LibrarySyncReport> {
    sync_library_with_notify(db, node_id, server, client, Arc::new(|| {})).await
}
pub async fn sync_library_with_notify(
    db: DatabaseConnection,
    node_id: Uuid,
    server: &str,
    client: reqwest::Client,
    notify: Arc<dyn Fn() + Send + Sync>,
) -> Result<LibrarySyncReport> {
    let remote = MetadataPeer::<RuneModel>::new(server, client.clone());
    let identity = remote.identity().await?;
    let local = LibrarySyncStore::open(db.clone(), node_id, Some(identity.namespace)).await?;
    local.import_existing(node_id).await?;
    ensure!(
        local.identity().await?.replica != identity.replica,
        "cannot sync with self"
    );
    let mut session = LibrarySession::new(local.clone())?;
    while !session.advance(&remote, notify.clone()).await? {
        tokio::task::yield_now().await;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut cursor = PageCursor::default();
    let (mut attached, mut pending) = (0, 0);
    while !cursor.is_complete() && Instant::now() < deadline {
        let (new_attached, new_pending) = fetch_missing(
            &db,
            &local.record_store(),
            server,
            &client,
            &mut cursor,
            deadline,
        )
        .await?;
        attached += new_attached;
        pending += new_pending;
    }
    if attached > 0 {
        notify();
    }
    let mut report = session.report()?;
    report.artwork_pending = pending;
    report.artwork_complete = cursor.is_complete();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::retry_storage_busy;
    use sync::SyncError;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn busy_step_retries_and_recovers() {
        let cancel = CancellationToken::new();
        let mut calls = AtomicUsize::new(0);
        let result = retry_storage_busy(&cancel, &mut calls, |calls| {
            Box::pin(async move {
                if calls.fetch_add(1, Ordering::SeqCst) < 2 {
                    Err(anyhow::Error::new(SyncError::StorageBusy))
                } else {
                    Ok("complete")
                }
            })
        })
        .await
        .unwrap();

        assert_eq!(result, "complete");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn busy_step_stops_at_five_attempts_with_typed_error() {
        let cancel = CancellationToken::new();
        let mut calls = AtomicUsize::new(0);
        let error = retry_storage_busy(&cancel, &mut calls, |calls| {
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(anyhow::Error::new(SyncError::StorageBusy))
            })
        })
        .await
        .unwrap_err();

        assert_eq!(calls.load(Ordering::SeqCst), 5);
        assert_eq!(
            error.downcast_ref::<SyncError>(),
            Some(&SyncError::StorageBusy)
        );
    }
}
