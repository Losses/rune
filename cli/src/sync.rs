//! Resumable bidirectional metadata sessions for an authenticated paired peer.
use anyhow::{Result, ensure};
pub use database::actions::cover_art::fetch_missing;
use database::sync::{LibrarySyncStore, model::RuneModel};
use http_request::sync::PageCursor;
use http_request::sync::{BidirectionalSession, MetadataPeer};
use sea_orm::DatabaseConnection;
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

pub async fn sync_library(
    db: DatabaseConnection,
    node_id: Uuid,
    server: &str,
    client: reqwest::Client,
) -> Result<LibrarySyncReport> {
    let remote = MetadataPeer::<RuneModel>::new(server, client.clone());
    let identity = remote.identity().await?;
    let local = LibrarySyncStore::open(db.clone(), node_id, Some(identity.namespace)).await?;
    local.import_existing(node_id).await?;
    ensure!(
        local.identity().await?.replica != identity.replica,
        "cannot sync with self"
    );
    let scope = sync::Scope {
        collections: RuneModel::collections().into_iter().collect(),
    };
    let mut session = BidirectionalSession::new(scope, session_limits())?;
    let endpoint = local.endpoint();
    while !session.advance(&endpoint, &remote).await? {
        tokio::task::yield_now().await;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut cursor = PageCursor::default();
    let mut artwork_pending = 0;
    while !cursor.is_complete() && Instant::now() < deadline {
        let (_, pending) = fetch_missing(
            &db,
            &local.record_store(),
            server,
            &client,
            &mut cursor,
            deadline,
        )
        .await?;
        artwork_pending += pending;
    }
    let (pulled, pushed) = session
        .progress()
        .ok_or_else(|| anyhow::anyhow!("metadata session incomplete"))?;
    Ok(LibrarySyncReport {
        pulled,
        pushed,
        artwork_pending,
        artwork_complete: cursor.is_complete(),
    })
}
