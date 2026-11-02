//! Transaction-bound publication continuity and directional completion.
use anyhow::{Result, ensure};
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, ExprTrait,
    sea_query::{Alias, Condition, Expr, OnConflict, Query},
};
use sync::protocol::{self as p, CheckpointKey, HistoryStatus, ReceiveProgress, SyncModel};
use uuid::Uuid;

fn checkpoint_filter(key: &CheckpointKey) -> Condition {
    Condition::all()
        .add(Expr::col(Alias::new("namespace")).eq(key.namespace.as_bytes().to_vec()))
        .add(Expr::col(Alias::new("source_replica")).eq(key.source_replica.as_bytes().to_vec()))
        .add(
            Expr::col(Alias::new("source_generation"))
                .eq(key.source_generation.as_bytes().to_vec()),
        )
        .add(Expr::col(Alias::new("scope_digest")).eq(key.scope_id.to_vec()))
}
fn valid_key(key: &CheckpointKey) -> Result<()> {
    ensure!(
        !key.namespace.is_nil() && !key.source_replica.is_nil() && !key.source_generation.is_nil(),
        p::SyncError::Invalid("nil checkpoint identity".into())
    );
    Ok(())
}
fn progress(sequence: i64, anchor: Vec<u8>) -> Result<ReceiveProgress> {
    Ok(ReceiveProgress {
        sequence: u64::try_from(sequence)?,
        anchor: anchor
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid stored continuity anchor"))?,
    })
}

pub async fn head_on<C: ConnectionTrait>(db: &C, namespace: Uuid) -> Result<ReceiveProgress> {
    let query = Query::select()
        .columns(["head", "head_anchor"].map(Alias::new))
        .from(Alias::new("sync_meta"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .to_owned();
    let row = db
        .query_one(&query)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing namespace"))?;
    progress(row.try_get_by_index(0)?, row.try_get_by_index(1)?)
}

/// Caller-owned initialization is a namespace lifecycle operation, not an
/// inferred per-object publication state.
pub async fn initialization_complete_on<C: ConnectionTrait>(
    db: &C,
    namespace: Uuid,
) -> Result<bool> {
    let query = Query::select()
        .column(Alias::new("initialized_at"))
        .from(Alias::new("sync_meta"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .to_owned();
    let row = db
        .query_one(&query)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing namespace"))?;
    Ok(row.try_get_by_index::<Option<String>>(0)?.is_some())
}

/// Commit with the caller's final checked initialization batch.
pub async fn complete_initialization_on(tx: &DatabaseTransaction, namespace: Uuid) -> Result<()> {
    crate::causal_leases::lock_on(tx, namespace).await?;
    let query = Query::update()
        .table(Alias::new("sync_meta"))
        .value(
            Alias::new("initialized_at"),
            chrono::Utc::now().to_rfc3339(),
        )
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("initialized_at")).is_null())
        .to_owned();
    tx.execute(&query).await?;
    Ok(())
}

pub async fn checkpoint_on<M: SyncModel, C: ConnectionTrait>(
    db: &C,
    key: &CheckpointKey,
) -> Result<Option<ReceiveProgress>> {
    valid_key(key)?;
    let query = Query::select()
        .columns(["sequence", "anchor", "normalized_scope"].map(Alias::new))
        .from(Alias::new("sync_checkpoints"))
        .cond_where(checkpoint_filter(key))
        .to_owned();
    let Some(row) = db.query_one(&query).await? else {
        return Ok(None);
    };
    let scope: p::Scope<M> = serde_json::from_slice(&row.try_get_by_index::<Vec<u8>>(2)?)?;
    ensure!(
        p::scope_id(&scope)? == key.scope_id,
        "stored checkpoint scope mismatch"
    );
    Ok(Some(progress(
        row.try_get_by_index(0)?,
        row.try_get_by_index(1)?,
    )?))
}

/// The caller uses one database snapshot for the identity, floor and anchor reads.
pub async fn history_status_on<C: ConnectionTrait>(
    db: &C,
    identity: &p::StoreIdentity,
    key: &CheckpointKey,
    checkpoint: Option<ReceiveProgress>,
) -> Result<HistoryStatus> {
    valid_key(key)?;
    ensure!(
        key.namespace == identity.namespace && key.source_replica == identity.replica,
        p::SyncError::Invalid("source identity mismatch".into())
    );
    if key.source_generation != identity.generation {
        return Ok(HistoryStatus::GenerationChanged {
            current: identity.generation,
        });
    }
    let query = Query::select()
        .columns(
            [
                "head",
                "head_anchor",
                "history_floor",
                "floor_anchor",
                "generation",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_meta"))
        .and_where(Expr::col(Alias::new("namespace")).eq(identity.namespace.as_bytes().to_vec()))
        .to_owned();
    let row = db
        .query_one(&query)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing namespace"))?;
    ensure!(
        row.try_get_by_index::<Vec<u8>>(4)? == identity.generation.as_bytes(),
        "generation changed during read"
    );
    let head = progress(row.try_get_by_index(0)?, row.try_get_by_index(1)?)?;
    let floor = progress(row.try_get_by_index(2)?, row.try_get_by_index(3)?)?;
    ensure!(
        floor.sequence <= head.sequence,
        "history floor exceeds head"
    );
    let Some(checkpoint) = checkpoint else {
        return Ok(HistoryStatus::Uninitialized { head });
    };
    if checkpoint.sequence > head.sequence {
        return Ok(HistoryStatus::HeadBehind {
            head: head.sequence,
        });
    }
    if checkpoint.sequence < floor.sequence {
        return Ok(HistoryStatus::BelowFloor { floor });
    }
    let expected = if checkpoint.sequence == floor.sequence {
        floor.anchor
    } else if checkpoint.sequence == head.sequence {
        head.anchor
    } else {
        let query = Query::select()
            .column(Alias::new("anchor_after"))
            .from(Alias::new("sync_journal"))
            .and_where(
                Expr::col(Alias::new("namespace")).eq(identity.namespace.as_bytes().to_vec()),
            )
            .and_where(
                Expr::col(Alias::new("generation")).eq(identity.generation.as_bytes().to_vec()),
            )
            .and_where(Expr::col(Alias::new("sequence")).eq(i64::try_from(checkpoint.sequence)?))
            .to_owned();
        let row = db
            .query_one(&query)
            .await?
            .ok_or_else(|| anyhow::anyhow!("retained journal has a gap"))?;
        let anchor: Vec<u8> = row.try_get_by_index(0)?;
        anchor
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid journal anchor"))?
    };
    if expected != checkpoint.anchor {
        return Ok(HistoryStatus::AnchorMismatch { checkpoint });
    }
    Ok(HistoryStatus::Continuous { head, floor })
}

/// Only verified complete coverage may initialize receive progress. Call this
/// after every range and its application projection have committed.
pub async fn initialize_checkpoint_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    local: &p::StoreIdentity,
    key: &CheckpointKey,
    scope: &p::Scope<M>,
    received: &ReceiveProgress,
) -> Result<ReceiveProgress> {
    valid_key(key)?;
    ensure!(
        key.namespace == local.namespace
            && key.scope_id == p::scope_id(scope)?
            && (key.source_replica != local.replica || key.source_generation != local.generation),
        p::SyncError::Invalid("invalid checkpoint initialization".into())
    );
    crate::causal_leases::lock_on(tx, local.namespace).await?;
    if let Some(current) = checkpoint_on::<M, _>(tx, key).await? {
        ensure!(
            current.sequence <= received.sequence,
            p::SyncError::RetryAt(current.clone())
        );
        ensure!(
            current.sequence != received.sequence || current.anchor == received.anchor,
            p::SyncError::RetryAt(current)
        );
    }
    let query = Query::insert()
        .into_table(Alias::new("sync_checkpoints"))
        .columns(
            [
                "namespace",
                "source_replica",
                "source_generation",
                "scope_digest",
                "normalized_scope",
                "sequence",
                "anchor",
                "completed_at",
            ]
            .map(Alias::new),
        )
        .values_panic([
            key.namespace.as_bytes().to_vec().into(),
            key.source_replica.as_bytes().to_vec().into(),
            key.source_generation.as_bytes().to_vec().into(),
            key.scope_id.to_vec().into(),
            serde_json::to_vec(scope)?.into(),
            i64::try_from(received.sequence)?.into(),
            received.anchor.to_vec().into(),
            chrono::Utc::now().to_rfc3339().into(),
        ])
        .on_conflict(
            OnConflict::columns(
                [
                    "namespace",
                    "source_replica",
                    "source_generation",
                    "scope_digest",
                ]
                .map(Alias::new),
            )
            .update_columns(["sequence", "anchor", "completed_at"].map(Alias::new))
            .to_owned(),
        )
        .to_owned();
    tx.execute(&query).await?;
    Ok(received.clone())
}

/// Completion never advances a checkpoint; all preceding changes must already
/// have committed at this exact position in their receive transactions.
pub async fn finish_session_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    key: &CheckpointKey,
    through: &ReceiveProgress,
) -> Result<()> {
    ensure!(
        key.namespace == namespace,
        p::SyncError::Invalid("namespace mismatch".into())
    );
    crate::causal_leases::lock_on(tx, namespace).await?;
    let current = checkpoint_on::<M, _>(tx, key)
        .await?
        .ok_or(p::SyncError::IncompleteCoverage)?;
    ensure!(current == *through, p::SyncError::RetryAt(current));
    let query = Query::update()
        .table(Alias::new("sync_checkpoints"))
        .value(Alias::new("completed_at"), chrono::Utc::now().to_rfc3339())
        .cond_where(checkpoint_filter(key))
        .to_owned();
    ensure!(
        tx.execute(&query).await?.rows_affected() == 1,
        "checkpoint disappeared"
    );
    Ok(())
}
