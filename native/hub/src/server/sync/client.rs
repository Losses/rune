//! Resumable bidirectional metadata sessions for an authenticated paired peer.
use anyhow::{Result, ensure};
pub use database::actions::cover_art::fetch_missing;
use database::sync::{LibrarySyncStore, model::RuneModel};
use http_request::sync::PageCursor;
use http_request::sync::{BidirectionalSession, MetadataPeer};
use sea_orm::DatabaseConnection;
use std::sync::Arc;
use std::time::Duration;
use sync::{SyncModel, SyncPeer};
use tokio::time::Instant;
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

/// Keep this value between background passes and network retries.
pub struct LibrarySession {
    local: LibrarySyncStore,
    session: BidirectionalSession<RuneModel>,
}
impl LibrarySession {
    pub fn new(local: LibrarySyncStore) -> Result<Self> {
        let scope = sync::Scope {
            collections: RuneModel::collections().into_iter().collect(),
        };
        Ok(Self {
            local,
            session: BidirectionalSession::new(scope, session_limits())?,
        })
    }
    /// One bounded pass; a failed request leaves its continuation available for retry.
    pub async fn advance(
        &mut self,
        remote: &MetadataPeer<RuneModel>,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<bool> {
        let endpoint = self.local.endpoint().with_notification(notify);
        Ok(self.session.advance(&endpoint, remote).await?)
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
