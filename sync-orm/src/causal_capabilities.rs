//! Capability composition over one SQL replica and caller-owned projections.
use crate::{
    causal_candidates,
    causal_leases::lock_on,
    causal_manifests, causal_progress, causal_snapshot,
    causal_source::{open_batch_step_locked_on, split_locked_on, summarize_locked_on},
    causal_store::CausalReplica,
};
use anyhow::{Result, ensure};
use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, ExprTrait, TransactionTrait,
    sea_query::{Alias, Expr, Func, Query},
};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use sync::causal::Context;
use sync::causal::Dot;
use sync::protocol::{self as p, BoxFuture};
use tokio::sync::{Mutex, MutexGuard};
use uuid::Uuid;

/// The application projects accepted changes using this exact transaction.
/// It must not re-enter the endpoint or author from received changes.
#[async_trait]
pub trait TransactionProjection<M: p::SyncModel>: Send + Sync {
    /// Conservative source-row mutation count, admitted with causal persistence.
    fn mutation_ceiling(&self, changes: &p::ProjectionBatch<M>) -> Result<u64>;
    async fn apply(&self, tx: &DatabaseTransaction, changes: &p::ProjectionBatch<M>) -> Result<()>;
}
struct NoProjection;
#[async_trait]
impl<M: p::SyncModel> TransactionProjection<M> for NoProjection {
    fn mutation_ceiling(&self, _: &p::ProjectionBatch<M>) -> Result<u64> {
        Ok(0)
    }
    async fn apply(&self, _: &DatabaseTransaction, _: &p::ProjectionBatch<M>) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct CausalEndpoint<M: p::SyncModel> {
    inner: Arc<EndpointState<M>>,
    notify: Arc<dyn Fn() + Send + Sync>,
    lock_holder_observer: Option<Arc<dyn Fn(&'static str, Duration) + Send + Sync>>,
}
struct EndpointState<M: p::SyncModel> {
    store: CausalReplica<M>,
    projection: Arc<dyn TransactionProjection<M>>,
    receiver: Mutex<p::ReceiverLedger<M>>,
}
fn error(error: anyhow::Error) -> p::SyncError {
    if crate::is_storage_busy(&error) {
        return p::SyncError::StorageBusy;
    }
    error
        .downcast_ref::<p::SyncError>()
        .cloned()
        .unwrap_or_else(|| p::SyncError::Storage(format!("{error:#}")))
}

#[cfg(test)]
mod tests {
    use sea_orm::{
        ConnectionTrait, SqlxSqliteConnector,
        sqlx::{SqlitePool, sqlite::SqliteConnectOptions},
    };
    use std::time::Duration;

    #[tokio::test]
    async fn sqlite_busy_db_error_converts_to_typed_sync_error() {
        let path = std::env::temp_dir().join(format!("sync-busy-{}.db", uuid::Uuid::new_v4()));
        let lock_pool = SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .busy_timeout(Duration::from_secs(1)),
        )
        .await
        .unwrap();
        let test_pool = SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .busy_timeout(Duration::from_millis(1)),
        )
        .await
        .unwrap();
        let lock = SqlxSqliteConnector::from_sqlx_sqlite_pool(lock_pool);
        let under_test = SqlxSqliteConnector::from_sqlx_sqlite_pool(test_pool);
        lock.execute_unprepared("CREATE TABLE writes (value INTEGER)")
            .await
            .unwrap();
        lock.execute_unprepared("BEGIN IMMEDIATE").await.unwrap();
        let db_error = under_test
            .execute_unprepared("INSERT INTO writes VALUES (1)")
            .await
            .unwrap_err();
        assert_eq!(
            super::error(anyhow::Error::new(db_error)),
            sync::SyncError::StorageBusy
        );
        drop(lock);
        drop(under_test);
        let _ = std::fs::remove_file(path);
    }
}
fn now() -> u64 {
    std::cmp::max(chrono::Utc::now().timestamp_millis(), 0) as u64
}
fn source_work(mut limits: p::WorkLimits) -> p::Result<p::WorkLimits> {
    if limits.max_requests == 0
        || limits.max_requests > p::MAX_ROUND_REQUESTS
        || limits.max_rows_scanned == 0
        || limits.max_rows_scanned > p::MAX_ROWS
        || limits.max_bytes == 0
        || limits.max_bytes > p::MAX_ROUND_BYTES
        || limits.max_mutations == 0
        || limits.max_mutations > p::MAX_MUTATIONS
        || limits.max_depth == 0
        || limits.max_depth > p::MAX_DEPTH
    {
        return Err(p::SyncError::Invalid("invalid source work limits".into()));
    }
    // A pass budget is an upper bound. Each durable continuation can consume
    // a smaller unit without rejecting a caller's larger valid allowance.
    limits.max_rows_scanned = std::cmp::min(limits.max_rows_scanned, 1024);
    limits.max_bytes = std::cmp::min(limits.max_bytes, p::DEFAULT_LOGICAL_BYTES);
    Ok(limits)
}

/// One capability request may commit several short source transactions. Each
/// transaction reserves its entire admitted allowance before doing any work.
struct SourceRequestBudget {
    rows: u64,
    bytes: u64,
    mutations: u64,
    transactions: u32,
}
impl SourceRequestBudget {
    fn new(limits: p::WorkLimits) -> Self {
        Self {
            rows: limits.max_rows_scanned,
            bytes: limits.max_bytes,
            mutations: limits.max_mutations,
            transactions: std::cmp::min(limits.max_requests, 64),
        }
    }
    fn has_work(&self) -> bool {
        self.rows > 0 && self.bytes > 0 && self.mutations > 0 && self.transactions > 0
    }
    fn reserve(&mut self, limits: p::WorkLimits) -> bool {
        if self.transactions == 0
            || limits.max_rows_scanned > self.rows
            || limits.max_bytes > self.bytes
            || limits.max_mutations > self.mutations
        {
            return false;
        }
        self.rows -= limits.max_rows_scanned;
        self.bytes -= limits.max_bytes;
        self.mutations -= limits.max_mutations;
        self.transactions -= 1;
        true
    }
    fn validate_charge(reserved: p::WorkLimits, charged: p::WorkLimits) -> Result<()> {
        ensure!(
            charged.max_rows_scanned <= reserved.max_rows_scanned
                && charged.max_bytes <= reserved.max_bytes
                && charged.max_mutations <= reserved.max_mutations,
            "snapshot step exceeded its reservation"
        );
        Ok(())
    }
    fn release_unused(&mut self, reserved: p::WorkLimits, charged: p::WorkLimits) {
        self.rows += reserved.max_rows_scanned - charged.max_rows_scanned;
        self.bytes += reserved.max_bytes - charged.max_bytes;
        self.mutations += reserved.max_mutations - charged.max_mutations;
    }
}

impl<M: p::SyncModel> p::SnapshotSource<M> for CausalEndpoint<M> {
    fn release_request(&self, request_id: Uuid) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let result: Result<()> = async {
                let tx = self.inner.store.connection().begin().await?;
                crate::causal_leases::lock_on(&tx, self.inner.store.namespace()).await?;
                let ns = self.inner.store.namespace();
                crate::causal_source::release_request_on(&tx, ns, request_id).await?;
                tx.commit().await?;
                Ok(())
            }
            .await;
            result.map_err(error)
        })
    }
    fn close_snapshot(&self, snapshot: Uuid) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let result: Result<()> = async {
                let tx = self.inner.store.connection().begin().await?;
                crate::causal_leases::lock_on(&tx, self.inner.store.namespace()).await?;
                let ns = self.inner.store.namespace();
                crate::causal_source::release_request_on(&tx, ns, snapshot).await?;
                tx.commit().await?;
                Ok(())
            }
            .await;
            result.map_err(error)
        })
    }
    fn cancel_snapshot_build(&self, cursor: p::WorkCursor) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let result: Result<()> = async {
                let tx = self.inner.store.connection().begin().await?;
                crate::causal_leases::lock_on(&tx, self.inner.store.namespace()).await?;
                let ns = self.inner.store.namespace();
                crate::causal_source::cancel_snapshot_build_on::<M>(&tx, ns, cursor).await?;
                tx.commit().await?;
                Ok(())
            }
            .await;
            result.map_err(error)
        })
    }
    fn open_snapshot(
        &self,
        request_id: Uuid,
        scope: p::Scope<M>,
        limits: p::WorkLimits,
        resume: Option<p::WorkCursor>,
    ) -> BoxFuture<'_, p::WorkOutcome<p::SnapshotDescriptor<M>>> {
        Box::pin(async move {
            let limits = source_work(limits)?;
            let result: Result<p::WorkOutcome<p::SnapshotDescriptor<M>>> = async {
                let mut budget = SourceRequestBudget::new(limits);
                let started = Instant::now();
                let mut resume = resume;
                let mut committed: Option<p::WorkCursor> = None;
                let mut plan: Option<crate::causal_source::SnapshotStepPlan> = None;
                loop {
                    // The deadline is cooperative: an admitted transaction
                    // completes atomically before another step is considered.
                    if let Some(cursor) = &committed
                        && (!budget.has_work() || started.elapsed() >= Duration::from_millis(200))
                    {
                        return Ok(p::WorkOutcome::Incomplete {
                            cursor: cursor.clone(),
                        });
                    }
                    let reserved = plan.as_ref().map_or(limits, |step| step.admitted);
                    if !budget.reserve(reserved) {
                        return committed
                            .map(|cursor| p::WorkOutcome::Incomplete { cursor })
                            .ok_or_else(|| p::SyncError::WorkLimit.into());
                    }
                    let tx = self.inner.store.connection().begin().await?;
                    crate::causal_leases::lock_on(&tx, self.inner.store.namespace()).await?;
                    let transaction_started = Instant::now();
                    let step = open_batch_step_locked_on(
                        &tx,
                        self.inner.store.identity_on(&tx).await?,
                        request_id,
                        scope.clone(),
                        crate::causal_source::SnapshotTiming {
                            now_ms: now() as i64,
                            ttl_ms: p::DEFAULT_SNAPSHOT_TTL_MS as i64,
                        },
                        crate::causal_source::SnapshotBatchLimits { full: limits, plan },
                        resume,
                    )
                    .await?;
                    SourceRequestBudget::validate_charge(reserved, step.charged)?;
                    tx.commit().await?;
                    if let Some(observer) = &self.lock_holder_observer {
                        observer("open_batch_step_on", transaction_started.elapsed());
                    }
                    budget.release_unused(reserved, step.charged);
                    plan = step.next;
                    match step.outcome {
                        p::WorkOutcome::Ready(descriptor) => {
                            return Ok(p::WorkOutcome::Ready(descriptor));
                        }
                        p::WorkOutcome::Incomplete { cursor } => {
                            resume = Some(cursor.clone());
                            committed = Some(cursor);
                        }
                    }
                    tokio::task::yield_now().await;
                }
            }
            .await;
            result.map_err(error)
        })
    }
    fn summarize(
        &self,
        snapshot: Uuid,
        range: p::KeyRange<M>,
        limits: p::WorkLimits,
        resume: Option<p::WorkCursor>,
    ) -> BoxFuture<'_, p::WorkOutcome<p::RangeSummary<M>>> {
        Box::pin(async move {
            let limits = source_work(limits)?;
            let result: Result<p::WorkOutcome<p::RangeSummary<M>>> = async {
                let tx = self.inner.store.connection().begin().await?;
                lock_on(&tx, self.inner.store.namespace()).await?;
                let transaction_started = Instant::now();
                let ns = self.inner.store.namespace();
                let value =
                    summarize_locked_on(&tx, ns, snapshot, range, now() as i64, limits, resume)
                        .await?;
                tx.commit().await?;
                if let Some(observer) = &self.lock_holder_observer {
                    observer("summarize_on", transaction_started.elapsed());
                }
                Ok(value)
            }
            .await;
            result.map_err(error)
        })
    }
    fn split(
        &self,
        snapshot: Uuid,
        range: p::KeyRange<M>,
        limits: p::WorkLimits,
        resume: Option<p::WorkCursor>,
    ) -> BoxFuture<'_, p::WorkOutcome<Vec<p::KeyRange<M>>>> {
        Box::pin(async move {
            let limits = source_work(limits)?;
            let result: Result<p::WorkOutcome<Vec<p::KeyRange<M>>>> = async {
                let tx = self.inner.store.connection().begin().await?;
                lock_on(&tx, self.inner.store.namespace()).await?;
                let transaction_started = Instant::now();
                let ns = self.inner.store.namespace();
                let value =
                    split_locked_on(&tx, ns, snapshot, range, now() as i64, limits, resume).await?;
                tx.commit().await?;
                if let Some(observer) = &self.lock_holder_observer {
                    observer("split_on", transaction_started.elapsed());
                }
                Ok(value)
            }
            .await;
            result.map_err(error)
        })
    }
    fn leaf(
        &self,
        snapshot: Uuid,
        range: p::KeyRange<M>,
        after: Option<p::CandidateCursor<M>>,
        limits: p::PageLimits,
    ) -> BoxFuture<'_, p::LeafPage<M>> {
        Box::pin(async move {
            let result: Result<p::LeafPage<M>> = async {
                let tx = self.inner.store.connection().begin().await?;
                let ns = self.inner.store.namespace();
                let value =
                    causal_snapshot::leaf_on(&tx, ns, snapshot, now() as i64, range, after, limits)
                        .await?;
                tx.commit().await?;
                Ok(value)
            }
            .await;
            result.map_err(error)
        })
    }
    fn read_fragment(
        &self,
        snapshot: Uuid,
        manifest: p::ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    ) -> BoxFuture<'_, p::Fragment> {
        Box::pin(async move {
            let result: Result<p::Fragment> = async {
                let tx = self.inner.store.connection().begin().await?;
                let ns = self.inner.store.namespace();
                let value = causal_snapshot::fragment_on::<M>(
                    &tx,
                    ns,
                    snapshot,
                    now() as i64,
                    manifest,
                    ordinal,
                    max_bytes,
                )
                .await?;
                tx.commit().await?;
                Ok(value)
            }
            .await;
            result.map_err(error)
        })
    }
}
fn empty_projection<M: p::SyncModel>() -> p::ProjectionBatch<M> {
    p::ProjectionBatch {
        changes: vec![],
        causal_changes: 0,
        pending: 0,
    }
}
impl<M: p::SyncModel> CausalEndpoint<M> {
    pub fn new(store: CausalReplica<M>) -> Self {
        Self::with_projection(store, Arc::new(NoProjection))
    }
    pub fn with_projection(
        store: CausalReplica<M>,
        projection: Arc<dyn TransactionProjection<M>>,
    ) -> Self {
        Self {
            inner: Arc::new(EndpointState {
                store,
                projection,
                receiver: Mutex::new(p::ReceiverLedger::new()),
            }),
            notify: Arc::new(|| {}),
            lock_holder_observer: None,
        }
    }
    /// Shares the replica, writer and receive ledger; only its post-commit
    /// notification target changes. No notification runs inside a transaction.
    pub fn with_notification(mut self, notify: Arc<dyn Fn() + Send + Sync>) -> Self {
        self.notify = notify;
        self
    }
    /// Observe source transactions from their first lock attempt through commit.
    pub fn with_lock_holder_observer(
        mut self,
        observer: Arc<dyn Fn(&'static str, Duration) + Send + Sync>,
    ) -> Self {
        self.lock_holder_observer = Some(observer);
        self
    }
    pub fn store(&self) -> CausalReplica<M> {
        self.inner.store.clone()
    }
    async fn ledger(&self) -> MutexGuard<'_, p::ReceiverLedger<M>> {
        let mut ledger = self.inner.receiver.lock().await;
        ledger.expire(now());
        ledger
    }
    async fn project_on(
        &self,
        tx: &DatabaseTransaction,
        changes: &p::ProjectionBatch<M>,
    ) -> Result<()> {
        self.inner.projection.apply(tx, changes).await
    }
    async fn project_with_budget_on(
        &self,
        tx: &DatabaseTransaction,
        changes: &p::ProjectionBatch<M>,
        start: u64,
        limit: u64,
    ) -> Result<()> {
        let used = mutations_on(tx)
            .await?
            .checked_sub(start)
            .ok_or(p::SyncError::WorkLimit)?;
        let reserved = self.inner.projection.mutation_ceiling(changes)?;
        ensure!(
            used.checked_add(reserved)
                .is_some_and(|total| total <= limit),
            p::SyncError::WorkLimit
        );
        self.project_on(tx, changes).await?;
        Ok(())
    }
    async fn identity_impl(&self) -> Result<p::StoreIdentity> {
        self.inner
            .store
            .identity_on(self.inner.store.connection())
            .await
    }
    async fn history(
        &self,
        key: &p::CheckpointKey,
        progress: Option<p::ReceiveProgress>,
    ) -> Result<p::HistoryStatus> {
        let tx = self.inner.store.connection().begin().await?;
        let identity = self.inner.store.identity_on(&tx).await?;
        let status = causal_progress::history_status_on(&tx, &identity, key, progress).await?;
        tx.commit().await?;
        Ok(status)
    }
    async fn fragment(
        &self,
        manifest: p::ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    ) -> Result<p::Fragment> {
        let tx = self.inner.store.connection().begin().await?;
        let result = read_fragment_on(
            &tx,
            self.inner.store.namespace(),
            manifest,
            ordinal,
            max_bytes,
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }
    async fn candidate_page(
        &self,
        key: &p::ObjectKey<M>,
        after: Option<p::Dot>,
        limits: p::PageLimits,
    ) -> Result<p::CandidatePage<M>> {
        ensure!(
            limits.max_scanned > 0
                && limits.max_scanned <= p::MAX_PAGE_ITEMS
                && limits.max_bytes > 0
                && limits.max_bytes <= p::MAX_PAGE_BYTES,
            p::SyncError::Invalid("invalid candidate page limits".into())
        );
        ensure!(
            !key.id.is_nil() && M::collections().contains(&key.collection),
            p::SyncError::Invalid("invalid key".into())
        );
        let tx = self.inner.store.connection().begin().await?;
        let q = Query::select()
            .column(Alias::new("state_seq"))
            .from(Alias::new("sync_meta"))
            .and_where(
                Expr::col(Alias::new("namespace")).eq(self
                    .inner
                    .store
                    .namespace()
                    .as_bytes()
                    .to_vec()),
            )
            .to_owned();
        let state: i64 = tx
            .query_one(&q)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing namespace"))?
            .try_get_by_index(0)?;
        let rows = causal_candidates::page_on(
            &tx,
            crate::causal_candidates::CandidatePageRequest {
                namespace: self.inner.store.namespace(),
                collection: M::collection_tag(&key.collection).as_bytes(),
                state_seq: state,
                lower: Some(key.id),
                upper: key.id.as_u128().checked_add(1).map(Uuid::from_u128),
                after: after.map(|dot| causal_candidates::Cursor {
                    object: key.id,
                    writer: dot.writer_incarnation,
                    counter: dot.counter as i64,
                }),
                max_items: limits.max_scanned as usize,
                max_bytes: limits.max_bytes as usize,
            },
        )
        .await?;
        let mut page = p::CandidatePage {
            values: vec![],
            next: after,
            done: rows.done,
        };
        for row in rows.rows {
            let candidate: p::Candidate<M> = p::decode_candidate(&row.body)?;
            ensure!(
                candidate.namespace == self.inner.store.namespace()
                    && candidate.key == *key
                    && p::candidate_digest(&candidate)?.as_slice() == row.digest.as_slice(),
                "candidate identity mismatch"
            );
            page.next = Some(candidate.dot);
            page.values.push(candidate);
        }
        while serde_json::to_vec(&page)?.len() as u64 > limits.max_bytes {
            ensure!(
                page.values.len() > 1,
                p::SyncError::LimitTooSmall {
                    required: serde_json::to_vec(&page)?.len() as u64
                }
            );
            page.values.pop();
            page.next = page.values.last().map(|candidate| candidate.dot);
            page.done = false;
        }
        tx.commit().await?;
        Ok(page)
    }
}

impl<M: p::SyncModel> p::SyncPeer<M> for CausalEndpoint<M> {
    fn identity(&self) -> BoxFuture<'_, p::StoreIdentity> {
        Box::pin(async { self.identity_impl().await.map_err(error) })
    }
    fn head(&self) -> BoxFuture<'_, p::ReceiveProgress> {
        Box::pin(async {
            causal_progress::head_on(self.inner.store.connection(), self.inner.store.namespace())
                .await
                .map_err(error)
        })
    }
    fn history_status<'a>(
        &'a self,
        key: &'a p::CheckpointKey,
        progress: Option<p::ReceiveProgress>,
    ) -> BoxFuture<'a, p::HistoryStatus> {
        Box::pin(async move { self.history(key, progress).await.map_err(error) })
    }
    fn read_page(&self, request: p::PageRequest<M>) -> BoxFuture<'_, p::PageRead<M>> {
        Box::pin(async move { self.inner.store.read_page(request).await.map_err(error) })
    }
    fn read_fragment(
        &self,
        manifest: p::ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    ) -> BoxFuture<'_, p::Fragment> {
        Box::pin(async move {
            self.fragment(manifest, ordinal, max_bytes)
                .await
                .map_err(error)
        })
    }
}

impl<M: p::SyncModel> p::ReceiveStore<M> for CausalEndpoint<M> {
    fn identity(&self) -> BoxFuture<'_, p::StoreIdentity> {
        Box::pin(async { self.identity_impl().await.map_err(error) })
    }
    fn checkpoint<'a>(
        &'a self,
        key: &'a p::CheckpointKey,
    ) -> BoxFuture<'a, Option<p::ReceiveProgress>> {
        Box::pin(async move {
            if key.namespace != self.inner.store.namespace() {
                return Err(p::SyncError::Invalid("namespace mismatch".into()));
            }
            causal_progress::checkpoint_on::<M, _>(self.inner.store.connection(), key)
                .await
                .map_err(error)
        })
    }
    fn apply_page<'a>(
        &'a self,
        page: &'a p::ChangePage<M>,
        limits: p::ApplyLimits,
    ) -> BoxFuture<'a, p::ApplyResult<M>> {
        Box::pin(async move {
            async {
                let ledger = self.ledger().await;
                let tx = self.inner.store.connection().begin().await?;
                crate::causal_leases::lock_on(&tx, self.inner.store.namespace()).await?;
                let mutation_start = mutations_on(&tx).await?;
                let key = p::CheckpointKey {
                    namespace: page.request.source.namespace,
                    source_replica: page.request.source.replica,
                    source_generation: page.request.source.generation,
                    scope_id: p::scope_id(&page.request.scope)?,
                };
                let result = self
                    .inner
                    .store
                    .apply_page_locked_on(&tx, page, limits)
                    .await?;
                self.project_with_budget_on(
                    &tx,
                    &result.projection,
                    mutation_start,
                    limits.max_mutations,
                )
                .await?;
                if page.done && result.progress == page.request.through {
                    causal_progress::finish_session_locked_on::<M>(
                        &tx,
                        self.inner.store.namespace(),
                        &key,
                        &result.progress,
                    )
                    .await?;
                }
                check_mutations_on(&tx, mutation_start, limits.max_mutations).await?;
                tx.commit().await?;
                drop(ledger);
                if !result.projection.changes.is_empty() {
                    (self.notify)();
                }
                Ok(result)
            }
            .await
            .map_err(error)
        })
    }
    fn begin_stage(
        &self,
        descriptor: p::StageDescriptor<M>,
        limits: p::StageLimits,
    ) -> BoxFuture<'_, p::FragmentProgress> {
        Box::pin(async move {
            let mut ledger = self.ledger().await;
            let namespace = match &descriptor.origin {
                p::StageOrigin::Journal { request, .. } => request.source.namespace,
                p::StageOrigin::SnapshotContext { session, .. } => {
                    ledger.descriptors(*session)?.0.source.namespace
                }
            };
            if namespace != self.inner.store.namespace() {
                return Err(p::SyncError::Invalid("namespace mismatch".into()));
            }
            ledger.begin_stage(
                Uuid::new_v4(),
                descriptor,
                limits,
                now() + p::DEFAULT_SNAPSHOT_TTL_MS,
            )
        })
    }
    fn stage_fragment(
        &self,
        stage: Uuid,
        fragment: p::Fragment,
    ) -> BoxFuture<'_, p::FragmentProgress> {
        Box::pin(async move { self.ledger().await.stage_fragment(stage, fragment) })
    }
    fn stage_status(&self, stage: Uuid) -> BoxFuture<'_, p::FragmentProgress> {
        Box::pin(async move { self.ledger().await.stage_status(stage) })
    }
    fn abandon_stage(&self, stage: Uuid) -> BoxFuture<'_, ()> {
        Box::pin(async move { self.ledger().await.abandon_stage(stage) })
    }
    fn abandon_origin(&self, origin: p::StageOrigin<M>) -> BoxFuture<'_, ()> {
        Box::pin(async move { self.ledger().await.abandon_origin(&origin) })
    }
    fn commit_fragmented_event(
        &self,
        key: p::CheckpointKey,
        sequence: u64,
        stage: Uuid,
        limits: p::ApplyLimits,
    ) -> BoxFuture<'_, p::ApplyResult<M>> {
        Box::pin(async move {
            async {
                let mut ledger = self.ledger().await;
                if let Some(progress) = ledger.completed_event(stage, &key, sequence)? {
                    return Ok(p::ApplyResult {
                        progress,
                        projection: empty_projection(),
                    });
                }
                let input = ledger.prepare_event(stage)?;
                let p::StageOrigin::Journal {
                    request,
                    sequence: expected_sequence,
                    ..
                } = &input.origin
                else {
                    anyhow::bail!(p::SyncError::StaleWork)
                };
                let expected = p::CheckpointKey {
                    namespace: request.source.namespace,
                    source_replica: request.source.replica,
                    source_generation: request.source.generation,
                    scope_id: p::scope_id(&request.scope)?,
                };
                ensure!(
                    key == expected
                        && sequence == *expected_sequence
                        && key.namespace == self.inner.store.namespace(),
                    p::SyncError::Invalid("staged event origin mismatch".into())
                );
                let tx = self.inner.store.connection().begin().await?;
                crate::causal_leases::lock_on(&tx, self.inner.store.namespace()).await?;
                let mutation_start = mutations_on(&tx).await?;
                let result = self
                    .inner
                    .store
                    .apply_staged_event_locked_on(&tx, &input, limits)
                    .await?;
                self.project_with_budget_on(
                    &tx,
                    &result.projection,
                    mutation_start,
                    limits.max_mutations,
                )
                .await?;
                if result.progress == request.through {
                    causal_progress::finish_session_locked_on::<M>(
                        &tx,
                        self.inner.store.namespace(),
                        &key,
                        &result.progress,
                    )
                    .await?;
                }
                check_mutations_on(&tx, mutation_start, limits.max_mutations).await?;
                tx.commit().await?;
                ledger.finalize_event(
                    &input,
                    key,
                    result.progress.clone(),
                    now() + p::DEFAULT_SNAPSHOT_TTL_MS,
                )?;
                drop(ledger);
                if !result.projection.changes.is_empty() {
                    (self.notify)();
                }
                Ok(result)
            }
            .await
            .map_err(error)
        })
    }
    fn finish_session(
        &self,
        key: p::CheckpointKey,
        through: p::ReceiveProgress,
    ) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            async {
                let tx = self.inner.store.connection().begin().await?;
                crate::causal_leases::lock_on(&tx, self.inner.store.namespace()).await?;
                causal_progress::finish_session_locked_on::<M>(
                    &tx,
                    self.inner.store.namespace(),
                    &key,
                    &through,
                )
                .await?;
                tx.commit().await?;
                Ok(())
            }
            .await
            .map_err(error)
        })
    }
}

impl<M: p::SyncModel> p::SyncStore<M> for CausalEndpoint<M> {
    fn local_author(&self) -> BoxFuture<'_, p::LocalAuthor> {
        Box::pin(async {
            Ok(p::LocalAuthor {
                author: self.inner.store.author(),
                writer_incarnation: self.inner.store.writer_incarnation(),
            })
        })
    }
    fn head(&self) -> BoxFuture<'_, p::ReceiveProgress> {
        p::SyncPeer::head(self)
    }
    fn history_status<'a>(
        &'a self,
        key: &'a p::CheckpointKey,
        progress: Option<p::ReceiveProgress>,
    ) -> BoxFuture<'a, p::HistoryStatus> {
        p::SyncPeer::history_status(self, key, progress)
    }
    fn get<'a>(&'a self, key: &'a p::ObjectKey<M>) -> BoxFuture<'a, Option<p::Record<M>>> {
        Box::pin(async move {
            self.inner
                .store
                .get_on(self.inner.store.connection(), key)
                .await
                .map_err(error)
        })
    }
    fn candidates<'a>(
        &'a self,
        key: &'a p::ObjectKey<M>,
        after: Option<p::Dot>,
        limits: p::PageLimits,
    ) -> BoxFuture<'a, p::CandidatePage<M>> {
        Box::pin(async move { self.candidate_page(key, after, limits).await.map_err(error) })
    }
    fn write_local(
        &self,
        key: p::ObjectKey<M>,
        value: Option<M::Payload>,
    ) -> BoxFuture<'_, p::Record<M>> {
        p::SyncStore::write_observed(self, key, value, p::Observed::AllCurrent)
    }
    fn write_observed(
        &self,
        key: p::ObjectKey<M>,
        value: Option<M::Payload>,
        observed: p::Observed,
    ) -> BoxFuture<'_, p::Record<M>> {
        Box::pin(async move {
            async {
                let tx = self.inner.store.connection().begin().await?;
                crate::causal_leases::lock_on(&tx, self.inner.store.namespace()).await?;
                let before = causal_progress::head_on(&tx, self.inner.store.namespace()).await?;
                let command = self
                    .inner
                    .store
                    .write_observed_locked_on(&tx, key, value, observed)
                    .await?;
                let after = causal_progress::head_on(&tx, self.inner.store.namespace()).await?;
                let batch = p::ProjectionBatch {
                    changes: vec![command.projection],
                    causal_changes: u64::from(before != after),
                    pending: 0,
                };
                self.project_on(&tx, &batch).await?;
                tx.commit().await?;
                if !batch.changes.is_empty() {
                    (self.notify)();
                }
                Ok(command.record)
            }
            .await
            .map_err(error)
        })
    }
    fn read_page(&self, request: p::PageRequest<M>) -> BoxFuture<'_, p::PageRead<M>> {
        p::SyncPeer::read_page(self, request)
    }
    fn read_fragment(
        &self,
        manifest: p::ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    ) -> BoxFuture<'_, p::Fragment> {
        p::SyncPeer::read_fragment(self, manifest, ordinal, max_bytes)
    }
}
impl<M: p::SyncModel> p::SnapshotSink<M> for CausalEndpoint<M> {
    fn cancel_reconciliation(&self, session: Uuid) -> BoxFuture<'_, ()> {
        Box::pin(async move { self.ledger().await.cancel_reconciliation(session) })
    }
    fn discard_range(&self, session: Uuid, range: p::KeyRange<M>) -> BoxFuture<'_, ()> {
        Box::pin(async move { self.ledger().await.discard_range(session, &range) })
    }
    fn begin_reconciliation(
        &self,
        request_id: Uuid,
        remote: p::SnapshotDescriptor<M>,
        local: p::SnapshotDescriptor<M>,
    ) -> BoxFuture<'_, Uuid> {
        Box::pin(async move {
            async {
                let mut ledger = self.ledger().await;
                let tx = self.inner.store.connection().begin().await?;
                let (_, retained) = causal_snapshot::descriptor_on::<M>(
                    &tx,
                    self.inner.store.namespace(),
                    local.snapshot_id,
                    now() as i64,
                )
                .await?;
                ensure!(
                    retained == local,
                    p::SyncError::Invalid("local snapshot mismatch".into())
                );
                let identity = self.inner.store.identity_on(&tx).await?;
                ensure!(
                    remote.source.namespace == identity.namespace
                        && remote.source.model == identity.model
                        && (remote.source.replica != identity.replica
                            || remote.source.generation != identity.generation),
                    p::SyncError::Invalid("incompatible source snapshot".into())
                );
                tx.commit().await?;
                Ok(ledger.begin_reconciliation(request_id, remote, local, now())?)
            }
            .await
            .map_err(error)
        })
    }
    fn coverage(
        &self,
        session: Uuid,
        revision: Option<u64>,
        after: u64,
        limits: p::PageLimits,
    ) -> BoxFuture<'_, p::CoveragePage<M>> {
        Box::pin(async move {
            self.ledger()
                .await
                .coverage(session, revision, after, limits)
        })
    }
    fn split_range(
        &self,
        session: Uuid,
        parent: p::KeyRange<M>,
        children: Vec<p::RangeSummary<M>>,
    ) -> BoxFuture<'_, p::CoverageProgress> {
        Box::pin(async move { self.ledger().await.split_range(session, parent, children) })
    }
    fn accept_equal_range(
        &self,
        session: Uuid,
        source: p::RangeSummary<M>,
    ) -> BoxFuture<'_, p::CoverageProgress> {
        Box::pin(async move {
            async {
                let mut ledger = self.ledger().await;
                let (_, local) = ledger.descriptors(session)?;
                let tx = self.inner.store.connection().begin().await?;
                let pinned = crate::causal_source::summary_ready_on::<M>(
                    &tx,
                    self.inner.store.namespace(),
                    local.snapshot_id,
                    &source.range,
                    now() as i64,
                )
                .await?;
                let result = ledger.accept_equal_range(session, source, &pinned)?;
                tx.commit().await?;
                Ok(result)
            }
            .await
            .map_err(error)
        })
    }
    fn leaf_progress(
        &self,
        session: Uuid,
        range: p::KeyRange<M>,
    ) -> BoxFuture<'_, p::LeafProgress<M>> {
        Box::pin(async move { self.ledger().await.leaf_progress(session, &range) })
    }
    fn stage_leaf_page(
        &self,
        session: Uuid,
        page: p::LeafPage<M>,
    ) -> BoxFuture<'_, p::LeafProgress<M>> {
        Box::pin(async move { self.ledger().await.stage_leaf_page(session, page) })
    }
    fn commit_complete_range(
        &self,
        session: Uuid,
        range: p::KeyRange<M>,
        limits: p::ApplyLimits,
    ) -> BoxFuture<'_, (p::CoverageProgress, p::ProjectionBatch<M>)> {
        Box::pin(async move {
            async {
                let mut ledger = self.ledger().await;
                if let Some(progress) = ledger.completed_range(session, &range)? {
                    return Ok((progress, empty_projection()));
                }
                validate_apply_limits(limits)?;
                let input = ledger.prepare_range(session, &range)?;
                let (remote, local) = ledger.descriptors(session)?;
                let tx = self.inner.store.connection().begin().await?;
                crate::causal_leases::lock_on(&tx, self.inner.store.namespace()).await?;
                let mutation_start = mutations_on(&tx).await?;
                causal_snapshot::descriptor_on::<M>(
                    &tx,
                    self.inner.store.namespace(),
                    local.snapshot_id,
                    now() as i64,
                )
                .await?;
                let state = self.inner.store.state_seq_on(&tx).await?;
                let raw = causal_candidates::page_on(
                    &tx,
                    causal_candidates::CandidatePageRequest {
                        namespace: self.inner.store.namespace(),
                        collection: M::collection_tag(&range.collection).as_bytes(),
                        state_seq: state,
                        lower: range.lower,
                        upper: range.upper,
                        after: None,
                        max_items: std::cmp::min(limits.max_mutations, 1024) as usize,
                        max_bytes: 4 * 1024 * 1024,
                    },
                )
                .await?;
                ensure!(raw.done, p::SyncError::WorkLimit);
                let current = raw
                    .rows
                    .iter()
                    .map(|row| -> Result<_> {
                        let candidate = p::decode_candidate::<M>(&row.body)?;
                        ensure!(
                            candidate.namespace == self.inner.store.namespace()
                                && candidate.key.collection == range.collection
                                && candidate.key.id.as_bytes() == row.object.as_slice()
                                && candidate.dot.writer_incarnation.as_bytes()
                                    == row.writer.as_slice()
                                && i64::try_from(candidate.dot.counter)? == row.counter
                                && candidate.revision.actor.as_bytes() == row.author.as_slice()
                                && i64::try_from(candidate.revision.counter)? == row.revision
                                && p::candidate_digest(&candidate)?.as_slice() == row.digest,
                            "stored candidate identity mismatch"
                        );
                        Ok(candidate)
                    })
                    .collect::<Result<Vec<_>>>()?;
                let mut observed = BTreeSet::new();
                for c in &input.candidates {
                    if crate::causal_observed::contains_on(
                        &tx,
                        self.inner.store.namespace(),
                        M::collection_tag(&range.collection).as_bytes(),
                        Dot::new(c.dot.writer_incarnation, c.dot.counter)?,
                    )
                    .await?
                    {
                        observed.insert(c.dot);
                    }
                }
                let mut budget = limits.max_mutations;
                let projection = match p::plan_leaf_merge(
                    &range.collection,
                    &current,
                    &input.candidates,
                    &input.source_context,
                    &observed,
                )? {
                    Some(delta) => {
                        self.inner
                            .store
                            .apply_merge_locked_on(&tx, &delta, &remote.source, limits, &mut budget)
                            .await?
                            .1
                    }
                    None => empty_projection(),
                };
                self.project_with_budget_on(&tx, &projection, mutation_start, limits.max_mutations)
                    .await?;
                check_mutations_on(&tx, mutation_start, limits.max_mutations).await?;
                tx.commit().await?;
                let coverage = ledger.finalize_range(&input.token)?;
                drop(ledger);
                if !projection.changes.is_empty() {
                    (self.notify)();
                }
                Ok((coverage, projection))
            }
            .await
            .map_err(error)
        })
    }
    fn promote_step(
        &self,
        session: Uuid,
        limits: p::ApplyLimits,
    ) -> BoxFuture<'_, p::PromoteProgress> {
        Box::pin(async move {
            async {
                validate_apply_limits(limits)?;
                let mut ledger = self.ledger().await;
                let Some(token) = ledger.prepare_promotion(
                    session,
                    (limits.max_mutations / 4).clamp(1, 1024) as usize,
                )?
                else {
                    return Ok(p::PromoteProgress {
                        done: true,
                        causal_changes: 0,
                    });
                };
                let (remote, local) = ledger.descriptors(session)?;
                let tx = self.inner.store.connection().begin().await?;
                crate::causal_leases::lock_on(&tx, self.inner.store.namespace()).await?;
                let mutation_start = mutations_on(&tx).await?;
                let (lease, _) = causal_snapshot::descriptor_on::<M>(
                    &tx,
                    self.inner.store.namespace(),
                    local.snapshot_id,
                    now() as i64,
                )
                .await?;
                let ns = self.inner.store.namespace();
                let mut observed = Context::default();
                let collection = match &token.step {
                    p::PromotionStep::Arrivals {
                        collection,
                        source_context,
                        source_live,
                    } => {
                        let q = Query::select()
                            .columns(["writer", "counter"].map(Alias::new))
                            .from(Alias::new("sync_candidates"))
                            .and_where(
                                Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()),
                            )
                            .and_where(
                                Expr::col(Alias::new("collection")).eq(M::collection_tag(
                                    collection,
                                )
                                .as_bytes()
                                .to_vec()),
                            )
                            .and_where(Expr::col(Alias::new("live_from")).gt(lease.state_seq))
                            .and_where(Expr::col(Alias::new("live_until")).is_null())
                            .limit(limits.max_mutations + 1)
                            .to_owned();
                        let rows = tx.query_all(&q).await?;
                        ensure!(
                            rows.len() as u64 <= limits.max_mutations,
                            p::SyncError::WorkLimit
                        );
                        for row in rows {
                            let dot = Dot::new(
                                Uuid::from_slice(&row.try_get_by_index::<Vec<u8>>(0)?)?,
                                u64::try_from(row.try_get_by_index::<i64>(1)?)?,
                            )?;
                            if source_context.contains(dot) && !source_live.contains(dot) {
                                observed.insert(dot)?;
                            }
                        }
                        collection.clone()
                    }
                    p::PromotionStep::Chunk { collection, chunk } => {
                        for (writer, first, last) in chunk.intervals() {
                            for (a, b) in crate::causal_observed::unobserved_on(
                                &tx,
                                ns,
                                M::collection_tag(collection).as_bytes(),
                                writer,
                                first,
                                last,
                            )
                            .await?
                            {
                                observed.insert_interval(writer, a, b)?;
                            }
                        }
                        collection.clone()
                    }
                };
                let mut budget = limits.max_mutations;
                let projection = if observed.is_empty() {
                    empty_projection()
                } else {
                    self.inner
                        .store
                        .apply_merge_locked_on(
                            &tx,
                            &p::MergeDelta {
                                collection,
                                added: vec![],
                                observed: p::exact_context(&observed),
                            },
                            &remote.source,
                            limits,
                            &mut budget,
                        )
                        .await?
                        .1
                };
                self.project_with_budget_on(&tx, &projection, mutation_start, limits.max_mutations)
                    .await?;
                check_mutations_on(&tx, mutation_start, limits.max_mutations).await?;
                tx.commit().await?;
                ledger.finalize_promotion(&token)?;
                let done = ledger.promotion_done(session)?;
                drop(ledger);
                if !projection.changes.is_empty() {
                    (self.notify)();
                }
                Ok(p::PromoteProgress {
                    done,
                    causal_changes: projection.causal_changes,
                })
            }
            .await
            .map_err(error)
        })
    }
    fn complete_reconciliation(&self, session: Uuid) -> BoxFuture<'_, p::ReceiveProgress> {
        Box::pin(async move {
            async {
                let mut ledger = self.ledger().await;
                if let Some(progress) = ledger.completed_session(session) {
                    return Ok(progress);
                }
                let admission = ledger.prepare_completion(session)?;
                let (remote, local) = ledger.descriptors(session)?;
                let tx = self.inner.store.connection().begin().await?;
                crate::causal_leases::lock_on(&tx, self.inner.store.namespace()).await?;
                causal_snapshot::descriptor_on::<M>(
                    &tx,
                    self.inner.store.namespace(),
                    local.snapshot_id,
                    now() as i64,
                )
                .await?;
                let identity = self.inner.store.identity_on(&tx).await?;
                let progress = causal_progress::initialize_checkpoint_on(
                    &tx,
                    &identity,
                    &admission.checkpoint,
                    &remote.scope,
                    &admission.through,
                )
                .await?;
                tx.commit().await?;
                Ok(ledger.finalize_completion(&admission, progress)?)
            }
            .await
            .map_err(error)
        })
    }
}

/// SQLite counts all changes on this transaction's connection, including
/// trigger effects. Check again after projection and completion publication.
async fn mutations_on(tx: &DatabaseTransaction) -> Result<u64> {
    let query = Query::select()
        .expr(Func::cust(Alias::new("total_changes")))
        .to_owned();
    let count: i64 = tx
        .query_one(&query)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing mutation count"))?
        .try_get_by_index(0)?;
    Ok(count.try_into()?)
}
async fn check_mutations_on(tx: &DatabaseTransaction, start: u64, limit: u64) -> Result<()> {
    ensure!(
        mutations_on(tx)
            .await?
            .checked_sub(start)
            .is_some_and(|used| used <= limit),
        p::SyncError::WorkLimit
    );
    Ok(())
}

async fn read_fragment_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    manifest: p::ManifestRef,
    ordinal: u32,
    max_bytes: u64,
) -> Result<p::Fragment> {
    ensure!(
        max_bytes > 0 && max_bytes <= p::MAX_PAGE_BYTES && ordinal < manifest.fragment_count,
        p::SyncError::Invalid("invalid fragment request".into())
    );
    ensure!(
        causal_manifests::reference_on(tx, namespace, manifest.manifest_digest).await? == manifest,
        p::SyncError::Invalid("manifest mismatch".into())
    );
    let mut q = Query::select();
    q.columns(["offset", "digest"].map(Alias::new))
        .expr(Func::cust(Alias::new("length")).arg(Expr::col(Alias::new("bytes"))))
        .from(Alias::new("sync_chunks"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("manifest")).eq(manifest.manifest_digest.to_vec()))
        .and_where(Expr::col(Alias::new("ordinal")).eq(i64::from(ordinal)));
    let header = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing retained fragment"))?;
    let offset = u64::try_from(header.try_get_by_index::<i64>(0)?)?;
    let digest: Vec<u8> = header.try_get_by_index(1)?;
    let size = u64::try_from(header.try_get_by_index::<i64>(2)?)?;
    ensure!(
        size > 0
            && size <= p::MAX_PAGE_BYTES
            && offset
                .checked_add(size)
                .is_some_and(|end| end <= manifest.encoded_bytes),
        "invalid fragment extent"
    );
    ensure!(
        size <= max_bytes,
        p::SyncError::LimitTooSmall { required: size }
    );
    q.clear_selects().column(Alias::new("bytes"));
    let bytes: Vec<u8> = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing fragment body"))?
        .try_get_by_index(0)?;
    let fragment = p::Fragment {
        manifest,
        ordinal,
        offset,
        bytes,
        digest: digest
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid fragment digest"))?,
    };
    ensure!(
        fragment.bytes.len() as u64 == size && p::fragment_digest(&fragment)? == fragment.digest,
        "fragment integrity mismatch"
    );
    Ok(fragment)
}

fn validate_apply_limits(limits: p::ApplyLimits) -> Result<()> {
    ensure!(
        limits.max_projection_items > 0
            && limits.max_projection_items <= p::MAX_PROJECTION_ITEMS
            && limits.max_projection_bytes > 0
            && limits.max_projection_bytes <= p::MAX_PROJECTION_BYTES
            && limits.max_mutations > 0
            && limits.max_mutations <= p::MAX_MUTATIONS,
        p::SyncError::Invalid("invalid apply limits".into())
    );
    Ok(())
}

#[cfg(test)]
mod request_budget_tests {
    use super::*;

    fn limits() -> p::WorkLimits {
        p::WorkLimits {
            max_requests: 128,
            max_rows_scanned: 20,
            max_bytes: 100,
            max_mutations: 30,
            max_depth: 4,
        }
    }

    #[test]
    fn reservations_accumulate_and_rejections_preserve_remaining_work() {
        let full = limits();
        let mut budget = SourceRequestBudget::new(full);
        let step = p::WorkLimits {
            max_rows_scanned: 5,
            max_bytes: 16,
            max_mutations: 6,
            ..full
        };
        for _ in 0..3 {
            assert!(budget.reserve(step));
        }
        for rejected in [
            p::WorkLimits {
                max_rows_scanned: 6,
                ..step
            },
            p::WorkLimits {
                max_bytes: 53,
                ..step
            },
            p::WorkLimits {
                max_mutations: 13,
                ..step
            },
        ] {
            assert!(!budget.reserve(rejected));
            assert_eq!(
                (
                    budget.rows,
                    budget.bytes,
                    budget.mutations,
                    budget.transactions
                ),
                (5, 52, 12, 61)
            );
        }
        assert!(budget.reserve(step));
        assert!(!budget.has_work());
        assert!(!budget.reserve(step));
    }

    #[test]
    fn request_transaction_limit_is_independent_of_content_allowance() {
        let full = p::WorkLimits {
            max_requests: 1,
            ..limits()
        };
        let mut budget = SourceRequestBudget::new(full);
        let step = p::WorkLimits {
            max_rows_scanned: 1,
            max_bytes: 1,
            max_mutations: 1,
            ..full
        };
        assert!(budget.reserve(step));
        assert!(!budget.has_work());
        assert!(!budget.reserve(step));
        assert_eq!((budget.rows, budget.bytes, budget.mutations), (19, 99, 29));
    }

    #[test]
    fn unused_first_step_allowance_keeps_the_transaction_charged() -> Result<()> {
        let full = limits();
        let mut budget = SourceRequestBudget::new(full);
        assert!(budget.reserve(full));
        let charged = p::WorkLimits {
            max_rows_scanned: 5,
            max_bytes: 16,
            max_mutations: 6,
            ..full
        };
        SourceRequestBudget::validate_charge(full, charged)?;
        budget.release_unused(full, charged);
        assert_eq!(
            (
                budget.rows,
                budget.bytes,
                budget.mutations,
                budget.transactions
            ),
            (15, 84, 24, 63)
        );
        assert!(SourceRequestBudget::validate_charge(charged, full).is_err());
        assert!(budget.reserve(charged));
        assert_eq!(budget.transactions, 62);
        Ok(())
    }
}
