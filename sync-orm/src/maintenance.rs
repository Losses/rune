//! Bounded caller-scheduled history and immutable-object maintenance.
use anyhow::Result;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, ExprTrait, TransactionTrait,
    sea_query::{Alias, Expr, Query},
};
use sync::protocol::{self as p, SyncModel};
use uuid::Uuid;
#[derive(Clone, Copy, Default)]
enum Phase {
    #[default]
    Revoke,
    Release,
    Floor,
    Prune,
    Rows,
    Objects,
}
#[derive(Clone, Default)]
pub struct Maintenance {
    namespace: Option<Uuid>,
    phase: Phase,
    objects: Option<p::Digest>,
}
impl Maintenance {
    /// Commit one bounded unit before advancing its volatile continuation.
    pub async fn advance<M: SyncModel>(
        &mut self,
        db: &DatabaseConnection,
        identity: p::StoreIdentity,
        now_ms: i64,
    ) -> Result<()> {
        let ns = identity.namespace;
        let mut next = if self.namespace == Some(ns) {
            self.clone()
        } else {
            Self {
                namespace: Some(ns),
                ..Self::default()
            }
        };
        let tx = db.begin().await?;
        crate::causal_leases::lock_on(&tx, ns).await?;
        match next.phase {
            Phase::Revoke => {
                crate::causal_leases::revoke_expired_on(&tx, ns, now_ms, 16).await?;
                next.phase = Phase::Release;
            }
            Phase::Release => {
                let expired = Query::select()
                    .column(Alias::new("id"))
                    .from(Alias::new("sync_leases"))
                    .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                    .and_where(Expr::col(Alias::new("status")).eq(2))
                    .limit(16)
                    .to_owned();
                tx.execute(
                    &Query::delete()
                        .from_table(Alias::new("sync_leases"))
                        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                        .and_where(Expr::col(Alias::new("status")).eq(2))
                        .and_where(Expr::col(Alias::new("id")).in_subquery(expired))
                        .to_owned(),
                )
                .await?;
                next.phase = Phase::Floor;
            }
            Phase::Floor => {
                crate::causal_compaction::maintenance_step_on::<M>(&tx, ns, 64, 64, 1024 * 1024)
                    .await?;
                next.phase = Phase::Prune;
            }
            Phase::Prune => {
                crate::causal_compaction::prune_history_step_on(&tx, ns, 64).await?;
                next.phase = Phase::Rows;
            }
            Phase::Rows => {
                crate::causal_compaction::reclaim_rows_step_on(&tx, ns, 64).await?;
                next.phase = Phase::Objects;
            }
            Phase::Objects => {
                let step =
                    crate::causal_gc::step_on(&tx, ns, next.objects, 16, 64, 1024 * 1024).await?;
                next.objects = if step.done { None } else { step.next };
                next.phase = Phase::Revoke;
            }
        }
        tx.commit().await?;
        *self = next;
        Ok(())
    }
}
