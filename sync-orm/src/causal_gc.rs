//! Bounded single-manifest DAG reclamation primitives.
//! Floor construction and publication must be implemented before invoking GC.
use crate::causal_leases::lock_on;
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, DbErr, ExprTrait,
    sea_query::{Alias, Expr, Func, Query},
};
use uuid::Uuid;
fn err(s: &str) -> DbErr {
    DbErr::Custom(s.to_owned())
}

async fn references(
    tx: &DatabaseTransaction,
    namespace: &[u8],
    table: &str,
    column: &str,
    digest: &[u8],
) -> Result<bool, DbErr> {
    let mut q = Query::select();
    q.expr(Expr::val(1))
        .from(Alias::new(table))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.to_vec()))
        .and_where(Expr::col(Alias::new(column)).eq(digest.to_vec()));
    // Unrevoked leases keep their root even after wall-clock expiry until
    // expiry is durably recorded under the namespace lock.
    if table == "sync_leases" {
        q.and_where(Expr::col(Alias::new("status")).ne(2));
    }
    Ok(tx.query_one(&q.limit(1).to_owned()).await?.is_some())
}

/// Remove one unrooted manifest and its owned rows in the caller transaction.
/// Returns false for a live or missing manifest. Exceeding the supplied work
/// budget is an error and removes nothing; graph traversal is left to the caller.
pub async fn collect_one_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    digest: [u8; 32],
    max_rows: u64,
    max_bytes: u64,
) -> Result<bool, DbErr> {
    if namespace.is_nil() || max_rows == 0 {
        return Err(err("invalid reclamation budget"));
    }
    lock_on(tx, namespace).await?;
    let ns = namespace.as_bytes().to_vec();
    let value = digest.to_vec();
    for (table, column) in [
        ("sync_meta", "baseline_manifest"),
        ("sync_journal", "event_manifest"),
        ("sync_context_ranges", "context_manifest"),
        ("sync_leases", "retained_root"),
        ("sync_manifest_edges", "child_manifest"),
    ] {
        if references(tx, &ns, table, column, &value).await? {
            return Ok(false);
        }
    }
    let find = Query::select()
        .column(Alias::new("encoded_bytes"))
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
        .and_where(Expr::col(Alias::new("digest")).eq(value.clone()))
        .to_owned();
    let Some(manifest) = tx.query_one(&find).await? else {
        return Ok(false);
    };
    let encoded = manifest.try_get_by_index::<i64>(0)?;
    if encoded < 0 || u64::try_from(encoded).map_err(|_| err("invalid manifest size"))? > max_bytes
    {
        return Err(err("reclamation byte budget exceeded"));
    }
    let mut total = 1_u64;
    for (table, column) in [
        ("sync_chunks", "manifest"),
        ("sync_context_intervals", "context_manifest"),
        ("sync_manifest_edges", "parent_manifest"),
    ] {
        let count = Query::select()
            .expr(Func::count(Expr::val(1)))
            .from(Alias::new(table))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
            .and_where(Expr::col(Alias::new(column)).eq(value.clone()))
            .to_owned();
        let count = tx
            .query_one(&count)
            .await?
            .ok_or_else(|| err("missing count"))?
            .try_get_by_index::<i64>(0)?;
        total = total
            .checked_add(u64::try_from(count).map_err(|_| err("invalid row count"))?)
            .ok_or_else(|| err("reclamation row count overflow"))?;
    }
    if total > max_rows {
        return Err(err("reclamation row budget exceeded"));
    }
    for (table, column) in [
        ("sync_chunks", "manifest"),
        ("sync_context_intervals", "context_manifest"),
        ("sync_manifest_edges", "parent_manifest"),
        ("sync_manifests", "digest"),
    ] {
        let delete = Query::delete()
            .from_table(Alias::new(table))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
            .and_where(Expr::col(Alias::new(column)).eq(value.clone()))
            .to_owned();
        tx.execute(&delete).await?;
    }
    Ok(true)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcStep {
    /// Exclusive continuation in the current pass; restart from None after a
    /// completed pass that reclaimed anything, as children may sort earlier.
    pub next: Option<[u8; 32]>,
    /// True when a manifest was marked, partially cleaned, or deleted.
    pub reclaimed: bool,
    pub done: bool,
}
/// Examine at most `scan_limit` manifests and advance reclamation of at most
/// one. A partially reclaimed digest is revisited on the next call.
pub async fn step_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    after: Option<[u8; 32]>,
    scan_limit: u64,
    max_rows: u64,
    max_bytes: u64,
) -> Result<GcStep, DbErr> {
    if namespace.is_nil() || scan_limit == 0 || scan_limit > 1024 {
        return Err(err("invalid GC scan limit"));
    }
    lock_on(tx, namespace).await?;
    let mut q = Query::select();
    q.column(Alias::new("digest"))
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()));
    if let Some(after) = after {
        q.and_where(Expr::col(Alias::new("digest")).gt(after.to_vec()));
    }
    q.order_by(Alias::new("digest"), sea_orm::sea_query::Order::Asc)
        .limit(scan_limit + 1);
    let rows = tx.query_all(&q.to_owned()).await?;
    let mut last = after;
    for (index, row) in rows.iter().enumerate() {
        if index >= scan_limit as usize {
            return Ok(GcStep {
                next: last,
                reclaimed: false,
                done: false,
            });
        }
        let bytes: Vec<u8> = row.try_get_by_index(0)?;
        let digest: [u8; 32] = bytes
            .try_into()
            .map_err(|_| err("invalid retained manifest digest"))?;
        let previous = last;
        last = Some(digest);
        match collect_incremental_on(tx, namespace, digest, max_rows, max_bytes).await? {
            IncrementalReclaim::LiveOrMissing => {}
            IncrementalReclaim::More => {
                return Ok(GcStep {
                    next: previous,
                    reclaimed: true,
                    done: false,
                });
            }
            IncrementalReclaim::Deleted => {
                return Ok(GcStep {
                    next: last,
                    reclaimed: true,
                    done: false,
                });
            }
        }
    }
    Ok(GcStep {
        next: last,
        reclaimed: false,
        done: true,
    })
}

/// A stable position in the bounded scan of obsolete baseline rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BasisCursor {
    pub generation: Vec<u8>,
    pub collection: Vec<u8>,
    pub state_seq: i64,
    pub ordinal: i64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BasisReclaimStep {
    pub removed: u64,
    pub next: Option<BasisCursor>,
}
/// Reclaim at most `limit` obsolete baseline rows. Every unrevoked lease pins
/// both its captured input generation and any output generation it is building,
/// even after expiry until explicit revocation. A completed scan may be restarted
/// later to revisit generations that were pinned during this pass.
pub async fn reclaim_basis_step_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    after: Option<BasisCursor>,
    limit: u64,
) -> anyhow::Result<BasisReclaimStep> {
    use sea_orm::sea_query::{Condition, Order};
    anyhow::ensure!(
        !namespace.is_nil() && limit > 0 && limit <= 1024,
        "invalid baseline reclaim budget"
    );
    crate::causal_leases::lock_on(tx, namespace).await?;
    let meta = Query::select()
        .column(Alias::new("baseline_generation"))
        .from(Alias::new("sync_meta"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .to_owned();
    let active: Vec<u8> = tx
        .query_one(&meta)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing causal namespace"))?
        .try_get_by_index(0)?;
    anyhow::ensure!(active.len() == 16, "invalid active baseline generation");
    let mut q = Query::select();
    q.columns(["basis_generation", "collection", "state_seq", "ordinal"].map(Alias::new))
        .from(Alias::new("sync_context_ranges"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("basis_generation")).ne(Uuid::nil().as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("basis_generation")).ne(active));
    if let Some(cursor) = after {
        anyhow::ensure!(
            cursor.generation.len() == 16
                && !cursor.collection.is_empty()
                && cursor.state_seq >= 0
                && cursor.ordinal >= 0,
            "invalid baseline reclaim cursor"
        );
        let generation = cursor.generation;
        let tag = cursor.collection;
        q.cond_where(
            Condition::any()
                .add(Expr::col(Alias::new("basis_generation")).gt(generation.clone()))
                .add(
                    Condition::all()
                        .add(Expr::col(Alias::new("basis_generation")).eq(generation.clone()))
                        .add(Expr::col(Alias::new("collection")).gt(tag.clone())),
                )
                .add(
                    Condition::all()
                        .add(Expr::col(Alias::new("basis_generation")).eq(generation.clone()))
                        .add(Expr::col(Alias::new("collection")).eq(tag.clone()))
                        .add(Expr::col(Alias::new("state_seq")).gt(cursor.state_seq)),
                )
                .add(
                    Condition::all()
                        .add(Expr::col(Alias::new("basis_generation")).eq(generation))
                        .add(Expr::col(Alias::new("collection")).eq(tag))
                        .add(Expr::col(Alias::new("state_seq")).eq(cursor.state_seq))
                        .add(Expr::col(Alias::new("ordinal")).gt(cursor.ordinal)),
                ),
        );
    }
    q.order_by(Alias::new("basis_generation"), Order::Asc)
        .order_by(Alias::new("collection"), Order::Asc)
        .order_by(Alias::new("state_seq"), Order::Asc)
        .order_by(Alias::new("ordinal"), Order::Asc)
        .limit(limit + 1);
    let rows = tx.query_all(&q.to_owned()).await?;
    let more = rows.len() as u64 > limit;
    let mut removed = 0;
    let mut last = None;
    for row in rows.into_iter().take(limit as usize) {
        let cursor = BasisCursor {
            generation: row.try_get_by_index(0)?,
            collection: row.try_get_by_index(1)?,
            state_seq: row.try_get_by_index(2)?,
            ordinal: row.try_get_by_index(3)?,
        };
        anyhow::ensure!(
            cursor.generation.len() == 16
                && !cursor.collection.is_empty()
                && cursor.state_seq >= 0
                && cursor.ordinal >= 0,
            "invalid retained baseline row"
        );
        let pin = Query::select()
            .expr(Expr::val(1))
            .from(Alias::new("sync_leases"))
            .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("status")).ne(2))
            .cond_where(
                Condition::any()
                    .add(Expr::col(Alias::new("basis_generation")).eq(cursor.generation.clone()))
                    .add(Expr::col(Alias::new("output_generation")).eq(cursor.generation.clone())),
            )
            .limit(1)
            .to_owned();
        if tx.query_one(&pin).await?.is_none() {
            let delete = Query::delete()
                .from_table(Alias::new("sync_context_ranges"))
                .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
                .and_where(Expr::col(Alias::new("collection")).eq(cursor.collection.clone()))
                .and_where(Expr::col(Alias::new("basis_generation")).eq(cursor.generation.clone()))
                .and_where(Expr::col(Alias::new("state_seq")).eq(cursor.state_seq))
                .and_where(Expr::col(Alias::new("ordinal")).eq(cursor.ordinal))
                .to_owned();
            removed += tx.execute(&delete).await?.rows_affected();
        }
        last = Some(cursor);
    }
    Ok(BasisReclaimStep {
        removed,
        next: if more { last } else { None },
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IncrementalReclaim {
    LiveOrMissing,
    More,
    Deleted,
}
/// Claim an unreferenced manifest and remove at most `max_rows` owned rows.
/// Kind 8 is an internal reclaiming marker: readers and all link/publication
/// paths must reject it. The namespace lock makes claim-versus-link atomic.
/// A crash leaves the marker durable and a later call resumes cleanup.
pub async fn collect_incremental_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    digest: [u8; 32],
    max_rows: u64,
    max_bytes: u64,
) -> Result<IncrementalReclaim, DbErr> {
    if namespace.is_nil() || max_rows == 0 || max_rows > 1024 || max_bytes == 0 {
        return Err(err("invalid incremental reclamation budget"));
    }
    lock_on(tx, namespace).await?;
    let ns = namespace.as_bytes().to_vec();
    let value = digest.to_vec();
    let q = Query::select()
        .column(Alias::new("kind"))
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
        .and_where(Expr::col(Alias::new("digest")).eq(value.clone()))
        .to_owned();
    let Some(header) = tx.query_one(&q).await? else {
        return Ok(IncrementalReclaim::LiveOrMissing);
    };
    let kind: i64 = header.try_get_by_index(0)?;
    if !(0..=8).contains(&kind) {
        return Err(err("invalid manifest kind"));
    }
    for (table, column) in [
        ("sync_meta", "baseline_manifest"),
        ("sync_journal", "event_manifest"),
        ("sync_context_ranges", "context_manifest"),
        ("sync_leases", "retained_root"),
        ("sync_manifest_edges", "child_manifest"),
    ] {
        if references(tx, &ns, table, column, &value).await? {
            if kind == 8 {
                return Err(err("reclaiming manifest acquired a reference"));
            }
            return Ok(IncrementalReclaim::LiveOrMissing);
        }
    }
    let mut remaining = max_rows;
    let mut used_bytes = 0_u64;
    if kind != 8 {
        if max_bytes < 8 {
            return Err(err("reclamation byte budget too small"));
        }
        let q = Query::update()
            .table(Alias::new("sync_manifests"))
            .value(Alias::new("kind"), 8_i64)
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
            .and_where(Expr::col(Alias::new("digest")).eq(value.clone()))
            .and_where(Expr::col(Alias::new("kind")).eq(kind))
            .to_owned();
        if tx.execute(&q).await?.rows_affected() != 1 {
            return Err(err("manifest claim changed"));
        }
        remaining -= 1;
        used_bytes += 8;
        if remaining == 0 {
            return Ok(IncrementalReclaim::More);
        }
    }
    loop {
        let q = Query::select()
            .column(Alias::new("ordinal"))
            .expr_as(
                Func::cust(Alias::new("length")).arg(Expr::col(Alias::new("bytes"))),
                Alias::new("size"),
            )
            .from(Alias::new("sync_chunks"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
            .and_where(Expr::col(Alias::new("manifest")).eq(value.clone()))
            .limit(1)
            .to_owned();
        let Some(row) = tx.query_one(&q).await? else {
            break;
        };
        let ordinal: i64 = row.try_get_by_index(0)?;
        let size = u64::try_from(row.try_get_by_index::<i64>(1)?)
            .map_err(|_| err("invalid chunk size"))?
            .checked_add(96)
            .ok_or_else(|| err("reclamation byte overflow"))?;
        if used_bytes.checked_add(size).is_none_or(|n| n > max_bytes) {
            if remaining == max_rows {
                return Err(err("reclamation byte budget too small"));
            }
            return Ok(IncrementalReclaim::More);
        }
        let q = Query::delete()
            .from_table(Alias::new("sync_chunks"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
            .and_where(Expr::col(Alias::new("manifest")).eq(value.clone()))
            .and_where(Expr::col(Alias::new("ordinal")).eq(ordinal))
            .to_owned();
        if tx.execute(&q).await?.rows_affected() != 1 {
            return Err(err("reclaim chunk changed"));
        }
        used_bytes += size;
        remaining -= 1;
        if remaining == 0 {
            return Ok(IncrementalReclaim::More);
        }
    }
    loop {
        let q = Query::select()
            .columns(["writer", "first"].map(Alias::new))
            .from(Alias::new("sync_context_intervals"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
            .and_where(Expr::col(Alias::new("context_manifest")).eq(value.clone()))
            .limit(1)
            .to_owned();
        let Some(row) = tx.query_one(&q).await? else {
            break;
        };
        if used_bytes.checked_add(80).is_none_or(|n| n > max_bytes) {
            if remaining == max_rows {
                return Err(err("reclamation byte budget too small"));
            }
            return Ok(IncrementalReclaim::More);
        }
        let writer: Vec<u8> = row.try_get_by_index(0)?;
        let first: i64 = row.try_get_by_index(1)?;
        let q = Query::delete()
            .from_table(Alias::new("sync_context_intervals"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
            .and_where(Expr::col(Alias::new("context_manifest")).eq(value.clone()))
            .and_where(Expr::col(Alias::new("writer")).eq(writer))
            .and_where(Expr::col(Alias::new("first")).eq(first))
            .to_owned();
        if tx.execute(&q).await?.rows_affected() != 1 {
            return Err(err("reclaim interval changed"));
        }
        used_bytes += 80;
        remaining -= 1;
        if remaining == 0 {
            return Ok(IncrementalReclaim::More);
        }
    }
    loop {
        let q = Query::select()
            .column(Alias::new("child_manifest"))
            .from(Alias::new("sync_manifest_edges"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
            .and_where(Expr::col(Alias::new("parent_manifest")).eq(value.clone()))
            .limit(1)
            .to_owned();
        let Some(row) = tx.query_one(&q).await? else {
            break;
        };
        if used_bytes.checked_add(80).is_none_or(|n| n > max_bytes) {
            if remaining == max_rows {
                return Err(err("reclamation byte budget too small"));
            }
            return Ok(IncrementalReclaim::More);
        }
        let child: Vec<u8> = row.try_get_by_index(0)?;
        let q = Query::delete()
            .from_table(Alias::new("sync_manifest_edges"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
            .and_where(Expr::col(Alias::new("parent_manifest")).eq(value.clone()))
            .and_where(Expr::col(Alias::new("child_manifest")).eq(child))
            .to_owned();
        if tx.execute(&q).await?.rows_affected() != 1 {
            return Err(err("reclaim edge changed"));
        }
        used_bytes += 80;
        remaining -= 1;
        if remaining == 0 {
            return Ok(IncrementalReclaim::More);
        }
    }
    let q = Query::select()
        .expr_as(
            Func::cust(Alias::new("length")).arg(Expr::col(Alias::new("metadata"))),
            Alias::new("size"),
        )
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
        .and_where(Expr::col(Alias::new("digest")).eq(value.clone()))
        .to_owned();
    let size = u64::try_from(
        tx.query_one(&q)
            .await?
            .ok_or_else(|| err("missing reclaiming header"))?
            .try_get_by_index::<i64>(0)?,
    )
    .map_err(|_| err("invalid manifest metadata size"))?
    .checked_add(112)
    .ok_or_else(|| err("reclamation byte overflow"))?;
    if used_bytes.checked_add(size).is_none_or(|n| n > max_bytes) {
        if remaining == max_rows {
            return Err(err("reclamation byte budget too small"));
        }
        return Ok(IncrementalReclaim::More);
    }
    let q = Query::delete()
        .from_table(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns))
        .and_where(Expr::col(Alias::new("digest")).eq(value))
        .and_where(Expr::col(Alias::new("kind")).eq(8_i64))
        .to_owned();
    if tx.execute(&q).await?.rows_affected() != 1 {
        return Err(err("reclaim header changed"));
    }
    Ok(IncrementalReclaim::Deleted)
}
