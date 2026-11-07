//! Bounded journal-floor advancement and historical-row reclamation.
use crate::causal_leases::{lock_on, minimum_pins_on};
use anyhow::{Result, ensure};
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, ExprTrait,
    sea_query::{Alias, Condition, Expr, Order, Query},
};
use sync::protocol::SyncModel;
use uuid::Uuid;

/// Advance to an exact retained state without exceeding active lease pins.
pub async fn compact_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    ns: Uuid,
    target: i64,
    max_rows: u64,
    max_bytes: u64,
) -> Result<()> {
    lock_on(tx, ns).await?;
    ensure!(max_rows > 0 && max_bytes > 0, "invalid compaction budget");
    let meta = tx
        .query_one(
            &Query::select()
                .columns(
                    [
                        "model",
                        "schema",
                        "history_floor",
                        "floor_state_seq",
                        "state_seq",
                    ]
                    .map(Alias::new),
                )
                .from(Alias::new("sync_meta"))
                .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                .to_owned(),
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing namespace"))?;
    ensure!(
        meta.try_get_by_index::<String>(0)? == M::model_id()
            && meta.try_get_by_index::<String>(1)? == M::schema_id(),
        "incompatible model"
    );
    let floor: i64 = meta.try_get_by_index(2)?;
    let state_floor: i64 = meta.try_get_by_index(3)?;
    ensure!(
        target >= state_floor && target <= meta.try_get_by_index::<i64>(4)?,
        "invalid floor target"
    );
    if target == state_floor {
        return Ok(());
    }
    let q = Query::select()
        .columns(["sequence", "anchor_after"].map(Alias::new))
        .from(Alias::new("sync_journal"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("state_seq")).eq(target))
        .to_owned();
    let row = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing retained floor event"))?;
    let sequence: i64 = row.try_get_by_index(0)?;
    let anchor: Vec<u8> = row.try_get_by_index(1)?;
    ensure!(
        sequence > floor && anchor.len() == 32,
        "invalid retained floor event"
    );
    if let Some((state, head)) = minimum_pins_on(tx, ns).await? {
        ensure!(
            target <= state && sequence <= head,
            "floor target exceeds active lease pin"
        );
    }
    tx.execute(
        &Query::update()
            .table(Alias::new("sync_meta"))
            .value(Alias::new("history_floor"), sequence)
            .value(Alias::new("floor_state_seq"), target)
            .value(Alias::new("floor_anchor"), anchor)
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
            .to_owned(),
    )
    .await?;
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FloorStep {
    Advanced { state_seq: i64 },
    Pinned,
    CaughtUp,
}
/// Advance by at most `max_events` committed events, respecting every source pin.
pub async fn maintenance_step_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    ns: Uuid,
    max_events: u64,
    max_rows: u64,
    max_bytes: u64,
) -> Result<FloorStep> {
    lock_on(tx, ns).await?;
    ensure!(max_events > 0 && max_events <= 128, "invalid floor budget");
    let row = tx
        .query_one(
            &Query::select()
                .columns(["head", "history_floor", "state_seq"].map(Alias::new))
                .from(Alias::new("sync_meta"))
                .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                .to_owned(),
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing namespace"))?;
    let head: i64 = row.try_get_by_index(0)?;
    let floor: i64 = row.try_get_by_index(1)?;
    if floor == head {
        return Ok(FloorStep::CaughtUp);
    }
    let pins = minimum_pins_on(tx, ns).await?;
    let cap = std::cmp::min(
        pins.map_or(head, |(_, pin)| std::cmp::min(pin, head)),
        floor.saturating_add(i64::try_from(max_events)?),
    );
    if cap <= floor {
        return Ok(FloorStep::Pinned);
    }
    let q = Query::select()
        .column(Alias::new("state_seq"))
        .from(Alias::new("sync_journal"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("sequence")).eq(cap))
        .to_owned();
    let target = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing floor target"))?
        .try_get_by_index::<i64>(0)?;
    compact_on::<M>(tx, ns, target, max_rows, max_bytes).await?;
    Ok(FloorStep::Advanced { state_seq: target })
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneStep {
    pub removed: u64,
    pub more: bool,
}
/// Delete a bounded prefix at or below the published journal floor.
pub async fn prune_history_step_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    max_rows: u64,
) -> Result<PruneStep> {
    lock_on(tx, ns).await?;
    ensure!(max_rows > 0 && max_rows <= 1024, "invalid prune budget");
    let meta = tx
        .query_one(
            &Query::select()
                .column(Alias::new("history_floor"))
                .from(Alias::new("sync_meta"))
                .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                .to_owned(),
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing namespace"))?;
    let floor: i64 = meta.try_get_by_index(0)?;
    let q = Query::select()
        .columns(["generation", "sequence"].map(Alias::new))
        .from(Alias::new("sync_journal"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("sequence")).lte(floor))
        .order_by(Alias::new("sequence"), Order::Asc)
        .limit(max_rows)
        .to_owned();
    let rows = tx.query_all(&q).await?;
    let mut removed = 0;
    for batch in rows.chunks(100) {
        let mut matches = Condition::any();
        for row in batch {
            matches = matches.add(
                Condition::all()
                    .add(
                        Expr::col(Alias::new("generation")).eq(row.try_get_by_index::<Vec<u8>>(0)?),
                    )
                    .add(Expr::col(Alias::new("sequence")).eq(row.try_get_by_index::<i64>(1)?)),
            );
        }
        removed += tx
            .execute(
                &Query::delete()
                    .from_table(Alias::new("sync_journal"))
                    .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                    .cond_where(matches)
                    .to_owned(),
            )
            .await?
            .rows_affected();
    }
    Ok(PruneStep {
        removed,
        more: removed == max_rows,
    })
}
/// Reclaim retired candidates and interval versions below the oldest state pin.
pub async fn reclaim_rows_step_on(tx: &DatabaseTransaction, ns: Uuid, limit: u64) -> Result<u64> {
    lock_on(tx, ns).await?;
    ensure!(limit > 0 && limit <= 1024, "invalid row reclaim budget");
    let meta = tx
        .query_one(
            &Query::select()
                .column(Alias::new("state_seq"))
                .from(Alias::new("sync_meta"))
                .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                .to_owned(),
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing namespace"))?;
    let state: i64 = meta.try_get_by_index(0)?;
    let below = minimum_pins_on(tx, ns)
        .await?
        .map_or(state, |(pin, _)| std::cmp::min(pin, state));
    let mut removed = crate::causal_observed::reclaim_step_on(tx, ns, below, limit).await?;
    if removed == limit {
        return Ok(removed);
    }
    let dead = Condition::any()
        .add(Expr::col(Alias::new("live_until")).lte(below))
        .add(
            Condition::all()
                .add(Expr::col(Alias::new("live_from")).is_null())
                .add(Expr::col(Alias::new("retention_seq")).lte(below)),
        );
    let q = Query::select()
        .columns(["collection", "writer", "counter"].map(Alias::new))
        .from(Alias::new("sync_candidates"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .cond_where(dead)
        .limit(limit - removed)
        .to_owned();
    let rows = tx.query_all(&q).await?;
    for batch in rows.chunks(100) {
        let mut matches = Condition::any();
        for row in batch {
            matches = matches.add(
                Condition::all()
                    .add(
                        Expr::col(Alias::new("collection")).eq(row.try_get_by_index::<Vec<u8>>(0)?),
                    )
                    .add(Expr::col(Alias::new("writer")).eq(row.try_get_by_index::<Vec<u8>>(1)?))
                    .add(Expr::col(Alias::new("counter")).eq(row.try_get_by_index::<i64>(2)?)),
            );
        }
        removed += tx
            .execute(
                &Query::delete()
                    .from_table(Alias::new("sync_candidates"))
                    .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                    .cond_where(matches)
                    .to_owned(),
            )
            .await?
            .rows_affected();
    }
    Ok(removed)
}
