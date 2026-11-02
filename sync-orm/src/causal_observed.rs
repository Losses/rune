//! Versioned, canonical collection-wide observed intervals.
use anyhow::{Result, ensure};
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, ExprTrait,
    sea_query::{Alias, Condition, Expr, Order, Query},
};
use sync::causal::{Context, Dot};
use sync::protocol::SyncError;
use uuid::Uuid;

fn current(ns: Uuid, tag: &[u8]) -> Condition {
    Condition::all()
        .add(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .add(Expr::col(Alias::new("collection")).eq(tag.to_vec()))
        .add(Expr::col(Alias::new("removed_seq")).is_null())
}

/// Test a dot against the current collection context.
pub async fn contains_on(tx: &DatabaseTransaction, ns: Uuid, tag: &[u8], dot: Dot) -> Result<bool> {
    let q = Query::select()
        .column(Alias::new("last"))
        .from(Alias::new("sync_observed"))
        .cond_where(current(ns, tag))
        .and_where(Expr::col(Alias::new("writer")).eq(dot.writer.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("first")).lte(i64::try_from(dot.counter)?))
        .order_by(Alias::new("first"), Order::Desc)
        .limit(1)
        .to_owned();
    Ok(tx
        .query_one(&q)
        .await?
        .map(|r| r.try_get_by_index::<i64>(0))
        .transpose()?
        .is_some_and(|last| last >= dot.counter as i64))
}

/// Union intervals, retiring old versions and charging every changed row.
/// The caller holds the namespace writer lock and rolls back on error.
pub async fn insert_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    tag: &[u8],
    state: i64,
    context: &Context,
    budget: &mut u64,
) -> Result<bool> {
    ensure!(state >= 0, "invalid observed state");
    let mut changed = false;
    for (writer, first, last) in context.intervals() {
        let q = Query::select()
            .columns(["first", "last", "added_seq"].map(Alias::new))
            .from(Alias::new("sync_observed"))
            .cond_where(current(ns, tag))
            .and_where(Expr::col(Alias::new("writer")).eq(writer.as_bytes().to_vec()))
            .and_where(
                Expr::col(Alias::new("first")).lte(i64::try_from(std::cmp::min(
                    last.saturating_add(1),
                    sync::causal::MAX_DOT_COUNTER,
                ))?),
            )
            .and_where(Expr::col(Alias::new("last")).gte(i64::try_from(first.saturating_sub(1))?))
            .order_by(Alias::new("first"), Order::Desc)
            .limit(std::cmp::min(*budget, sync::protocol::MAX_MUTATIONS).saturating_add(1))
            .to_owned();
        let mut overlapping = Vec::new();
        let mut a = first;
        let mut b = last;
        for row in tx.query_all(&q).await? {
            let left = u64::try_from(row.try_get_by_index::<i64>(0)?)?;
            let right = u64::try_from(row.try_get_by_index::<i64>(1)?)?;
            let added: i64 = row.try_get_by_index(2)?;
            ensure!(
                left > 0 && right >= left && added <= state,
                "invalid observed interval"
            );
            if right < first.saturating_sub(1) {
                break;
            }
            a = std::cmp::min(a, left);
            b = std::cmp::max(b, right);
            overlapping.push((left, right, added));
        }
        if overlapping.len() == 1 && overlapping[0].0 <= first && overlapping[0].1 >= last {
            continue;
        }
        let charge = overlapping.len() as u64 + 1;
        *budget = budget.checked_sub(charge).ok_or(SyncError::WorkLimit)?;
        for (left, _, added) in overlapping {
            let key = Condition::all()
                .add(current(ns, tag))
                .add(Expr::col(Alias::new("writer")).eq(writer.as_bytes().to_vec()))
                .add(Expr::col(Alias::new("first")).eq(i64::try_from(left)?))
                .add(Expr::col(Alias::new("added_seq")).eq(added));
            if added == state {
                tx.execute(
                    &Query::delete()
                        .from_table(Alias::new("sync_observed"))
                        .cond_where(key)
                        .to_owned(),
                )
                .await?;
            } else {
                tx.execute(
                    &Query::update()
                        .table(Alias::new("sync_observed"))
                        .value(Alias::new("removed_seq"), state)
                        .cond_where(key)
                        .to_owned(),
                )
                .await?;
            }
        }
        tx.execute(
            &Query::insert()
                .into_table(Alias::new("sync_observed"))
                .columns(
                    [
                        "namespace",
                        "collection",
                        "writer",
                        "first",
                        "last",
                        "added_seq",
                    ]
                    .map(Alias::new),
                )
                .values_panic([
                    ns.as_bytes().to_vec().into(),
                    tag.to_vec().into(),
                    writer.as_bytes().to_vec().into(),
                    i64::try_from(a)?.into(),
                    i64::try_from(b)?.into(),
                    state.into(),
                ])
                .to_owned(),
        )
        .await?;
        changed = true;
    }
    Ok(changed)
}

/// Read visible intervals in exclusive writer/first order at a pinned state.
pub async fn page_at_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    tag: &[u8],
    state: i64,
    after: Option<(Uuid, u64)>,
    limit: u64,
) -> Result<Vec<(Uuid, u64, u64)>> {
    ensure!(state >= 0 && limit > 0, "invalid observed page");
    let mut q = Query::select();
    q.columns(["writer", "first", "last"].map(Alias::new))
        .from(Alias::new("sync_observed"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("collection")).eq(tag.to_vec()))
        .and_where(Expr::col(Alias::new("added_seq")).lte(state))
        .cond_where(
            Condition::any()
                .add(Expr::col(Alias::new("removed_seq")).is_null())
                .add(Expr::col(Alias::new("removed_seq")).gt(state)),
        );
    if let Some((writer, first)) = after {
        q.cond_where(
            Condition::any()
                .add(Expr::col(Alias::new("writer")).gt(writer.as_bytes().to_vec()))
                .add(
                    Condition::all()
                        .add(Expr::col(Alias::new("writer")).eq(writer.as_bytes().to_vec()))
                        .add(Expr::col(Alias::new("first")).gt(i64::try_from(first)?)),
                ),
        );
    }
    q.order_by(Alias::new("writer"), Order::Asc)
        .order_by(Alias::new("first"), Order::Asc)
        .limit(limit);
    tx.query_all(&q.to_owned())
        .await?
        .into_iter()
        .map(|row| {
            Ok((
                Uuid::from_slice(&row.try_get_by_index::<Vec<u8>>(0)?)?,
                u64::try_from(row.try_get_by_index::<i64>(1)?)?,
                u64::try_from(row.try_get_by_index::<i64>(2)?)?,
            ))
        })
        .collect()
}

/// Return current context gaps within a single inclusive interval.
pub async fn unobserved_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    tag: &[u8],
    writer: Uuid,
    a: u64,
    b: u64,
) -> Result<Vec<(u64, u64)>> {
    ensure!(a > 0 && a <= b, "invalid observed bounds");
    let q = Query::select()
        .columns(["first", "last"].map(Alias::new))
        .from(Alias::new("sync_observed"))
        .cond_where(current(ns, tag))
        .and_where(Expr::col(Alias::new("writer")).eq(writer.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("first")).lte(i64::try_from(b)?))
        .and_where(Expr::col(Alias::new("last")).gte(i64::try_from(a)?))
        .order_by(Alias::new("first"), Order::Asc)
        .to_owned();
    let mut next = a;
    let mut gaps = Vec::new();
    for row in tx.query_all(&q).await? {
        let first = u64::try_from(row.try_get_by_index::<i64>(0)?)?;
        let last = u64::try_from(row.try_get_by_index::<i64>(1)?)?;
        if first > next {
            gaps.push((next, first - 1));
        }
        next = std::cmp::max(next, last.saturating_add(1));
        if next > b {
            break;
        }
    }
    if next <= b {
        gaps.push((next, b));
    }
    Ok(gaps)
}

/// Reclaim historical versions below all active state pins.
pub async fn reclaim_step_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    below_state: i64,
    limit: u64,
) -> Result<u64> {
    let q = Query::select()
        .columns(["collection", "writer", "first", "added_seq"].map(Alias::new))
        .from(Alias::new("sync_observed"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("removed_seq")).lte(below_state))
        .limit(limit)
        .to_owned();
    let mut removed = 0;
    for row in tx.query_all(&q).await? {
        removed += tx
            .execute(
                &Query::delete()
                    .from_table(Alias::new("sync_observed"))
                    .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                    .and_where(
                        Expr::col(Alias::new("collection")).eq(row.try_get_by_index::<Vec<u8>>(0)?),
                    )
                    .and_where(
                        Expr::col(Alias::new("writer")).eq(row.try_get_by_index::<Vec<u8>>(1)?),
                    )
                    .and_where(Expr::col(Alias::new("first")).eq(row.try_get_by_index::<i64>(2)?))
                    .and_where(
                        Expr::col(Alias::new("added_seq")).eq(row.try_get_by_index::<i64>(3)?),
                    )
                    .to_owned(),
            )
            .await?
            .rows_affected();
    }
    Ok(removed)
}
