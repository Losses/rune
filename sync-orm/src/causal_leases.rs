//! Transaction-bound durable source/build pins for the causal storage layout.
//! These primitives do not implement snapshot construction or compaction.
use sea_orm::sea_query::Condition;
use sea_orm::sea_query::Order;
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, DbErr, ExprTrait, FromQueryResult,
    sea_query::{Alias, Expr, Func, Query},
};
use sync::protocol::SyncError;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, FromQueryResult)]
pub struct Lease {
    pub id: Vec<u8>,
    pub generation: Vec<u8>,
    pub head: i64,
    pub head_anchor: Vec<u8>,
    pub state_seq: i64,
    pub parent_lease: Option<Vec<u8>>,
    pub build_state: Option<Vec<u8>>,
    pub scope: Vec<u8>,
    pub expires_at_ms: i64,
    pub status: i64,
    pub retained_root: Option<Vec<u8>>,
    pub descriptor: Option<Vec<u8>>,
}
#[derive(Debug, FromQueryResult)]
struct Position {
    generation: Vec<u8>,
    head: i64,
    head_anchor: Vec<u8>,
    state_seq: i64,
    floor_state_seq: i64,
}
fn err(message: &str) -> DbErr {
    DbErr::Custom(message.to_owned())
}
fn namespace(id: Uuid) -> Vec<u8> {
    id.as_bytes().to_vec()
}

/// Acquire the namespace serialization point in the caller's transaction.
pub async fn lock_on(tx: &DatabaseTransaction, ns: Uuid) -> Result<(), DbErr> {
    let query = Query::update()
        .table(Alias::new("sync_meta"))
        .value(Alias::new("head"), Expr::col(Alias::new("head")))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
        .to_owned();
    if tx.execute(&query).await?.rows_affected() != 1 {
        return Err(err("missing causal namespace"));
    }
    Ok(())
}

/// Capture both history axes while creating a non-serviceable durable pin.
pub struct BuildLeaseRequest {
    pub namespace: Uuid,
    pub id: Uuid,
    pub scope: Vec<u8>,
    pub now_ms: i64,
    pub expires_at_ms: i64,
    pub max_active: u64,
    pub max_ttl_ms: i64,
}

pub async fn create_build_on(
    tx: &DatabaseTransaction,
    request: BuildLeaseRequest,
) -> Result<Lease, DbErr> {
    let BuildLeaseRequest {
        namespace: ns,
        id,
        scope,
        now_ms,
        expires_at_ms,
        max_active,
        max_ttl_ms,
    } = request;
    if ns.is_nil()
        || id.is_nil()
        || scope.is_empty()
        || max_active == 0
        || max_ttl_ms <= 0
        || expires_at_ms <= now_ms
        || expires_at_ms
            .checked_sub(now_ms)
            .is_none_or(|ttl| ttl > max_ttl_ms)
    {
        return Err(err("invalid lease admission"));
    }
    lock_on(tx, ns).await?;
    let count = Query::select()
        .expr(Func::count(Expr::val(1)))
        .from(Alias::new("sync_leases"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
        .and_where(Expr::col(Alias::new("status")).is_in([0, 1]))
        .and_where(Expr::col(Alias::new("expires_at_ms")).gt(now_ms))
        .to_owned();
    let count = tx
        .query_one(&count)
        .await?
        .ok_or_else(|| err("missing lease count"))?
        .try_get_by_index::<i64>(0)?;
    if u64::try_from(count).map_err(|_| err("invalid lease count"))? >= max_active {
        return Err(err("lease quota exceeded"));
    }
    let position = Query::select()
        .columns(
            [
                "generation",
                "head",
                "head_anchor",
                "state_seq",
                "floor_state_seq",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_meta"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
        .to_owned();
    let position = Position::find_by_statement(tx.get_database_backend().build(&position))
        .one(tx)
        .await?
        .ok_or_else(|| err("missing causal namespace"))?;
    if position.generation.len() != 16
        || position.head_anchor.len() != 32
        || position.head < 0
        || position.state_seq < 0
        || position.floor_state_seq < 0
        || position.floor_state_seq > position.state_seq
    {
        return Err(err("invalid stored publication position"));
    }
    let lease = Lease {
        id: id.as_bytes().to_vec(),
        generation: position.generation,
        head: position.head,
        head_anchor: position.head_anchor,
        state_seq: position.state_seq,
        parent_lease: None,
        build_state: None,
        scope,
        expires_at_ms,
        status: 0,
        retained_root: None,
        descriptor: None,
    };
    let insert = Query::insert()
        .into_table(Alias::new("sync_leases"))
        .columns(
            [
                "namespace",
                "id",
                "generation",
                "head",
                "head_anchor",
                "state_seq",
                "scope",
                "expires_at_ms",
                "status",
            ]
            .map(Alias::new),
        )
        .values_panic([
            namespace(ns).into(),
            lease.id.clone().into(),
            lease.generation.clone().into(),
            lease.head.into(),
            lease.head_anchor.clone().into(),
            lease.state_seq.into(),
            lease.scope.clone().into(),
            lease.expires_at_ms.into(),
            lease.status.into(),
        ])
        .to_owned();
    tx.execute(&insert).await?;
    Ok(lease)
}

pub(crate) async fn load_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    id: Uuid,
) -> Result<Option<Lease>, DbErr> {
    let query = Query::select()
        .columns(
            [
                "id",
                "generation",
                "head",
                "head_anchor",
                "state_seq",
                "parent_lease",
                "build_state",
                "scope",
                "expires_at_ms",
                "status",
                "retained_root",
                "descriptor",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_leases"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
        .and_where(Expr::col(Alias::new("id")).eq(id.as_bytes().to_vec()))
        .to_owned();
    Lease::find_by_statement(tx.get_database_backend().build(&query))
        .one(tx)
        .await
}

/// A ready view is serviceable only while its durable lease is unexpired.
pub async fn ready_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    id: Uuid,
    now_ms: i64,
) -> Result<Lease, DbErr> {
    let lease = load_on(tx, ns, id)
        .await?
        .ok_or_else(|| err("snapshot expired"))?;
    if lease.status != 1
        || lease.expires_at_ms <= now_ms
        || lease.descriptor.is_none()
        || lease.retained_root.is_none()
    {
        return Err(err("snapshot expired"));
    }
    Ok(lease)
}

/// Publish a validated manifest root while it is protected by the same lock.
/// The caller must have already validated the root's complete graph in this transaction.
pub async fn pin_root_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    id: Uuid,
    root: [u8; 32],
    now_ms: i64,
) -> Result<(), DbErr> {
    lock_on(tx, ns).await?;
    let lease = load_on(tx, ns, id)
        .await?
        .ok_or_else(|| err("snapshot expired"))?;
    if lease.status != 0 || lease.expires_at_ms <= now_ms {
        return Err(err("snapshot expired"));
    }
    let exists = Query::select()
        .column(Alias::new("kind"))
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
        .and_where(Expr::col(Alias::new("digest")).eq(root.to_vec()))
        .limit(1)
        .to_owned();
    let root_row = tx
        .query_one(&exists)
        .await?
        .ok_or_else(|| err("missing validated manifest root"))?;
    if !(0..=7).contains(&root_row.try_get_by_index::<i64>(0)?) {
        return Err(err("manifest root has an invalid or reclaiming kind"));
    }
    let update = Query::update()
        .table(Alias::new("sync_leases"))
        .value(Alias::new("retained_root"), root.to_vec())
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
        .and_where(Expr::col(Alias::new("id")).eq(id.as_bytes().to_vec()))
        .to_owned();
    tx.execute(&update).await?;
    Ok(())
}

/// Finish construction exactly once; a ready descriptor is immutable.
pub async fn publish_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    id: Uuid,
    descriptor: Vec<u8>,
    now_ms: i64,
) -> Result<(), DbErr> {
    if descriptor.is_empty() {
        return Err(err("empty snapshot descriptor"));
    }
    lock_on(tx, ns).await?;
    let lease = load_on(tx, ns, id)
        .await?
        .ok_or_else(|| err("snapshot expired"))?;
    if lease.status != 0 || lease.expires_at_ms <= now_ms || lease.retained_root.is_none() {
        return Err(err("snapshot expired or unpinned"));
    }
    let update = Query::update()
        .table(Alias::new("sync_leases"))
        .value(Alias::new("status"), 1)
        .value(Alias::new("descriptor"), descriptor)
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
        .and_where(Expr::col(Alias::new("id")).eq(id.as_bytes().to_vec()))
        .to_owned();
    tx.execute(&update).await?;
    Ok(())
}

/// Revoke before clearing the root so later reclamation cannot race the lease.
pub async fn revoke_on(tx: &DatabaseTransaction, ns: Uuid, id: Uuid) -> Result<(), DbErr> {
    lock_on(tx, ns).await?;
    let update = Query::update()
        .table(Alias::new("sync_leases"))
        .value(Alias::new("status"), 2)
        .value(Alias::new("retained_root"), Option::<Vec<u8>>::None)
        .value(Alias::new("build_state"), Option::<Vec<u8>>::None)
        .value(Alias::new("descriptor"), Option::<Vec<u8>>::None)
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
        .cond_where(
            Condition::any()
                .add(Expr::col(Alias::new("id")).eq(id.as_bytes().to_vec()))
                .add(Expr::col(Alias::new("parent_lease")).eq(id.as_bytes().to_vec())),
        )
        .to_owned();
    if tx.execute(&update).await?.rows_affected() == 0 {
        return Err(err("snapshot expired"));
    }
    Ok(())
}

/// Reserve one bounded source-work slot. Completed child results can be
/// recomputed from their pinned parent and are evicted before active work.
pub(crate) async fn reserve_child_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    parent: Uuid,
    now_ms: i64,
    max_evictions: u64,
) -> anyhow::Result<()> {
    lock_on(tx, ns).await?;
    let source = ready_on(tx, ns, parent, now_ms).await?;
    if source.parent_lease.is_some() {
        return Err(err("work parent is not a source view").into());
    }
    let q = Query::select()
        .columns(["id", "status"].map(Alias::new))
        .from(Alias::new("sync_leases"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
        .and_where(Expr::col(Alias::new("status")).ne(2))
        .and_where(Expr::col(Alias::new("expires_at_ms")).gt(now_ms))
        .order_by(Alias::new("status"), Order::Desc)
        .order_by(Alias::new("id"), Order::Asc)
        .limit(33)
        .to_owned();
    let rows = tx.query_all(&q).await?;
    let mut count = rows.len();
    let needed = count.saturating_sub(15);
    if needed as u64 > max_evictions {
        return Err(SyncError::LimitTooSmall {
            required: needed as u64 + 3,
        }
        .into());
    }
    let cached = rows
        .iter()
        .filter(|row| row.try_get_by_index::<i64>(1).ok() == Some(3))
        .count();
    if cached < needed {
        return Err(SyncError::StageQuota.into());
    }
    for row in &rows {
        if count < 16 {
            break;
        }
        if row.try_get_by_index::<i64>(1)? != 3 {
            break;
        }
        let id: Vec<u8> = row.try_get_by_index(0)?;
        let q = Query::delete()
            .from_table(Alias::new("sync_leases"))
            .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
            .and_where(Expr::col(Alias::new("id")).eq(id))
            .and_where(Expr::col(Alias::new("status")).eq(3))
            .to_owned();
        tx.execute(&q).await?;
        count -= 1;
    }
    if count >= 16 {
        return Err(SyncError::StageQuota.into());
    }
    Ok(())
}

pub(crate) async fn complete_child_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    id: Uuid,
) -> Result<(), DbErr> {
    let q = Query::update()
        .table(Alias::new("sync_leases"))
        .value(Alias::new("status"), 3)
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
        .and_where(Expr::col(Alias::new("id")).eq(id.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("status")).eq(0))
        .and_where(Expr::col(Alias::new("parent_lease")).is_not_null())
        .to_owned();
    if tx.execute(&q).await?.rows_affected() != 1 {
        return Err(err("missing source work"));
    }
    Ok(())
}

/// Conservative minimum positions protected by every non-revoked lease.
/// Expired leases remain pins until `revoke_expired_on` commits their revocation.
pub async fn minimum_pins_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
) -> Result<Option<(i64, i64)>, DbErr> {
    minimum_pins_except_on(tx, ns, None).await
}
pub(crate) async fn minimum_pins_except_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    excluded: Option<Uuid>,
) -> Result<Option<(i64, i64)>, DbErr> {
    lock_on(tx, ns).await?;
    let mut q = Query::select()
        .expr(Func::min(Expr::col(Alias::new("state_seq"))))
        .expr(Func::min(Expr::col(Alias::new("head"))))
        .from(Alias::new("sync_leases"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
        .and_where(Expr::col(Alias::new("status")).ne(2))
        .to_owned();
    if let Some(id) = excluded {
        q.and_where(Expr::col(Alias::new("id")).ne(id.as_bytes().to_vec()));
    }
    let row = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| err("missing lease aggregate"))?;
    let state: Option<i64> = row.try_get_by_index(0)?;
    let head: Option<i64> = row.try_get_by_index(1)?;
    match (state, head) {
        (None, None) => Ok(None),
        (Some(s), Some(h)) if s >= 0 && h >= 0 => Ok(Some((s, h))),
        _ => Err(err("invalid lease positions")),
    }
}

/// Durably revoke at most `limit` expired leases before releasing their roots.
pub async fn revoke_expired_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    now_ms: i64,
    limit: u64,
) -> Result<u64, DbErr> {
    if limit == 0 || limit > 1024 {
        return Err(err("invalid lease revocation limit"));
    }
    lock_on(tx, ns).await?;
    let select = Query::select()
        .column(Alias::new("id"))
        .from(Alias::new("sync_leases"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
        .and_where(Expr::col(Alias::new("status")).ne(2))
        .and_where(Expr::col(Alias::new("expires_at_ms")).lte(now_ms))
        .order_by(Alias::new("expires_at_ms"), Order::Asc)
        .order_by(Alias::new("id"), Order::Asc)
        .limit(limit)
        .to_owned();
    let rows = tx.query_all(&select).await?;
    for row in &rows {
        let id: Vec<u8> = row.try_get_by_index(0)?;
        if id.len() != 16 {
            return Err(err("invalid lease UUID"));
        }
        let update = Query::update()
            .table(Alias::new("sync_leases"))
            .value(Alias::new("status"), 2)
            .value(Alias::new("retained_root"), Option::<Vec<u8>>::None)
            .value(Alias::new("build_state"), Option::<Vec<u8>>::None)
            .value(Alias::new("descriptor"), Option::<Vec<u8>>::None)
            .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
            .and_where(Expr::col(Alias::new("id")).eq(id))
            .to_owned();
        if tx.execute(&update).await?.rows_affected() != 1 {
            return Err(err("missing lease"));
        }
    }
    Ok(rows.len() as u64)
}

/// Atomically replace an opaque, versioned source-work checkpoint under a build pin.
/// A caller must pin all immutable output manifests in the same transaction.
pub async fn checkpoint_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    id: Uuid,
    expected: Option<&[u8]>,
    next: Vec<u8>,
    now_ms: i64,
) -> Result<(), DbErr> {
    checkpoint_bounded_on(tx, ns, id, expected, next, now_ms, 64 * 1024).await
}

/// Version 2 builds retain a context accumulator within their admitted byte budget.
pub(crate) async fn checkpoint_bounded_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    id: Uuid,
    expected: Option<&[u8]>,
    next: Vec<u8>,
    now_ms: i64,
    max_bytes: u64,
) -> Result<(), DbErr> {
    if max_bytes == 0
        || max_bytes > sync::protocol::MAX_LOGICAL_BYTES
        || next.is_empty()
        || next.len() as u64 > max_bytes
    {
        return Err(err("invalid source work checkpoint size"));
    }
    lock_on(tx, ns).await?;
    let lease = load_on(tx, ns, id)
        .await?
        .ok_or_else(|| err("snapshot expired"))?;
    if lease.status != 0 || lease.expires_at_ms <= now_ms {
        return Err(err("snapshot expired"));
    }
    if lease.build_state.as_deref() != expected {
        return Err(err("stale source work checkpoint"));
    }
    let q = Query::update()
        .table(Alias::new("sync_leases"))
        .value(Alias::new("build_state"), next)
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace(ns)))
        .and_where(Expr::col(Alias::new("id")).eq(id.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("status")).eq(0))
        .to_owned();
    if tx.execute(&q).await?.rows_affected() != 1 {
        return Err(err("snapshot expired"));
    }
    Ok(())
}

/// Read and validate an active build while holding the namespace serialization lock.
pub async fn building_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    id: Uuid,
    now_ms: i64,
) -> Result<Lease, DbErr> {
    lock_on(tx, ns).await?;
    let lease = load_on(tx, ns, id)
        .await?
        .ok_or_else(|| err("snapshot expired"))?;
    if lease.status != 0 || lease.expires_at_ms <= now_ms || lease.state_seq < 0 {
        return Err(err("invalid or expired build lease"));
    }
    Ok(lease)
}

/// Acquire SQLite's writer reservation when a namespace row does not yet exist.
pub(crate) async fn begin_write(
    db: &sea_orm::DatabaseConnection,
) -> Result<DatabaseTransaction, DbErr> {
    use sea_orm::{SqliteTransactionMode, TransactionOptions, TransactionTrait};
    db.begin_with_options(TransactionOptions {
        sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
        ..TransactionOptions::default()
    })
    .await
}
