//! Bounded caller-scheduled history and immutable-object maintenance.
use anyhow::Result;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, ExprTrait, TransactionTrait,
    sea_query::{Alias, Expr, Query},
};
use sync::protocol::{self as p, SyncModel};
use uuid::Uuid;

#[derive(Clone, Copy, Default)]
enum Phase {
    #[default]
    Expire,
    Baseline,
    History,
    Basis,
    Objects,
}
#[derive(Clone, Default)]
pub struct Maintenance {
    namespace: Option<Uuid>,
    phase: Phase,
    request: Option<Uuid>,
    resume: Option<p::WorkCursor>,
    basis: Option<crate::causal_gc::BasisCursor>,
    objects: Option<p::Digest>,
}
impl Maintenance {
    /// Commit a single bounded unit before updating its in-memory continuation.
    pub async fn advance<M: SyncModel>(
        &mut self,
        db: &DatabaseConnection,
        identity: p::StoreIdentity,
        now_ms: i64,
    ) -> Result<()> {
        let mut next = if self.namespace == Some(identity.namespace) {
            self.clone()
        } else {
            Self {
                namespace: Some(identity.namespace),
                ..Self::default()
            }
        };
        let tx = db.begin().await?;
        if let Err(error) = next.step_on::<M>(&tx, identity.clone(), now_ms).await {
            tx.rollback().await?;
            if matches!(
                error.downcast_ref::<p::SyncError>(),
                Some(p::SyncError::StaleWork | p::SyncError::SnapshotExpired)
            ) {
                // Another maintenance task may have advanced the captured
                // basis. Release this build before beginning from that basis.
                let cleanup = db.begin().await?;
                if let Some(request) = self.request {
                    crate::causal_source::release_request_on(&cleanup, identity.namespace, request)
                        .await?;
                }
                cleanup.commit().await?;
                *self = Self {
                    namespace: Some(identity.namespace),
                    ..Self::default()
                };
                return Ok(());
            }
            return Err(error);
        }
        tx.commit().await?;
        *self = next;
        Ok(())
    }
    async fn step_on<M: SyncModel>(
        &mut self,
        tx: &DatabaseTransaction,
        identity: p::StoreIdentity,
        now_ms: i64,
    ) -> Result<()> {
        let ns = identity.namespace;
        match self.phase {
            Phase::Expire => {
                crate::causal_leases::revoke_expired_on(tx, ns, now_ms, 16).await?;
                let query = Query::select()
                    .column(Alias::new("id"))
                    .from(Alias::new("sync_leases"))
                    .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                    .and_where(Expr::col(Alias::new("status")).eq(2))
                    .order_by(Alias::new("id"), sea_orm::sea_query::Order::Asc)
                    .limit(16)
                    .to_owned();
                for row in tx.query_all(&query).await? {
                    let id: Vec<u8> = row.try_get_by_index(0)?;
                    tx.execute(
                        &Query::delete()
                            .from_table(Alias::new("sync_leases"))
                            .and_where(
                                Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()),
                            )
                            .and_where(Expr::col(Alias::new("id")).eq(id))
                            .and_where(Expr::col(Alias::new("status")).eq(2))
                            .to_owned(),
                    )
                    .await?;
                }
                self.phase = Phase::Baseline;
            }
            Phase::Baseline => {
                if let Some(request) = self.request {
                    let active = crate::causal_leases::load_on(tx, ns, request).await?;
                    if active.is_none_or(|lease| lease.status == 2 || lease.expires_at_ms <= now_ms)
                    {
                        self.request = None;
                        self.resume = None;
                    }
                }
                if self.request.is_none() {
                    let query = Query::select()
                        .columns(
                            ["head", "history_floor", "generation", "baseline_generation"]
                                .map(Alias::new),
                        )
                        .from(Alias::new("sync_meta"))
                        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                        .to_owned();
                    let row = tx
                        .query_one(&query)
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("missing namespace"))?;
                    if row.try_get_by_index::<i64>(0)? == row.try_get_by_index::<i64>(1)? {
                        self.phase = Phase::History;
                        return Ok(());
                    }
                    // Processes sharing a namespace also share its floor build.
                    // Separate builds would pin each other's old basis forever.
                    let mut name = b"sync-orm/maintenance/baseline/".to_vec();
                    name.extend(row.try_get_by_index::<Vec<u8>>(2)?);
                    name.extend(row.try_get_by_index::<Vec<u8>>(3)?);
                    self.request = Some(Uuid::new_v5(&ns, &name));
                }
                let limits = p::WorkLimits {
                    max_requests: 16,
                    max_bytes: 4 * 1024 * 1024,
                    max_depth: p::MAX_DEPTH,
                    max_rows_scanned: 256,
                    max_mutations: 1024,
                };
                match crate::causal_compaction::advance_baseline_on::<M>(
                    tx,
                    identity,
                    self.request.unwrap(),
                    now_ms,
                    limits,
                    self.resume.clone(),
                )
                .await?
                {
                    crate::causal_compaction::BaselineAdvance::Incomplete { cursor } => {
                        self.resume = Some(cursor)
                    }
                    crate::causal_compaction::BaselineAdvance::Pinned { cursor } => {
                        self.resume = Some(cursor);
                        self.phase = Phase::History;
                    }
                    crate::causal_compaction::BaselineAdvance::Advanced { .. } => {
                        self.request = None;
                        self.resume = None;
                        self.phase = Phase::History;
                    }
                }
            }
            Phase::History => {
                crate::causal_compaction::prune_history_step_on(tx, ns, 64).await?;
                self.phase = Phase::Basis;
            }
            Phase::Basis => {
                let step =
                    crate::causal_gc::reclaim_basis_step_on(tx, ns, self.basis.clone(), 64).await?;
                self.basis = step.next;
                self.phase = Phase::Objects;
            }
            Phase::Objects => {
                let step =
                    crate::causal_gc::step_on(tx, ns, self.objects, 16, 64, 1024 * 1024).await?;
                self.objects = if step.done { None } else { step.next };
                self.phase = Phase::Expire;
            }
        }
        Ok(())
    }
}
