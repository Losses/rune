//! Resumable bidirectional metadata sessions for an authenticated paired peer.
use anyhow::{Result, ensure};
use database::sync::{LibrarySyncStore, model::RuneModel};
use http_request::sync::{BidirectionalSession, MetadataPeer};
use sea_orm::DatabaseConnection;
use std::sync::Arc;
use sync::{SyncModel, SyncPeer};
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
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut cursor = http_request::sync::PageCursor::default();
    let (mut attached, mut pending) = (0, 0);
    while !cursor.is_complete() && tokio::time::Instant::now() < deadline {
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

/// Fetch missing verified artwork bodies with per-item failure isolation.
pub async fn fetch_missing(
    db: &DatabaseConnection,
    store: &database::sync::SeaOrmSyncStore,
    server: &str,
    client: &reqwest::Client,
    cursor: &mut http_request::sync::PageCursor,
    deadline: tokio::time::Instant,
) -> Result<(usize, usize)> {
    use database::actions::cover_art::{
        artwork_body_needed, artwork_candidates, attach_received_body,
    };
    use database::sync::model::Payload;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let attached = AtomicUsize::new(0);
    let pending = AtomicUsize::new(0);
    http_request::sync::visit_page(
        cursor,
        |cursor| async move {
            let page = artwork_candidates(db, store, cursor).await?;
            let records = page
                .records
                .into_iter()
                .filter_map(|record| match record.value {
                    Some(Payload::Artwork(artwork)) => Some((record.key.id, artwork)),
                    _ => None,
                })
                .collect();
            Ok((records, page.next, page.done))
        },
        |id, artwork| {
            let attached = &attached;
            let pending = &pending;
            async move {
                if tokio::time::Instant::now() >= deadline {
                    pending.fetch_add(1, Ordering::Relaxed);
                    return Ok(std::ops::ControlFlow::Break(()));
                }
                if !artwork_body_needed(db, id, &artwork).await? {
                    return Ok(std::ops::ControlFlow::Continue(()));
                }
                let limit =
                    deadline.min(tokio::time::Instant::now() + std::time::Duration::from_secs(1));
                let result = tokio::time::timeout_at(limit, async {
                    let body = artwork.body()?;
                    let url = format!("{}/sync/artworks/{id}", server.trim_end_matches('/'));
                    let bytes =
                        http_request::sync::fetch_bounded(client, &url, body.size, limit).await?;
                    attach_received_body(db, store, id, &artwork, bytes).await?;
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("artwork request timed out")));
                match result {
                    Ok(()) => {
                        attached.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) => {
                        pending.fetch_add(1, Ordering::Relaxed);
                        log::warn!("artwork {id} remains pending: {error:#}");
                    }
                }
                Ok(std::ops::ControlFlow::Continue(()))
            }
        },
    )
    .await?;
    Ok((
        attached.load(Ordering::Relaxed),
        pending.load(Ordering::Relaxed),
    ))
}
