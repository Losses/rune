//! Capability composition over one SQL replica and caller-owned projections.
use crate::{
    causal_candidates, causal_manifests, causal_progress, causal_snapshot,
    causal_store::CausalReplica,
};
use anyhow::{Result, ensure};
use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, ExprTrait, TransactionTrait,
    sea_query::{Alias, Expr, Func, Query},
};
use std::sync::Arc;
use std::time::{Duration, Instant};
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
}
struct EndpointState<M: p::SyncModel> {
    store: CausalReplica<M>,
    projection: Arc<dyn TransactionProjection<M>>,
    receiver: Mutex<p::ReceiverLedger<M>>,
}
fn error(error: anyhow::Error) -> p::SyncError {
    error
        .downcast_ref::<p::SyncError>()
        .cloned()
        .unwrap_or_else(|| p::SyncError::Storage(format!("{error:#}")))
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
                    let step = crate::causal_source::open_batch_step_on(
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
                let ns = self.inner.store.namespace();
                let value = crate::causal_source::summarize_on(
                    &tx,
                    ns,
                    snapshot,
                    range,
                    now() as i64,
                    limits,
                    resume,
                )
                .await?;
                tx.commit().await?;
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
                let ns = self.inner.store.namespace();
                let value = crate::causal_source::split_on(
                    &tx,
                    ns,
                    snapshot,
                    range,
                    now() as i64,
                    limits,
                    resume,
                )
                .await?;
                tx.commit().await?;
                Ok(value)
            }
            .await;
            result.map_err(error)
        })
    }
    fn context_page(
        &self,
        snapshot: Uuid,
        range: p::KeyRange<M>,
        after: p::ContextCursor,
        limits: p::PageLimits,
    ) -> BoxFuture<'_, p::ContextPage<M>> {
        Box::pin(async move {
            let result: Result<p::ContextPage<M>> = async {
                let tx = self.inner.store.connection().begin().await?;
                let ns = self.inner.store.namespace();
                let value = causal_snapshot::context_page_on(
                    &tx,
                    ns,
                    snapshot,
                    now() as i64,
                    range,
                    after,
                    limits,
                )
                .await?;
                tx.commit().await?;
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
        }
    }
    /// Shares the replica, writer and receive ledger; only its post-commit
    /// notification target changes. No notification runs inside a transaction.
    pub fn with_notification(mut self, notify: Arc<dyn Fn() + Send + Sync>) -> Self {
        self.notify = notify;
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
                let mut ledger = self.ledger().await;
                let tx = self.inner.store.connection().begin().await?;
                let mutation_start = mutations_on(&tx).await?;
                let key = p::CheckpointKey {
                    namespace: page.request.source.namespace,
                    source_replica: page.request.source.replica,
                    source_generation: page.request.source.generation,
                    scope_id: p::scope_id(&page.request.scope)?,
                };
                let current = causal_progress::checkpoint_on::<M, _>(&tx, &key).await?;
                let mut prepared = Vec::new();
                if current
                    .as_ref()
                    .is_some_and(|current| current.sequence < page.next.sequence)
                {
                    let mut anchor = page.request.after.anchor;
                    for link in &page.links {
                        let preceding = p::ReceiveProgress {
                            sequence: link
                                .sequence
                                .checked_sub(1)
                                .ok_or(p::SyncError::Invalid("zero journal sequence".into()))?,
                            anchor,
                        };
                        anchor = p::next_anchor::<M>(
                            anchor,
                            link.sequence,
                            &link.collection,
                            link.event_digest,
                        )?;
                        if let Some(p::EventRef::Inline(event @ p::Event::Range(range))) =
                            &link.event
                        {
                            let mut request = page.request.clone();
                            request.after = preceding;
                            let origin = p::StageOrigin::Journal {
                                request,
                                sequence: link.sequence,
                                collection: link.collection.clone(),
                                event_digest: link.event_digest,
                            };
                            let input = ledger.prepare_inline_event(&origin, event)?;
                            publish_dependencies_on(
                                &tx,
                                self.inner.store.namespace(),
                                &range.range,
                                &input.dependencies,
                                &input.dependency_fragments,
                            )
                            .await?;
                            prepared.push((
                                input,
                                p::ReceiveProgress {
                                    sequence: link.sequence,
                                    anchor,
                                },
                            ));
                        }
                    }
                }
                let result = self.inner.store.apply_page_on(&tx, page, limits).await?;
                self.project_with_budget_on(
                    &tx,
                    &result.projection,
                    mutation_start,
                    limits.max_mutations,
                )
                .await?;
                if page.done && result.progress == page.request.through {
                    causal_progress::finish_session_on::<M>(
                        &tx,
                        self.inner.store.namespace(),
                        &key,
                        &result.progress,
                    )
                    .await?;
                }
                check_mutations_on(&tx, mutation_start, limits.max_mutations).await?;
                tx.commit().await?;
                for (input, progress) in prepared {
                    ledger.finalize_event(
                        &input,
                        key.clone(),
                        progress,
                        now() + p::DEFAULT_SNAPSHOT_TTL_MS,
                    )?;
                }
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
                p::StageOrigin::Snapshot { session, .. } => {
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
                let mutation_start = mutations_on(&tx).await?;
                if let p::Event::Range(range) = &input.event {
                    publish_dependencies_on(
                        &tx,
                        self.inner.store.namespace(),
                        &range.range,
                        &input.dependencies,
                        &input.dependency_fragments,
                    )
                    .await?;
                }
                let result = self
                    .inner
                    .store
                    .apply_staged_event_on(&tx, &input, limits)
                    .await?;
                self.project_with_budget_on(
                    &tx,
                    &result.projection,
                    mutation_start,
                    limits.max_mutations,
                )
                .await?;
                if result.progress == request.through {
                    causal_progress::finish_session_on::<M>(
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
                causal_progress::finish_session_on::<M>(
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
                let before = causal_progress::head_on(&tx, self.inner.store.namespace()).await?;
                let command = self
                    .inner
                    .store
                    .write_observed_on(&tx, key, value, observed)
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

async fn publish_dependencies_on<M: p::SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    range: &p::KeyRange<M>,
    dependencies: &[(p::ManifestRef, Vec<u8>)],
    retained_fragments: &[(p::Digest, Vec<p::Fragment>)],
) -> Result<()> {
    crate::causal_leases::lock_on(tx, namespace).await?;
    let mut unique = std::collections::BTreeMap::new();
    for (manifest, body) in dependencies {
        if let Some((previous, bytes)) = unique.insert(manifest.manifest_digest, (manifest, body)) {
            ensure!(
                previous == manifest && bytes == body,
                "conflicting prepared dependency"
            );
        }
    }
    // Children must exist before a parent can publish its dependency edges.
    for kind in [
        p::ManifestKind::Context,
        p::ManifestKind::Leaf,
        p::ManifestKind::ContextPartition,
    ] {
        for (manifest, body) in unique
            .values()
            .copied()
            .filter(|(manifest, _)| manifest.kind == kind)
        {
            let generated;
            let fragments = if let Some((_, fragments)) = retained_fragments
                .iter()
                .find(|(digest, _)| *digest == manifest.manifest_digest)
            {
                fragments.as_slice()
            } else {
                let (actual, fragments) = causal_manifests::fragment_body(
                    manifest.kind,
                    body,
                    manifest.item_count,
                    64 * 1024,
                )?;
                ensure!(&actual == manifest, "generated manifest layout mismatch");
                generated = fragments;
                generated.as_slice()
            };
            causal_manifests::publish_on(
                tx,
                namespace,
                manifest,
                fragments,
                p::MAX_LOGICAL_BYTES,
                |bytes| {
                    ensure!(
                        bytes == body.as_slice(),
                        "prepared dependency body mismatch"
                    );
                    Ok(match kind {
                        p::ManifestKind::Context => {
                            p::decode_context(bytes)?;
                            vec![]
                        }
                        p::ManifestKind::Leaf => {
                            let values = p::decode_candidates::<M>(bytes)?;
                            ensure!(
                                values.iter().all(|value| value.namespace == namespace),
                                "candidate namespace mismatch"
                            );
                            vec![]
                        }
                        p::ManifestKind::ContextPartition => {
                            p::decode_context_partition::<M>(range, bytes)?
                                .into_iter()
                                .map(|segment| segment.context.manifest.manifest_digest)
                                .collect()
                        }
                        _ => unreachable!(),
                    })
                },
            )
            .await?;
        }
    }
    Ok(())
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
    fn context_progress(
        &self,
        session: Uuid,
        range: p::KeyRange<M>,
    ) -> BoxFuture<'_, p::ContextProgress> {
        Box::pin(async move { self.ledger().await.context_progress(session, &range) })
    }
    fn leaf_progress(
        &self,
        session: Uuid,
        range: p::KeyRange<M>,
    ) -> BoxFuture<'_, p::LeafProgress<M>> {
        Box::pin(async move { self.ledger().await.leaf_progress(session, &range) })
    }
    fn stage_context_page(
        &self,
        session: Uuid,
        page: p::ContextPage<M>,
    ) -> BoxFuture<'_, p::ContextProgress> {
        Box::pin(async move { self.ledger().await.stage_context_page(session, page) })
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
                let input = ledger.prepare_range(session, &range)?;
                let completion = ledger.prepare_completion_after_range(&input.token)?;
                let (remote, local) = ledger.descriptors(session)?;
                let tx = self.inner.store.connection().begin().await?;
                let mutation_start = mutations_on(&tx).await?;
                causal_snapshot::descriptor_on::<M>(
                    &tx,
                    self.inner.store.namespace(),
                    local.snapshot_id,
                    now() as i64,
                )
                .await?;
                publish_dependencies_on(
                    &tx,
                    self.inner.store.namespace(),
                    &range,
                    &input.dependencies,
                    &input.dependency_fragments,
                )
                .await?;
                let verified = crate::causal_range::validate_on(
                    &tx,
                    self.inner.store.namespace(),
                    &input.event,
                    p::MAX_LOGICAL_BYTES,
                    p::MAX_LOGICAL_ITEMS,
                    p::MAX_FRAGMENTS,
                )
                .await?;
                let projection = self
                    .inner
                    .store
                    .apply_range_on(&tx, &verified, limits)
                    .await?;
                self.project_with_budget_on(&tx, &projection, mutation_start, limits.max_mutations)
                    .await?;
                if let Some(admission) = &completion {
                    causal_progress::initialize_checkpoint_on(
                        &tx,
                        &self.inner.store.identity_on(&tx).await?,
                        &admission.checkpoint,
                        &remote.scope,
                        &admission.through,
                    )
                    .await?;
                }
                check_mutations_on(&tx, mutation_start, limits.max_mutations).await?;
                tx.commit().await?;
                let coverage = ledger.finalize_range(&input.token)?;
                if let Some(admission) = &completion {
                    ledger.mark_completion_published(admission, admission.through.clone())?;
                }
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
    fn normalize_step(
        &self,
        session: Uuid,
        limits: p::WorkLimits,
        resume: Option<p::WorkCursor>,
    ) -> BoxFuture<'_, p::WorkOutcome<p::CoverageProgress>> {
        Box::pin(async move {
            if resume.is_some() {
                return Err(p::SyncError::StaleWork);
            }
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
                return Err(p::SyncError::Invalid("invalid normalization budget".into()));
            }
            // Accepted assertions already have exact set semantics. Physical
            // baseline normalization is performed by bounded maintenance.
            let progress = self.ledger().await.progress(session)?;
            if !progress.complete {
                return Err(p::SyncError::IncompleteCoverage);
            }
            Ok(p::WorkOutcome::Ready(progress))
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
                if let Some(progress) = ledger.published_completion(session)? {
                    return Ok(ledger.finalize_completion(&admission, progress)?);
                }
                let (remote, local) = ledger.descriptors(session)?;
                let tx = self.inner.store.connection().begin().await?;
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
