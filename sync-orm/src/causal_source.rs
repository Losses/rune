//! Durable bounded building blocks for pinned causal source views.
//!
//! Work is advanced under caller transactions; immutable candidate values remain
//! in the primary retained candidate relation and are never copied into a source table.
use anyhow::{Result, ensure};
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, ExprTrait, FromQueryResult,
    sea_query::{Alias, Condition, Expr, Func, Order, Query},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use sync::protocol::{self, KeyRange, SyncModel, WorkLimits};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CandidatePosition {
    object: Uuid,
    writer: Uuid,
    counter: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingHeader {
    position: CandidatePosition,
    author: Uuid,
    revision: i64,
    digest: Vec<u8>,
    body_len: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CandidateCount {
    after: Option<CandidatePosition>,
    pending: Option<PendingHeader>,
    pub count: u64,
    pub bytes: u64,
    pub done: bool,
}
pub(crate) enum CandidateStep<M: SyncModel> {
    Header,
    Candidate(protocol::Candidate<M>),
    Done,
}
#[derive(FromQueryResult)]
struct Header {
    object: Vec<u8>,
    writer: Vec<u8>,
    counter: i64,
    author: Vec<u8>,
    revision: i64,
    digest: Vec<u8>,
    body_len: i64,
}

/// Advance at most one physical row of the count pass at a captured state.
/// The first step reads only a size-checked header; the next step reads and
/// validates that exact immutable body. This permits a one-row work budget.
pub(crate) async fn count_candidate_step_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    range: &KeyRange<M>,
    state_seq: i64,
    state: &mut CandidateCount,
    limits: WorkLimits,
) -> Result<CandidateStep<M>> {
    ensure!(
        !namespace.is_nil()
            && state_seq >= 0
            && limits.max_rows_scanned > 0
            && limits.max_bytes > 0,
        "invalid candidate count step"
    );
    if state.done {
        return Ok(CandidateStep::Done);
    }
    let tag = M::collection_tag(&range.collection).as_bytes().to_vec();
    if let Some(header) = state.pending.clone() {
        let required = header
            .body_len
            .checked_add(96)
            .ok_or_else(|| anyhow::anyhow!("candidate size overflow"))?;
        if required > limits.max_bytes {
            return Err(protocol::SyncError::LimitTooSmall { required }.into());
        }
        let q = Query::select()
            .column(Alias::new("body"))
            .from(Alias::new("sync_candidates"))
            .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("collection")).eq(tag))
            .and_where(
                Expr::col(Alias::new("writer")).eq(header.position.writer.as_bytes().to_vec()),
            )
            .and_where(Expr::col(Alias::new("counter")).eq(header.position.counter))
            .to_owned();
        let row = tx
            .query_one(&q)
            .await?
            .ok_or_else(|| anyhow::anyhow!("pinned candidate disappeared"))?;
        let body: Vec<u8> = row.try_get_by_index(0)?;
        ensure!(
            body.len() as u64 == header.body_len,
            "pinned candidate size changed"
        );
        let candidate = protocol::decode_candidate::<M>(&body)?;
        ensure!(
            candidate.namespace == namespace
                && candidate.key.collection == range.collection
                && candidate.key.id == header.position.object
                && candidate.dot.writer_incarnation == header.position.writer
                && i64::try_from(candidate.dot.counter)? == header.position.counter
                && candidate.revision.actor == header.author
                && i64::try_from(candidate.revision.counter)? == header.revision
                && protocol::candidate_digest(&candidate)?.as_slice() == header.digest,
            "candidate identity mismatch"
        );
        state.count = state
            .count
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("candidate count overflow"))?;
        state.bytes = state
            .bytes
            .checked_add(body.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("candidate byte overflow"))?;
        state.after = Some(header.position);
        state.pending = None;
        return Ok(CandidateStep::Candidate(candidate));
    }
    const HEADER_BYTES: u64 = 128;
    if limits.max_bytes < HEADER_BYTES {
        return Err(protocol::SyncError::LimitTooSmall {
            required: HEADER_BYTES,
        }
        .into());
    }
    let mut q = Query::select();
    q.columns(
        [
            "object", "writer", "counter", "author", "revision", "digest",
        ]
        .map(Alias::new),
    )
    .expr_as(
        Func::cust(Alias::new("length")).arg(Expr::col(Alias::new("body"))),
        Alias::new("body_len"),
    )
    .from(Alias::new("sync_candidates"))
    .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
    .and_where(Expr::col(Alias::new("collection")).eq(tag))
    .and_where(Expr::col(Alias::new("live_from")).is_not_null())
    .and_where(Expr::col(Alias::new("live_from")).lte(state_seq))
    .cond_where(
        Condition::any()
            .add(Expr::col(Alias::new("live_until")).is_null())
            .add(Expr::col(Alias::new("live_until")).gt(state_seq)),
    );
    if let Some(lower) = range.lower {
        q.and_where(Expr::col(Alias::new("object")).gte(lower.as_bytes().to_vec()));
    }
    if let Some(upper) = range.upper {
        q.and_where(Expr::col(Alias::new("object")).lt(upper.as_bytes().to_vec()));
    }
    if let Some(cursor) = &state.after {
        let object = cursor.object.as_bytes().to_vec();
        let writer = cursor.writer.as_bytes().to_vec();
        q.cond_where(
            Condition::any()
                .add(Expr::col(Alias::new("object")).gt(object.clone()))
                .add(
                    Condition::all()
                        .add(Expr::col(Alias::new("object")).eq(object.clone()))
                        .add(Expr::col(Alias::new("writer")).gt(writer.clone())),
                )
                .add(
                    Condition::all()
                        .add(Expr::col(Alias::new("object")).eq(object))
                        .add(Expr::col(Alias::new("writer")).eq(writer))
                        .add(Expr::col(Alias::new("counter")).gt(cursor.counter)),
                ),
        );
    }
    q.order_by(Alias::new("object"), Order::Asc)
        .order_by(Alias::new("writer"), Order::Asc)
        .order_by(Alias::new("counter"), Order::Asc)
        .limit(1);
    let Some(row) = Header::find_by_statement(tx.get_database_backend().build(&q.to_owned()))
        .one(tx)
        .await?
    else {
        state.done = true;
        return Ok(CandidateStep::Done);
    };
    let position = CandidatePosition {
        object: Uuid::from_slice(&row.object)?,
        writer: Uuid::from_slice(&row.writer)?,
        counter: row.counter,
    };
    let author = Uuid::from_slice(&row.author)?;
    ensure!(
        !position.object.is_nil()
            && !position.writer.is_nil()
            && !author.is_nil()
            && position.counter > 0
            && row.revision > 0
            && row.digest.len() == 32,
        "invalid stored candidate header"
    );
    let body_len = u64::try_from(row.body_len)?;
    ensure!(
        body_len > 0 && body_len <= 64 * 1024,
        "invalid stored candidate length"
    );
    state.pending = Some(PendingHeader {
        position,
        author,
        revision: row.revision,
        digest: row.digest,
        body_len,
    });
    Ok(CandidateStep::Header)
}

#[derive(Debug, Default)]
struct CandidateReadUsage {
    rows: u64,
    bytes: u64,
}

fn pending_batch_header(row: &crate::causal_candidates::BatchHeader) -> Result<PendingHeader> {
    let position = CandidatePosition {
        object: Uuid::from_slice(&row.object)?,
        writer: Uuid::from_slice(&row.writer)?,
        counter: row.counter,
    };
    let author = Uuid::from_slice(&row.author)?;
    let body_len = u64::try_from(row.body_len)?;
    ensure!(
        !position.object.is_nil()
            && !position.writer.is_nil()
            && !author.is_nil()
            && position.counter > 0
            && row.revision > 0
            && row.digest.len() == 32
            && body_len > 0
            && body_len <= 64 * 1024,
        "invalid stored candidate header"
    );
    Ok(PendingHeader {
        position,
        author,
        revision: row.revision,
        digest: row.digest.clone(),
        body_len,
    })
}

/// The enclosing operation reserves its checkpoint before supplying this budget.
async fn scan_candidate_batch_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    range: &KeyRange<M>,
    state_seq: i64,
    state: &mut CandidateCount,
    limits: WorkLimits,
    mut consume: impl FnMut(&protocol::Candidate<M>, u64) -> Result<bool>,
) -> Result<CandidateReadUsage> {
    ensure!(
        limits.max_rows_scanned > 0 && limits.max_bytes > 0,
        "empty candidate batch budget"
    );
    let mut used = CandidateReadUsage::default();
    while !state.done && used.rows < limits.max_rows_scanned {
        let remaining_rows = limits.max_rows_scanned - used.rows;
        let remaining_bytes = limits.max_bytes - used.bytes;
        // Keep the established one-row path for a pending body and tiny budgets.
        if state.pending.is_some() || remaining_rows < 2 || remaining_bytes < 225 {
            let bytes = match &state.pending {
                Some(header) => header
                    .body_len
                    .checked_add(96)
                    .ok_or(protocol::SyncError::LogicalLimit)?,
                None => 128,
            };
            if bytes > remaining_bytes {
                if used.rows == 0 {
                    return Err(protocol::SyncError::LimitTooSmall { required: bytes }.into());
                }
                break;
            }
            let mut step_limits = limits;
            step_limits.max_rows_scanned = 1;
            step_limits.max_bytes = bytes;
            let previous_bytes = state.bytes;
            let step = count_candidate_step_on(tx, namespace, range, state_seq, state, step_limits)
                .await?;
            used.rows += 1;
            used.bytes += bytes;
            if let CandidateStep::Candidate(candidate) = step
                && !consume(&candidate, state.bytes - previous_bytes)?
            {
                break;
            }
            continue;
        }
        // Limit the OR expression as well as row and byte work. Header rows
        // are charged even when a later byte bound or callback discards them.
        let max_headers = std::cmp::min(
            std::cmp::min(remaining_rows / 2, remaining_bytes / 225),
            crate::causal_candidates::MAX_HEADER_BATCH as u64,
        );
        let headers = crate::causal_candidates::header_batch_on(
            tx,
            crate::causal_candidates::CandidatePageRequest {
                namespace,
                collection: M::collection_tag(&range.collection).as_bytes(),
                state_seq,
                lower: range.lower,
                upper: range.upper,
                after: state
                    .after
                    .as_ref()
                    .map(|p| crate::causal_candidates::Cursor {
                        object: p.object,
                        writer: p.writer,
                        counter: p.counter,
                    }),
                max_items: usize::try_from(max_headers)?,
                max_bytes: usize::try_from(remaining_bytes)?,
            },
        )
        .await?;
        if headers.is_empty() {
            used.rows += 1;
            used.bytes += 128;
            state.done = true;
            break;
        }
        used.rows += u64::try_from(headers.len())?;
        used.bytes += u64::try_from(headers.len())? * 128;
        let mut count = 0usize;
        let mut body_bytes = 0u64;
        for header in &headers {
            let size = match u64::try_from(header.body_len) {
                Ok(size) if size > 0 && size <= 64 * 1024 => size + 96,
                _ if count > 0 => break,
                _ => anyhow::bail!("invalid stored candidate length"),
            };
            if used.rows + u64::try_from(count)? + 1 > limits.max_rows_scanned
                || size > limits.max_bytes - used.bytes - body_bytes
            {
                break;
            }
            count += 1;
            body_bytes += size;
        }
        if count == 0 {
            state.pending = Some(pending_batch_header(&headers[0])?);
            break;
        }
        let bodies = crate::causal_candidates::body_batch_on(
            tx,
            namespace,
            M::collection_tag(&range.collection).as_bytes(),
            &headers[..count],
        )
        .await?;
        used.rows += u64::try_from(count)?;
        used.bytes += body_bytes;
        let mut body_map = BTreeMap::new();
        for body in bodies {
            ensure!(
                body_map
                    .insert((body.writer, body.counter), body.body)
                    .is_none(),
                "duplicate candidate body"
            );
        }
        let mut bodies = body_map;
        let mut stopped = false;
        for header in &headers[..count] {
            let pending = pending_batch_header(header)?;
            let body = bodies
                .remove(&(header.writer.clone(), header.counter))
                .ok_or_else(|| anyhow::anyhow!("pinned candidate disappeared or changed"))?;
            ensure!(
                body.len() as u64 == pending.body_len,
                "pinned candidate size changed"
            );
            let candidate = protocol::decode_candidate::<M>(&body)?;
            ensure!(
                candidate.namespace == namespace
                    && candidate.key.collection == range.collection
                    && candidate.key.id == pending.position.object
                    && candidate.dot.writer_incarnation == pending.position.writer
                    && i64::try_from(candidate.dot.counter)? == pending.position.counter
                    && candidate.revision.actor == pending.author
                    && i64::try_from(candidate.revision.counter)? == pending.revision
                    && protocol::candidate_digest(&candidate)?.as_slice() == pending.digest,
                "candidate identity mismatch"
            );
            state.count = state
                .count
                .checked_add(1)
                .ok_or(protocol::SyncError::LogicalLimit)?;
            state.bytes = state
                .bytes
                .checked_add(body.len() as u64)
                .ok_or(protocol::SyncError::LogicalLimit)?;
            state.after = Some(pending.position);
            if !consume(&candidate, body.len() as u64)? {
                stopped = true;
                break;
            }
        }
        if stopped {
            break;
        }
        if count < headers.len() {
            state.pending = Some(pending_batch_header(&headers[count])?);
            break;
        }
    }
    Ok(used)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
struct ReadySummary<M: SyncModel> {
    kind: String,
    version: u32,
    snapshot: Uuid,
    range: KeyRange<M>,
    cursor_generation: Uuid,
    limits: WorkLimits,
    summary: protocol::RangeSummary<M>,
}
fn summary_work_id<M: SyncModel>(snapshot: Uuid, range: &KeyRange<M>) -> Result<Uuid> {
    let encoded = serde_json::to_vec(range)?;
    let mut hash = blake3::Hasher::new();
    hash.update(b"sync.source.summary.work.v1\0");
    hash.update(snapshot.as_bytes());
    hash.update(&encoded);
    let mut id = [0u8; 16];
    id.copy_from_slice(&hash.finalize().as_bytes()[..16]);
    id[6] = (id[6] & 0x0f) | 0x50;
    id[8] = (id[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(id))
}

/// Look up an already completed exact summary without initiating source work.
pub async fn summary_ready_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    snapshot_id: Uuid,
    range: &KeyRange<M>,
    now_ms: i64,
) -> Result<protocol::RangeSummary<M>> {
    let lease = crate::causal_leases::load_on(tx, namespace, snapshot_id)
        .await?
        .ok_or(protocol::SyncError::SnapshotExpired)?;
    if lease.status != 1
        || lease.expires_at_ms <= now_ms
        || lease.descriptor.is_none()
        || lease.retained_root.is_none()
    {
        return Err(protocol::SyncError::SnapshotExpired.into());
    }
    let descriptor: protocol::SnapshotDescriptor<M> = serde_json::from_slice(
        lease
            .descriptor
            .as_deref()
            .ok_or(protocol::SyncError::SnapshotExpired)?,
    )?;
    ensure!(
        descriptor.snapshot_id == snapshot_id
            && descriptor.source.namespace == namespace
            && descriptor.source.model.model == M::model_id()
            && descriptor.source.model.schema == M::schema_id()
            && descriptor.expires_unix_ms == u64::try_from(lease.expires_at_ms)?
            && descriptor.source.generation.as_bytes() == lease.generation.as_slice()
            && descriptor.head.sequence == u64::try_from(lease.head)?
            && descriptor.head.anchor.as_slice() == lease.head_anchor.as_slice()
            && serde_json::to_vec(&descriptor.scope)? == lease.scope,
        "snapshot descriptor mismatch"
    );
    ensure!(
        descriptor.scope.collections.contains(&range.collection)
            && range.lower.zip(range.upper).is_none_or(|(a, b)| a < b),
        "invalid summary range"
    );
    if let Some(summary) = descriptor
        .roots
        .iter()
        .find(|summary| &summary.range == range)
    {
        return Ok(summary.clone());
    }
    let id = summary_work_id(snapshot_id, range)?;
    let entry = crate::causal_leases::load_on(tx, namespace, id)
        .await?
        .ok_or(protocol::SyncError::StaleWork)?;
    if entry.status == 2
        || entry.expires_at_ms <= now_ms
        || entry.generation != lease.generation
        || entry.state_seq != lease.state_seq
        || entry.head != lease.head
        || entry.head_anchor != lease.head_anchor
        || entry.scope != lease.scope
        || entry.parent_lease.as_deref() != Some(snapshot_id.as_bytes().as_slice())
    {
        return Err(protocol::SyncError::StaleWork.into());
    }
    let cache: ReadySummary<M> = serde_json::from_slice(
        entry
            .build_state
            .as_deref()
            .ok_or(protocol::SyncError::StaleWork)?,
    )
    .map_err(|_| protocol::SyncError::StaleWork)?;
    if cache.kind != "summary_ready"
        || cache.version != 1
        || cache.snapshot != snapshot_id
        || &cache.range != range
        || cache.summary.snapshot_id != snapshot_id
        || &cache.summary.range != range
    {
        return Err(protocol::SyncError::StaleWork.into());
    }
    Ok(cache.summary)
}

/// Count and choose boundaries from bounded headers. Hash and leaf passes
/// validate immutable bodies before publishing or transferring their values.
async fn scan_candidate_headers_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    range: &KeyRange<M>,
    state_seq: i64,
    state: &mut CandidateCount,
    limits: WorkLimits,
    mut consume: impl FnMut(&CandidatePosition, u64) -> Result<bool>,
) -> Result<CandidateReadUsage> {
    use crate::causal_candidates::{CandidatePageRequest, Cursor, header_batch_on};
    const HEADER_BYTES: u64 = 128;
    ensure!(limits.max_rows_scanned > 0, "empty candidate header budget");
    if limits.max_bytes < HEADER_BYTES {
        return Err(protocol::SyncError::LimitTooSmall {
            required: HEADER_BYTES,
        }
        .into());
    }
    let mut used = CandidateReadUsage::default();
    if state.done {
        return Ok(used);
    }
    if let Some(header) = state.pending.take() {
        state.count = state
            .count
            .checked_add(1)
            .ok_or(protocol::SyncError::LogicalLimit)?;
        state.bytes = state
            .bytes
            .checked_add(header.body_len)
            .ok_or(protocol::SyncError::LogicalLimit)?;
        state.after = Some(header.position.clone());
        used.rows = 1;
        used.bytes = HEADER_BYTES;
        if !consume(&header.position, header.body_len)? {
            return Ok(used);
        }
    }
    while used.rows < limits.max_rows_scanned && limits.max_bytes - used.bytes >= HEADER_BYTES {
        let capacity = std::cmp::min(
            crate::causal_candidates::MAX_HEADER_BATCH as u64,
            std::cmp::min(
                limits.max_rows_scanned - used.rows,
                (limits.max_bytes - used.bytes) / HEADER_BYTES,
            ),
        ) as usize;
        let headers = header_batch_on(
            tx,
            CandidatePageRequest {
                namespace,
                collection: M::collection_tag(&range.collection).as_bytes(),
                state_seq,
                lower: range.lower,
                upper: range.upper,
                after: state.after.as_ref().map(|position| Cursor {
                    object: position.object,
                    writer: position.writer,
                    counter: position.counter,
                }),
                max_items: capacity,
                max_bytes: (capacity as u64 * HEADER_BYTES) as usize,
            },
        )
        .await?;
        let fetched = headers.len();
        used.rows += fetched.max(1) as u64;
        used.bytes += fetched.max(1) as u64 * HEADER_BYTES;
        for row in headers {
            let header = pending_batch_header(&row)?;
            state.count = state
                .count
                .checked_add(1)
                .ok_or(protocol::SyncError::LogicalLimit)?;
            state.bytes = state
                .bytes
                .checked_add(header.body_len)
                .ok_or(protocol::SyncError::LogicalLimit)?;
            state.after = Some(header.position.clone());
            if !consume(&header.position, header.body_len)? {
                return Ok(used);
            }
        }
        if fetched < capacity {
            state.done = true;
            break;
        }
    }
    Ok(used)
}

/// Resumable candidate feed for a pinned range.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CandidateHash {
    scan: CandidateCount,
    hasher_checkpoint: Vec<u8>,
}
impl CandidateHash {
    pub(crate) fn new<M: SyncModel>(hasher: &protocol::RangeHasher<M>) -> Result<Self> {
        Ok(Self {
            scan: CandidateCount::default(),
            hasher_checkpoint: hasher.checkpoint()?,
        })
    }
    pub(crate) fn done(&self) -> bool {
        self.scan.done
    }
    pub(crate) fn finish<M: SyncModel>(
        self,
        scope: &protocol::Scope<M>,
        range: &KeyRange<M>,
    ) -> Result<protocol::Digest> {
        ensure!(self.scan.done, "candidate hash pass is incomplete");
        Ok(
            protocol::RangeHasher::<M>::from_checkpoint(scope, range, &self.hasher_checkpoint)?
                .finish()?,
        )
    }
}

pub(crate) async fn hash_candidate_step_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    scope: &protocol::Scope<M>,
    range: &KeyRange<M>,
    state_seq: i64,
    state: &mut CandidateHash,
    limits: WorkLimits,
) -> Result<bool> {
    if state.scan.done {
        return Ok(false);
    }
    let mut hasher =
        protocol::RangeHasher::<M>::from_checkpoint(scope, range, &state.hasher_checkpoint)?;
    scan_candidate_batch_on(
        tx,
        namespace,
        range,
        state_seq,
        &mut state.scan,
        limits,
        |candidate, _| {
            hasher.push_candidate(candidate)?;
            Ok(true)
        },
    )
    .await?;
    state.hasher_checkpoint = hasher.checkpoint()?;
    Ok(true)
}

/// A previously revoked request is already released; no permanent receipt is
/// required. The caller commits this transaction before reporting completion.
pub async fn release_request_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    request_id: Uuid,
) -> Result<()> {
    ensure!(
        !namespace.is_nil() && !request_id.is_nil(),
        "invalid snapshot request"
    );
    crate::causal_leases::lock_on(tx, namespace).await?;
    if crate::causal_leases::load_on(tx, namespace, request_id)
        .await?
        .is_some()
    {
        crate::causal_leases::revoke_on(tx, namespace, request_id).await?;
    }
    Ok(())
}

/// Cancel an admitted build using its private generation. A delayed caller may
/// hold an earlier position after losing a reply; it still owns the same build.
pub async fn cancel_snapshot_build_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    cursor: protocol::WorkCursor,
) -> Result<()> {
    crate::causal_leases::lock_on(tx, namespace).await?;
    let lease = crate::causal_leases::load_on(tx, namespace, cursor.operation_id)
        .await?
        .ok_or(protocol::SyncError::StaleWork)?;
    if lease.status != 0 || lease.parent_lease.is_some() {
        return Err(protocol::SyncError::StaleWork.into());
    }
    let work: SnapshotWork<M> = serde_json::from_slice(
        lease
            .build_state
            .as_deref()
            .ok_or(protocol::SyncError::StaleWork)?,
    )
    .map_err(|_| protocol::SyncError::StaleWork)?;
    if work.cursor.operation_id != cursor.operation_id
        || work.cursor.generation != cursor.generation
        || work.cursor.position < cursor.position
        || work.source.namespace != namespace
    {
        return Err(protocol::SyncError::StaleWork.into());
    }
    crate::causal_leases::revoke_on(tx, namespace, cursor.operation_id).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{causal_schema, causal_store::CausalReplica};
    use sea_orm::{Database, TransactionTrait};
    use sync::protocol::{ObjectKey, Observed};

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Model;
    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    enum Collection {
        Notes,
        Other,
        Third,
    }
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct Note {
        text: String,
    }
    impl SyncModel for Model {
        type Collection = Collection;
        type Payload = Note;
        fn model_id() -> &'static str {
            "notes"
        }
        fn schema_id() -> &'static str {
            "1"
        }
        fn collections() -> Vec<Collection> {
            vec![Collection::Notes, Collection::Other, Collection::Third]
        }
        fn collection_tag(collection: &Collection) -> &'static str {
            match collection {
                Collection::Notes => "notes",
                Collection::Other => "other",
                Collection::Third => "third",
            }
        }
        fn collection_from_tag(tag: &str) -> Option<Collection> {
            match tag {
                "notes" => Some(Collection::Notes),
                "other" => Some(Collection::Other),
                "third" => Some(Collection::Third),
                _ => None,
            }
        }
        fn validate(_: &ObjectKey<Self>, _: &Option<Note>) -> protocol::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn snapshot_admission_retries_pin_one_view_and_reject_stale_work() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        causal_schema::install_fresh(&db).await?;
        let ns = Uuid::new_v4();
        let replica = CausalReplica::<Model>::create_empty(db.clone(), ns, Uuid::new_v4()).await?;
        let request = Uuid::new_v4();
        let scope = protocol::Scope::<Model> {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let limits = WorkLimits {
            max_requests: 1,
            max_bytes: 65536,
            max_depth: 1,
            max_rows_scanned: 1,
            max_mutations: 32,
        };
        let tx = db.begin().await?;
        crate::causal_leases::lock_on(&tx, ns).await?;
        let source = replica.identity_on(&tx).await?;
        let timing = SnapshotTiming {
            now_ms: 1,
            ttl_ms: 20,
        };
        let first = match admit_snapshot_on(
            &tx,
            source.clone(),
            request,
            scope.clone(),
            timing,
            limits,
            None,
        )
        .await?
        {
            SnapshotAdmission::Building(s) => s,
            _ => panic!(),
        };
        let repeated = match admit_snapshot_on(
            &tx,
            source.clone(),
            request,
            scope.clone(),
            timing,
            limits,
            None,
        )
        .await?
        {
            SnapshotAdmission::Building(s) => s,
            _ => panic!(),
        };
        assert_eq!(first.cursor, repeated.cursor);
        let mut bad = first.cursor.clone();
        bad.position += 1;
        assert!(matches!(
            open_on(
                &tx,
                source.clone(),
                request,
                scope.clone(),
                timing,
                limits,
                Some(bad)
            )
            .await
            .unwrap_err()
            .downcast_ref(),
            Some(protocol::SyncError::StaleWork)
        ));
        let mut cursor = Some(first.cursor.clone());
        let descriptor = loop {
            match open_on(
                &tx,
                source.clone(),
                request,
                scope.clone(),
                timing,
                limits,
                cursor,
            )
            .await?
            {
                protocol::WorkOutcome::Ready(d) => break d,
                protocol::WorkOutcome::Incomplete { cursor: next } => cursor = Some(next),
            }
        };
        let retry = open_on(
            &tx,
            source,
            request,
            scope,
            timing,
            limits,
            Some(first.cursor),
        )
        .await?;
        assert!(matches!(retry, protocol::WorkOutcome::Ready(ref d) if *d == descriptor));
        tx.commit().await?;
        Ok(())
    }

    #[tokio::test]
    async fn bounded_step_plans_preserve_identity_budget_and_lost_reply_resume() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        causal_schema::install_fresh(&db).await?;
        let namespace = Uuid::new_v4();
        let replica =
            CausalReplica::<Model>::create_empty(db.clone(), namespace, Uuid::new_v4()).await?;
        let tx = db.begin().await?;
        let source = replica.identity_on(&tx).await?;
        tx.commit().await?;
        let scope = protocol::Scope::<Model> {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let full = WorkLimits {
            max_requests: 1,
            max_bytes: 4 * 1024 * 1024,
            max_depth: 16,
            max_rows_scanned: 128,
            max_mutations: 1024,
        };
        let timing = SnapshotTiming {
            now_ms: 2,
            ttl_ms: 100,
        };
        let request = Uuid::new_v4();
        let mut cursor = None;
        let mut older = None;
        let mut fixed_steps = 0;
        let mut variable_steps = 0;
        let mut combined = false;
        let mut completed = None;
        for _ in 0..1000 {
            let mut rows = full.max_rows_scanned;
            let mut bytes = full.max_bytes;
            let mut mutations = full.max_mutations;
            let mut steps = 0;
            let mut plan: Option<SnapshotStepPlan> = None;
            loop {
                // A fresh capability request reserves full before its first DB read.
                let reserved = plan.as_ref().map_or(full, |next| next.admitted);
                if reserved.max_rows_scanned > rows
                    || reserved.max_bytes > bytes
                    || reserved.max_mutations > mutations
                {
                    assert!(steps > 0);
                    break;
                }
                let tx = db.begin().await?;
                let result = open_batch_step_on(
                    &tx,
                    source.clone(),
                    request,
                    scope.clone(),
                    timing,
                    SnapshotBatchLimits {
                        full,
                        plan: plan.take(),
                    },
                    older.clone().or_else(|| cursor.clone()),
                )
                .await?;
                let charged = result.charged;
                assert!(
                    charged.max_rows_scanned <= reserved.max_rows_scanned
                        && charged.max_bytes <= reserved.max_bytes
                        && charged.max_mutations <= reserved.max_mutations
                );
                if charged == full {
                    variable_steps += 1;
                } else {
                    fixed_steps += 1;
                }
                rows -= charged.max_rows_scanned;
                bytes -= charged.max_bytes;
                mutations -= charged.max_mutations;
                if let protocol::WorkOutcome::Incomplete { cursor: next } = result.outcome {
                    if older.is_none() {
                        older = Some(next.clone());
                    }
                    cursor = Some(next);
                    let lease = crate::causal_leases::load_on(&tx, namespace, request)
                        .await?
                        .unwrap();
                    let encoded = lease.build_state.unwrap();
                    let mut state: SnapshotWork<Model> = serde_json::from_slice(&encoded)?;
                    assert_eq!(state.limits, full);
                    assert!(state.transient_limits.is_none());
                    state.transient_limits = Some(charged);
                    state.transient_lease_bytes = 999;
                    assert_eq!(serde_json::to_vec(&state)?, encoded);
                } else if let protocol::WorkOutcome::Ready(descriptor) = result.outcome {
                    completed = Some(descriptor);
                }
                plan = result.next;
                tx.commit().await?;
                steps += 1;
                combined |= steps > 1;
                if completed.is_some() || rows == 0 || bytes == 0 || mutations == 0 {
                    break;
                }
            }
            if completed.is_some() {
                break;
            }
        }
        let descriptor =
            completed.ok_or_else(|| anyhow::anyhow!("batched snapshot did not finish"))?;
        assert!(combined && fixed_steps > 0 && variable_steps > 0);
        assert_eq!(descriptor.roots.len(), 1);
        assert_eq!(descriptor.roots[0].candidate_count, 0);
        let tx = db.begin().await?;
        let stale = SnapshotStepPlan {
            admitted: full,
            expected: older.clone().unwrap(),
        };
        let err = match open_batch_step_on(
            &tx,
            source.clone(),
            request,
            scope.clone(),
            timing,
            SnapshotBatchLimits {
                full,
                plan: Some(stale),
            },
            older.clone(),
        )
        .await
        {
            Ok(_) => panic!("stale plan accepted after publication"),
            Err(err) => err,
        };
        assert!(matches!(
            err.downcast_ref::<protocol::SyncError>(),
            Some(protocol::SyncError::StaleWork)
        ));
        let retry = open_batch_step_on(
            &tx,
            source.clone(),
            request,
            scope.clone(),
            timing,
            SnapshotBatchLimits { full, plan: None },
            older.clone(),
        )
        .await?;
        assert!(matches!(retry.outcome, protocol::WorkOutcome::Ready(_)));
        tx.commit().await?;
        let tx = db.begin().await?;
        let retry = open_on(
            &tx,
            source.clone(),
            request,
            scope.clone(),
            timing,
            full,
            older,
        )
        .await?;
        assert!(matches!(retry, protocol::WorkOutcome::Ready(_)));
        tx.commit().await?;

        // A successor plan cannot be applied after a different caller advances
        // the same lease, while the older external cursor remains resumable.
        let request = Uuid::new_v4();
        let tx = db.begin().await?;
        let first = open_batch_step_on(
            &tx,
            source.clone(),
            request,
            scope.clone(),
            timing,
            SnapshotBatchLimits { full, plan: None },
            None,
        )
        .await?;
        let old_cursor = match first.outcome {
            protocol::WorkOutcome::Incomplete { cursor } => cursor,
            _ => panic!("unexpected ready"),
        };
        let stale_plan = first.next.unwrap();
        tx.commit().await?;
        let tx = db.begin().await?;
        open_on(
            &tx,
            source.clone(),
            request,
            scope.clone(),
            timing,
            full,
            Some(old_cursor.clone()),
        )
        .await?;
        tx.commit().await?;
        let tx = db.begin().await?;
        let err = match open_batch_step_on(
            &tx,
            source.clone(),
            request,
            scope.clone(),
            timing,
            SnapshotBatchLimits {
                full,
                plan: Some(stale_plan),
            },
            Some(old_cursor.clone()),
        )
        .await
        {
            Ok(_) => panic!("stale internal plan was accepted"),
            Err(err) => err,
        };
        assert!(matches!(
            err.downcast_ref::<protocol::SyncError>(),
            Some(protocol::SyncError::StaleWork)
        ));
        tx.rollback().await?;
        let tx = db.begin().await?;
        assert!(matches!(
            open_batch_step_on(
                &tx,
                source.clone(),
                request,
                scope.clone(),
                timing,
                SnapshotBatchLimits { full, plan: None },
                Some(old_cursor)
            )
            .await?
            .outcome,
            protocol::WorkOutcome::Incomplete { .. }
        ));
        tx.commit().await?;

        // Low-row requests retain the original single-step allowance.
        let small = WorkLimits {
            max_rows_scanned: 1,
            ..full
        };
        let small_request = Uuid::new_v4();
        let tx = db.begin().await?;
        let first = open_batch_step_on(
            &tx,
            source,
            small_request,
            scope,
            timing,
            SnapshotBatchLimits {
                full: small,
                plan: None,
            },
            None,
        )
        .await?;
        assert_eq!(first.charged, small);
        assert_eq!(first.next.unwrap().admitted, small);
        tx.rollback().await?;
        Ok(())
    }

    #[tokio::test]
    async fn planned_snapshots_match_small_steps_across_captured_later_writes() -> Result<()> {
        async fn finish(
            db: &sea_orm::DatabaseConnection,
            source: &protocol::StoreIdentity,
            scope: &protocol::Scope<Model>,
            request: Uuid,
            full: WorkLimits,
            mut cursor: protocol::WorkCursor,
            mut plan: SnapshotStepPlan,
        ) -> Result<(protocol::SnapshotDescriptor<Model>, bool)> {
            let mut saw_paged_scan = false;
            for _ in 0..2000 {
                let tx = db.begin().await?;
                let result = open_batch_step_on(
                    &tx,
                    source.clone(),
                    request,
                    scope.clone(),
                    SnapshotTiming {
                        now_ms: 2,
                        ttl_ms: 100,
                    },
                    SnapshotBatchLimits {
                        full,
                        plan: Some(plan),
                    },
                    Some(cursor),
                )
                .await?;
                match result.outcome {
                    protocol::WorkOutcome::Ready(descriptor) => {
                        tx.commit().await?;
                        return Ok((descriptor, saw_paged_scan));
                    }
                    protocol::WorkOutcome::Incomplete { cursor: next } => {
                        cursor = next;
                        plan = result.next.unwrap();
                        let lease = crate::causal_leases::load_on(&tx, source.namespace, request)
                            .await?
                            .unwrap();
                        let state: SnapshotWork<Model> =
                            serde_json::from_slice(lease.build_state.as_deref().unwrap())?;
                        if (matches!(state.phase, SnapshotPhase::Context) && state.after.is_some())
                            || (matches!(state.phase, SnapshotPhase::Count)
                                && state.count.after.is_some()
                                && !state.count.done)
                        {
                            saw_paged_scan = true;
                            assert_eq!(plan.admitted, full);
                        }
                    }
                }
                tx.commit().await?;
            }
            anyhow::bail!("snapshot did not finish")
        }
        for populated in [false, true] {
            let db = Database::connect("sqlite::memory:").await?;
            causal_schema::install_fresh(&db).await?;
            let namespace = Uuid::new_v4();
            let replica =
                CausalReplica::<Model>::create_empty(db.clone(), namespace, Uuid::new_v4()).await?;
            let tx = db.begin().await?;
            if populated {
                for id in 1..=8 {
                    replica
                        .write_observed_on(
                            &tx,
                            ObjectKey {
                                collection: Collection::Notes,
                                id: Uuid::from_u128(id),
                            },
                            Some(Note {
                                text: "captured".into(),
                            }),
                            Observed::AllCurrent,
                        )
                        .await?;
                }
            }
            let source = replica.identity_on(&tx).await?;
            let scope = protocol::Scope::<Model> {
                collections: [Collection::Notes].into_iter().collect(),
            };
            let wide = WorkLimits {
                max_requests: 16,
                max_bytes: 4 * 1024 * 1024,
                max_depth: 16,
                max_rows_scanned: 128,
                max_mutations: 1024,
            };
            let small = WorkLimits {
                max_rows_scanned: 1,
                ..wide
            };
            let timing = SnapshotTiming {
                now_ms: 2,
                ttl_ms: 100,
            };
            let wide_request = Uuid::new_v4();
            let small_request = Uuid::new_v4();
            let first_wide = open_batch_step_on(
                &tx,
                source.clone(),
                wide_request,
                scope.clone(),
                timing,
                SnapshotBatchLimits {
                    full: wide,
                    plan: None,
                },
                None,
            )
            .await?;
            let first_small = open_batch_step_on(
                &tx,
                source.clone(),
                small_request,
                scope.clone(),
                timing,
                SnapshotBatchLimits {
                    full: small,
                    plan: None,
                },
                None,
            )
            .await?;
            let wide_cursor = match first_wide.outcome {
                protocol::WorkOutcome::Incomplete { cursor } => cursor,
                _ => panic!("unexpected ready"),
            };
            let small_cursor = match first_small.outcome {
                protocol::WorkOutcome::Incomplete { cursor } => cursor,
                _ => panic!("unexpected ready"),
            };
            let wide_plan = first_wide.next.unwrap();
            let small_plan = first_small.next.unwrap();
            assert_eq!(wide_plan.admitted.max_rows_scanned, 5);
            assert_eq!(small_plan.admitted, small);
            tx.commit().await?;
            let tx = db.begin().await?;
            replica
                .write_observed_on(
                    &tx,
                    ObjectKey {
                        collection: Collection::Notes,
                        id: Uuid::from_u128(1),
                    },
                    Some(Note {
                        text: "later".into(),
                    }),
                    Observed::AllCurrent,
                )
                .await?;
            tx.commit().await?;
            let (wide_descriptor, saw_paged_scan) = finish(
                &db,
                &source,
                &scope,
                wide_request,
                wide,
                wide_cursor,
                wide_plan,
            )
            .await?;
            let (small_descriptor, _) = finish(
                &db,
                &source,
                &scope,
                small_request,
                small,
                small_cursor,
                small_plan,
            )
            .await?;
            assert_eq!(saw_paged_scan, populated);
            assert_eq!(wide_descriptor.head, small_descriptor.head);
            let actual = &wide_descriptor.roots[0];
            let expected = &small_descriptor.roots[0];
            assert_eq!(actual.candidate_count, if populated { 8 } else { 0 });
            assert_eq!(actual.candidate_count, expected.candidate_count);
            assert_eq!(actual.candidate_bytes, expected.candidate_bytes);
            assert_eq!(actual.aggregate, expected.aggregate);
        }
        Ok(())
    }

    #[tokio::test]
    async fn public_open_resumes_across_transactions_and_publishes_complete_empty_view()
    -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        causal_schema::install_fresh(&db).await?;
        let namespace = Uuid::new_v4();
        let replica =
            CausalReplica::<Model>::create_empty(db.clone(), namespace, Uuid::new_v4()).await?;
        let tx = db.begin().await?;
        let source = replica.identity_on(&tx).await?;
        tx.commit().await?;
        let scope: protocol::Scope<Model> = protocol::Scope {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let limits = WorkLimits {
            max_requests: 1,
            max_bytes: 65536,
            max_depth: 1,
            max_rows_scanned: 1,
            max_mutations: 32,
        };
        let request = Uuid::new_v4();
        let mut cursor = None;
        let mut descriptor = None;
        for _ in 0..1000 {
            let tx = db.begin().await?;
            match open_on(
                &tx,
                source.clone(),
                request,
                scope.clone(),
                crate::causal_source::SnapshotTiming {
                    now_ms: 2,
                    ttl_ms: 100,
                },
                limits,
                cursor.clone(),
            )
            .await?
            {
                protocol::WorkOutcome::Incomplete { cursor: next } => {
                    cursor = Some(serde_json::from_slice(&serde_json::to_vec(&next)?)?)
                }
                protocol::WorkOutcome::Ready(ready) => {
                    descriptor = Some(ready);
                    tx.commit().await?;
                    break;
                }
            }
            tx.commit().await?;
        }
        let descriptor = descriptor.ok_or_else(|| anyhow::anyhow!("snapshot did not finish"))?;
        assert_eq!(descriptor.snapshot_id, request);
        assert_eq!(descriptor.roots.len(), 1);
        assert_eq!(descriptor.roots[0].candidate_count, 0);
        let tx = db.begin().await?;
        let retry = open_on(
            &tx,
            source,
            request,
            scope,
            crate::causal_source::SnapshotTiming {
                now_ms: 2,
                ttl_ms: 100,
            },
            limits,
            None,
        )
        .await?;
        match retry {
            protocol::WorkOutcome::Ready(repeated) => assert_eq!(repeated, descriptor),
            _ => panic!("ready retry not stable"),
        }
        let range = protocol::KeyRange::<Model> {
            collection: Collection::Notes,
            lower: Some(Uuid::from_u128(1)),
            upper: Some(Uuid::from_u128(2)),
        };
        assert!(matches!(
            summarize_on(&tx, namespace, request, range, 2, limits, None).await?,
            protocol::WorkOutcome::Incomplete { .. }
        ));
        crate::causal_leases::revoke_on(&tx, namespace, request).await?;
        assert_eq!(
            crate::causal_leases::minimum_pins_on(&tx, namespace).await?,
            None
        );
        tx.commit().await?;
        Ok(())
    }

    #[tokio::test]
    async fn public_open_preserves_captured_nonempty_view_during_later_writes() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        causal_schema::install_fresh(&db).await?;
        let namespace = Uuid::new_v4();
        let replica =
            CausalReplica::<Model>::create_empty(db.clone(), namespace, Uuid::new_v4()).await?;
        let tx = db.begin().await?;
        replica
            .write_observed_on(
                &tx,
                ObjectKey {
                    collection: Collection::Notes,
                    id: Uuid::from_u128(10),
                },
                Some(Note {
                    text: "before".into(),
                }),
                Observed::AllCurrent,
            )
            .await?;
        let source = replica.identity_on(&tx).await?;
        let scope: protocol::Scope<Model> = protocol::Scope {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let atomic_limits = WorkLimits {
            max_requests: 16,
            max_bytes: 8 * 1024 * 1024,
            max_depth: 16,
            max_rows_scanned: 1024,
            max_mutations: 1024,
        };
        let atomic = crate::causal_snapshot::open_bounded_on(
            &tx,
            source.clone(),
            scope.clone(),
            2,
            100,
            atomic_limits,
        )
        .await?;
        tx.commit().await?;
        let limits = WorkLimits {
            max_requests: 1,
            max_bytes: 65536,
            max_depth: 1,
            max_rows_scanned: 1,
            max_mutations: 32,
        };
        let request = Uuid::new_v4();
        let tx = db.begin().await?;
        let first = open_on(
            &tx,
            source.clone(),
            request,
            scope.clone(),
            crate::causal_source::SnapshotTiming {
                now_ms: 2,
                ttl_ms: 100,
            },
            limits,
            None,
        )
        .await?;
        let mut cursor = match first {
            protocol::WorkOutcome::Incomplete { cursor } => Some(cursor),
            _ => panic!("unexpected ready"),
        };
        tx.commit().await?;
        let tx = db.begin().await?;
        replica
            .write_observed_on(
                &tx,
                ObjectKey {
                    collection: Collection::Notes,
                    id: Uuid::from_u128(20),
                },
                Some(Note {
                    text: "after".into(),
                }),
                Observed::AllCurrent,
            )
            .await?;
        tx.commit().await?;
        let mut descriptor = None;
        for _ in 0..5000 {
            let tx = db.begin().await?;
            match open_on(
                &tx,
                source.clone(),
                request,
                scope.clone(),
                crate::causal_source::SnapshotTiming {
                    now_ms: 2,
                    ttl_ms: 100,
                },
                limits,
                cursor.clone(),
            )
            .await?
            {
                protocol::WorkOutcome::Incomplete { cursor: next } => {
                    cursor = Some(serde_json::from_slice(&serde_json::to_vec(&next)?)?)
                }
                protocol::WorkOutcome::Ready(ready) => {
                    descriptor = Some(ready);
                    tx.commit().await?;
                    break;
                }
            }
            tx.commit().await?;
        }
        let descriptor = descriptor.ok_or_else(|| anyhow::anyhow!("snapshot did not finish"))?;
        assert_eq!(descriptor.head, atomic.head);
        assert_eq!(descriptor.roots[0].candidate_count, 1);
        assert_eq!(
            descriptor.roots[0].candidate_count,
            atomic.roots[0].candidate_count
        );
        assert_eq!(
            descriptor.roots[0].candidate_bytes,
            atomic.roots[0].candidate_bytes
        );

        assert_eq!(descriptor.contexts, atomic.contexts);
        assert_eq!(descriptor.roots[0].aggregate, atomic.roots[0].aggregate);
        let tx = db.begin().await?;
        let context = &descriptor.contexts[0].context;
        let (_, bytes) = crate::causal_manifests::read_on(
            &tx,
            namespace,
            context.manifest.manifest_digest,
            65536,
            100,
        )
        .await?;
        assert_eq!(
            protocol::context_digest(&protocol::decode_context(&bytes)?)?,
            context.digest
        );
        tx.commit().await?;
        let subrange = protocol::KeyRange::<Model> {
            collection: Collection::Notes,
            lower: Some(Uuid::from_u128(5)),
            upper: Some(Uuid::from_u128(15)),
        };
        let mut cursor = None;
        let mut result = None;
        for _ in 0..500 {
            let tx = db.begin().await?;
            match summarize_on(
                &tx,
                namespace,
                request,
                subrange.clone(),
                2,
                limits,
                cursor.clone(),
            )
            .await?
            {
                protocol::WorkOutcome::Incomplete { cursor: next } => {
                    cursor = Some(serde_json::from_slice(&serde_json::to_vec(&next)?)?)
                }
                protocol::WorkOutcome::Ready(summary) => {
                    result = Some(summary);
                    tx.commit().await?;
                    break;
                }
            }
            tx.commit().await?;
        }
        let summary = result.ok_or_else(|| anyhow::anyhow!("summary did not finish"))?;
        let tx = db.begin().await?;
        let lease = crate::causal_leases::ready_on(&tx, namespace, request, 2).await?;
        let candidates = crate::causal_candidates::page_on(
            &tx,
            crate::causal_candidates::CandidatePageRequest {
                namespace,
                collection: b"notes",
                state_seq: lease.state_seq,
                lower: subrange.lower,
                upper: subrange.upper,
                after: None,
                max_items: 100,
                max_bytes: 65536,
            },
        )
        .await?;
        let total_bytes = candidates
            .rows
            .iter()
            .map(|row| row.body.len() as u64)
            .sum();
        let mut hash = protocol::RangeHasher::<Model>::new(
            &scope,
            subrange.clone(),
            candidates.rows.len() as u64,
            total_bytes,
        )?;

        for row in &candidates.rows {
            hash.push_candidate(&protocol::decode_candidate::<Model>(&row.body)?)?;
        }
        assert_eq!(summary.aggregate, hash.finish()?);
        assert_eq!(summary.candidate_count, 1);
        assert_eq!(
            summary_ready_on::<Model>(&tx, namespace, request, &subrange, 2).await?,
            summary
        );
        tx.commit().await?;
        let whole = protocol::KeyRange::<Model> {
            collection: Collection::Notes,
            lower: None,
            upper: None,
        };
        let mut split_cursor = None;
        let mut children = None;
        for _ in 0..500 {
            let tx = db.begin().await?;
            match split_on(
                &tx,
                namespace,
                request,
                whole.clone(),
                2,
                limits,
                split_cursor.clone(),
            )
            .await?
            {
                protocol::WorkOutcome::Incomplete { cursor } => {
                    split_cursor = Some(serde_json::from_slice(&serde_json::to_vec(&cursor)?)?)
                }
                protocol::WorkOutcome::Ready(result) => {
                    children = Some(result);
                    tx.commit().await?;
                    break;
                }
            }
            tx.commit().await?;
        }
        let children = children.ok_or_else(|| anyhow::anyhow!("split did not finish"))?;
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].lower, None);
        assert_eq!(children[1].upper, None);
        assert!(children[0].upper.is_some());
        assert_eq!(children[0].upper, children[1].lower);
        let tx = db.begin().await?;
        let repeated = split_on(&tx, namespace, request, whole, 2, limits, split_cursor).await?;
        match repeated {
            protocol::WorkOutcome::Ready(result) => assert_eq!(result, children),
            _ => panic!("split retry not stable"),
        }
        tx.commit().await?;
        Ok(())
    }

    #[tokio::test]
    async fn release_request_is_idempotent_for_unknown_and_active_lease() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        causal_schema::install_fresh(&db).await?;
        let namespace = Uuid::new_v4();
        let request = Uuid::new_v4();
        CausalReplica::<Model>::create_empty(db.clone(), namespace, Uuid::new_v4()).await?;
        let tx = db.begin().await?;
        release_request_on(&tx, namespace, request).await?;
        crate::causal_leases::create_build_on(
            &tx,
            crate::causal_leases::BuildLeaseRequest {
                namespace,
                id: request,
                scope: vec![1],
                now_ms: 1,
                expires_at_ms: 10,
                max_active: 2,
                max_ttl_ms: 20,
            },
        )
        .await?;
        release_request_on(&tx, namespace, request).await?;
        release_request_on(&tx, namespace, request).await?;
        let lease = crate::causal_leases::load_on(&tx, namespace, request)
            .await?
            .unwrap();
        assert_eq!(lease.status, 2);
        assert!(lease.retained_root.is_none());
        tx.commit().await?;
        Ok(())
    }
    #[tokio::test]
    async fn candidate_batches_share_captured_values_and_respect_each_read_budget() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        causal_schema::install_fresh(&db).await?;
        let namespace = Uuid::new_v4();
        let replica =
            CausalReplica::<Model>::create_empty(db.clone(), namespace, Uuid::new_v4()).await?;
        for id in 1..=8 {
            let tx = db.begin().await?;
            replica
                .write_observed_on(
                    &tx,
                    ObjectKey {
                        collection: Collection::Notes,
                        id: Uuid::from_u128(id),
                    },
                    Some(Note {
                        text: "captured".into(),
                    }),
                    Observed::AllCurrent,
                )
                .await?;
            tx.commit().await?;
        }
        let state_seq = 8;
        let range = KeyRange::<Model> {
            collection: Collection::Notes,
            lower: None,
            upper: None,
        };
        let mut limits = WorkLimits {
            max_requests: 1,
            max_bytes: 65536,
            max_depth: 1,
            max_rows_scanned: 64,
            max_mutations: 2,
        };
        let tx = db.begin().await?;
        let mut header = CandidateCount::default();
        let mut header_limits = limits;
        header_limits.max_bytes = 128;
        let usage = scan_candidate_batch_on(
            &tx,
            namespace,
            &range,
            state_seq,
            &mut header,
            header_limits,
            |_, _| anyhow::bail!("header budget read a body"),
        )
        .await?;
        assert_eq!((usage.rows, usage.bytes, header.count), (1, 128, 0));
        let body_budget = header.pending.as_ref().unwrap().body_len + 96;
        tx.commit().await?;
        // Later replacement must not enter any of the retained scans.
        let tx = db.begin().await?;
        replica
            .write_observed_on(
                &tx,
                ObjectKey {
                    collection: Collection::Notes,
                    id: Uuid::from_u128(1),
                },
                Some(Note {
                    text: "later".into(),
                }),
                Observed::AllCurrent,
            )
            .await?;
        tx.commit().await?;
        let mut results = Vec::new();
        let mut calls = Vec::new();
        let mut totals = Vec::new();
        for (rows, bytes) in [(1, 65536), (64, 65536), (64, body_budget)] {
            limits.max_rows_scanned = rows;
            limits.max_bytes = bytes;
            let mut scan = CandidateCount::default();
            let mut values = Vec::new();
            let mut count = 0;
            while !scan.done {
                let tx = db.begin().await?;
                let used = scan_candidate_batch_on(
                    &tx,
                    namespace,
                    &range,
                    state_seq,
                    &mut scan,
                    limits,
                    |candidate, _| {
                        assert_eq!(candidate.payload.text, "captured");
                        values.push(protocol::candidate_digest(candidate)?);
                        Ok(true)
                    },
                )
                .await?;
                assert!(used.rows > 0 && used.rows <= rows && used.bytes <= bytes);
                tx.commit().await?;
                scan = serde_json::from_slice(&serde_json::to_vec(&scan)?)?;
                count += 1;
                assert!(count <= 17);
            }
            assert_eq!(scan.count, 8);
            results.push(values);
            calls.push(count);
            totals.push(scan.bytes);
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(results[1], results[2]);
        assert_eq!(calls[0], 17);
        assert_eq!(calls[1], 1);
        assert!(calls[2] > 1);
        // A callback may stop after an early item even when the query fetched
        // later rows. Only the consumed prefix is checkpointed; all fetched
        // rows and bytes remain charged to this step.
        let mut stopped = CandidateCount::default();
        let mut prefix = Vec::new();
        let tx = db.begin().await?;
        let used = scan_candidate_batch_on(
            &tx,
            namespace,
            &range,
            state_seq,
            &mut stopped,
            WorkLimits {
                max_rows_scanned: 64,
                max_bytes: 65536,
                ..limits
            },
            |candidate, _| {
                prefix.push(protocol::candidate_digest(candidate)?);
                Ok(prefix.len() < 2)
            },
        )
        .await?;
        assert_eq!(prefix, results[0][..2]);
        assert_eq!(stopped.count, 2);
        assert!(used.rows >= 16 && used.rows <= 64 && used.bytes <= 65536);
        tx.commit().await?;
        stopped = serde_json::from_slice(&serde_json::to_vec(&stopped)?)?;
        let mut suffix = Vec::new();
        while !stopped.done {
            let tx = db.begin().await?;
            let used = scan_candidate_batch_on(
                &tx,
                namespace,
                &range,
                state_seq,
                &mut stopped,
                WorkLimits {
                    max_rows_scanned: 64,
                    max_bytes: 65536,
                    ..limits
                },
                |candidate, _| {
                    suffix.push(protocol::candidate_digest(candidate)?);
                    Ok(true)
                },
            )
            .await?;
            assert!(used.rows <= 64 && used.bytes <= 65536);
            tx.commit().await?;
            stopped = serde_json::from_slice(&serde_json::to_vec(&stopped)?)?;
        }
        assert_eq!(suffix, results[0][2..]);
        // A body enlarged after its header was observed must not be loaded by
        // the bulk query, and a scan of the corrupt row must reject it.
        let tx = db.begin().await?;
        let headers = crate::causal_candidates::header_batch_on(
            &tx,
            crate::causal_candidates::CandidatePageRequest {
                namespace,
                collection: b"notes",
                state_seq,
                lower: None,
                upper: None,
                after: None,
                max_items: 1,
                max_bytes: 65536,
            },
        )
        .await?;
        assert_eq!(headers.len(), 1);
        let change = Query::update()
            .table(Alias::new("sync_candidates"))
            .value(Alias::new("body"), vec![0u8; 65537])
            .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("collection")).eq(b"notes".to_vec()))
            .and_where(Expr::col(Alias::new("writer")).eq(headers[0].writer.clone()))
            .and_where(Expr::col(Alias::new("counter")).eq(headers[0].counter))
            .to_owned();
        tx.execute(&change).await?;
        assert!(
            crate::causal_candidates::body_batch_on(&tx, namespace, b"notes", &headers)
                .await?
                .is_empty()
        );
        let mut corrupt = CandidateCount::default();
        assert!(
            scan_candidate_batch_on(
                &tx,
                namespace,
                &range,
                state_seq,
                &mut corrupt,
                WorkLimits {
                    max_rows_scanned: 64,
                    max_bytes: 65536,
                    ..limits
                },
                |_, _| Ok(true)
            )
            .await
            .is_err()
        );
        tx.rollback().await?;
        let scope = protocol::Scope::<Model> {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let mut hashes = Vec::new();
        for rows in [1, 64] {
            limits.max_rows_scanned = rows;
            limits.max_bytes = 65536;
            let hasher = protocol::RangeHasher::new(&scope, range.clone(), 8, totals[0])?;
            let mut scan = CandidateHash::new(&hasher)?;
            let mut steps = 0;
            while !scan.done() {
                let tx = db.begin().await?;
                hash_candidate_step_on(
                    &tx, namespace, &scope, &range, state_seq, &mut scan, limits,
                )
                .await?;
                tx.commit().await?;
                scan = serde_json::from_slice(&serde_json::to_vec(&scan)?)?;
                steps += 1;
            }
            assert_eq!(steps, if rows == 1 { 17 } else { 1 });
            hashes.push(scan.finish(&scope, &range)?);
        }
        assert_eq!(hashes[0], hashes[1]);
        Ok(())
    }

    #[tokio::test]
    async fn count_pass_resumes_one_row_at_a_time_at_captured_state() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        causal_schema::install_fresh(&db).await?;
        let namespace = Uuid::new_v4();
        let actor = Uuid::new_v4();
        let replica = CausalReplica::<Model>::create_empty(db.clone(), namespace, actor).await?;
        let keys = [Uuid::new_v4(), Uuid::new_v4()];
        for id in keys {
            let tx = db.begin().await?;
            replica
                .write_observed_on(
                    &tx,
                    ObjectKey {
                        collection: Collection::Notes,
                        id,
                    },
                    Some(Note { text: "old".into() }),
                    Observed::AllCurrent,
                )
                .await?;
            tx.commit().await?;
        }
        let tx = db.begin().await?;
        let row = tx
            .query_one(
                &Query::select()
                    .column(Alias::new("state_seq"))
                    .from(Alias::new("sync_meta"))
                    .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
                    .to_owned(),
            )
            .await?
            .unwrap();
        let state_seq: i64 = row.try_get_by_index(0)?;
        tx.commit().await?;
        let range = KeyRange::<Model> {
            collection: Collection::Notes,
            lower: None,
            upper: None,
        };
        let limits = WorkLimits {
            max_requests: 1,
            max_bytes: 1024 * 1024,
            max_depth: 1,
            max_rows_scanned: 1,
            max_mutations: 2,
        };
        let mut state = CandidateCount::default();
        let mut steps = 0;
        while !state.done {
            let tx = db.begin().await?;
            let _ = count_candidate_step_on(&tx, namespace, &range, state_seq, &mut state, limits)
                .await?;
            tx.commit().await?;
            state = serde_json::from_slice(&serde_json::to_vec(&state)?)?;
            steps += 1;
            if steps == 1 {
                let tx = db.begin().await?;
                replica
                    .write_observed_on(
                        &tx,
                        ObjectKey {
                            collection: Collection::Notes,
                            id: keys[0],
                        },
                        Some(Note { text: "new".into() }),
                        Observed::AllCurrent,
                    )
                    .await?;
                tx.commit().await?;
            }
            assert!(steps < 12);
        }
        assert_eq!(state.count, 2);
        assert_eq!(steps, 5);
        assert!(state.bytes > 0);
        let scope = protocol::Scope::<Model> {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let hasher =
            protocol::RangeHasher::<Model>::new(&scope, range.clone(), state.count, state.bytes)?;
        let mut direct = hasher.clone();
        let tx = db.begin().await?;
        let rows = crate::causal_candidates::page_on(
            &tx,
            crate::causal_candidates::CandidatePageRequest {
                namespace,
                collection: b"notes",
                state_seq,
                lower: None,
                upper: None,
                after: None,
                max_items: 128,
                max_bytes: 1024 * 1024,
            },
        )
        .await?;
        for row in &rows.rows {
            direct.push_candidate(&protocol::decode_candidate::<Model>(&row.body)?)?;
        }
        tx.commit().await?;
        let expected = direct.finish()?;
        let mut hash_state = CandidateHash::new(&hasher)?;
        while !hash_state.done() {
            let tx = db.begin().await?;
            hash_candidate_step_on(
                &tx,
                namespace,
                &scope,
                &range,
                state_seq,
                &mut hash_state,
                limits,
            )
            .await?;
            tx.commit().await?;
            hash_state = serde_json::from_slice(&serde_json::to_vec(&hash_state)?)?;
        }
        assert_eq!(hash_state.finish::<Model>(&scope, &range)?, expected);
        Ok(())
    }
    #[tokio::test]
    async fn source_root_rejects_missing_and_extra_persisted_dependencies() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        causal_schema::install_fresh(&db).await?;
        let ns = Uuid::new_v4();
        let replica = CausalReplica::<Model>::create_empty(db.clone(), ns, Uuid::new_v4()).await?;
        let tx = db.begin().await?;
        replica.lock(&tx).await?;
        let scope = protocol::Scope {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let limits = WorkLimits {
            max_requests: 128,
            max_bytes: 1024 * 1024,
            max_depth: 32,
            max_rows_scanned: 128,
            max_mutations: 1024,
        };
        let snapshot = crate::causal_snapshot::open_bounded_on::<Model>(
            &tx,
            replica.identity_on(&tx).await?,
            scope,
            1,
            100,
            limits,
        )
        .await?;
        let lease = crate::causal_leases::load_on(&tx, ns, snapshot.snapshot_id)
            .await?
            .unwrap();
        let root = lease.retained_root.unwrap();
        tx.execute(
            &Query::delete()
                .from_table(Alias::new("sync_manifest_edges"))
                .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                .and_where(Expr::col(Alias::new("parent_manifest")).eq(root.clone()))
                .to_owned(),
        )
        .await?;
        assert!(
            crate::causal_snapshot::descriptor_on::<Model>(&tx, ns, snapshot.snapshot_id, 2)
                .await
                .is_err()
        );
        for child in [
            snapshot.contexts[0]
                .context
                .manifest
                .manifest_digest
                .to_vec(),
            vec![99; 32],
        ] {
            tx.execute(
                &Query::insert()
                    .into_table(Alias::new("sync_manifest_edges"))
                    .columns(["namespace", "parent_manifest", "child_manifest"].map(Alias::new))
                    .values_panic([
                        ns.as_bytes().to_vec().into(),
                        root.clone().into(),
                        child.into(),
                    ])
                    .to_owned(),
            )
            .await?;
        }
        assert!(
            crate::causal_snapshot::descriptor_on::<Model>(&tx, ns, snapshot.snapshot_id, 2)
                .await
                .is_err()
        );
        tx.rollback().await?;
        Ok(())
    }

    #[tokio::test]
    async fn ready_generation_is_pinned_across_connections_until_revoked() -> Result<()> {
        let path = std::env::temp_dir().join(format!("observed-pin-{}.sqlite", Uuid::new_v4()));
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let a = Database::connect(&url).await?;
        causal_schema::install_fresh(&a).await?;
        let b = Database::connect(&url).await?;
        let ns = Uuid::new_v4();
        let replica = CausalReplica::<Model>::create_empty(a.clone(), ns, Uuid::new_v4()).await?;
        let key = ObjectKey {
            collection: Collection::Notes,
            id: Uuid::from_u128(1),
        };
        let tx = a.begin().await?;
        replica
            .author_on(
                &tx,
                key.clone(),
                Some(Note {
                    text: "captured".into(),
                }),
            )
            .await?;
        tx.commit().await?;
        let tx = a.begin().await?;
        replica.lock(&tx).await?;
        let limits = WorkLimits {
            max_requests: 128,
            max_bytes: 1024 * 1024,
            max_depth: 32,
            max_rows_scanned: 128,
            max_mutations: 1024,
        };
        let snapshot = crate::causal_snapshot::open_bounded_on::<Model>(
            &tx,
            replica.identity_on(&tx).await?,
            protocol::Scope {
                collections: [Collection::Notes].into_iter().collect(),
            },
            1,
            100,
            limits,
        )
        .await?;
        tx.commit().await?;
        let tx = b.begin().await?;
        replica
            .author_on(
                &tx,
                key,
                Some(Note {
                    text: "later".into(),
                }),
            )
            .await?;
        tx.commit().await?;
        let tx = b.begin().await?;
        replica.lock(&tx).await?;
        crate::causal_compaction::reclaim_rows_step_on(&tx, ns, 100).await?;
        assert_eq!(
            crate::causal_observed::page_at_on(&tx, ns, b"notes", 1, None, 100).await?,
            vec![(replica.writer_incarnation(), 1, 1)]
        );
        let leaf = crate::causal_snapshot::leaf_on::<Model>(
            &tx,
            ns,
            snapshot.snapshot_id,
            2,
            snapshot.roots[0].range.clone(),
            None,
            protocol::PageLimits {
                max_scanned: 10,
                max_bytes: 65536,
            },
        )
        .await?;
        assert_eq!(leaf.candidates[0].payload.text, "captured");
        crate::causal_leases::revoke_on(&tx, ns, snapshot.snapshot_id).await?;
        crate::causal_compaction::reclaim_rows_step_on(&tx, ns, 100).await?;
        assert_eq!(
            crate::causal_observed::page_at_on(&tx, ns, b"notes", 2, None, 100).await?,
            vec![(replica.writer_incarnation(), 1, 2)]
        );
        assert!(
            crate::causal_snapshot::descriptor_on::<Model>(&tx, ns, snapshot.snapshot_id, 2)
                .await
                .is_err()
        );
        tx.commit().await?;
        drop(replica);
        a.close().await?;
        b.close().await?;
        std::fs::remove_file(path)?;
        Ok(())
    }
    #[tokio::test]
    async fn context_manifests_stay_pinned_between_snapshot_build_steps() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        causal_schema::install_fresh(&db).await?;
        let ns = Uuid::new_v4();
        let replica = CausalReplica::<Model>::create_empty(db.clone(), ns, Uuid::new_v4()).await?;
        let source = replica.identity_on(&db).await?;
        let request = Uuid::new_v4();
        let scope = protocol::Scope::<Model> {
            collections: Model::collections().into_iter().collect(),
        };
        let limits = WorkLimits {
            max_requests: 128,
            max_bytes: 1024 * 1024,
            max_depth: 32,
            max_rows_scanned: 128,
            max_mutations: 128,
        };
        let mut cursor = None;
        for _ in 0..128 {
            let tx = db.begin().await?;
            let outcome = open_on::<Model>(
                &tx,
                source.clone(),
                request,
                scope.clone(),
                SnapshotTiming {
                    now_ms: 1,
                    ttl_ms: 100,
                },
                limits,
                cursor,
            )
            .await?;
            let lease = crate::causal_leases::load_on(&tx, ns, request)
                .await?
                .unwrap();
            let work: SnapshotWork<Model> =
                serde_json::from_slice(lease.build_state.as_deref().unwrap())?;
            tx.commit().await?;
            let tx = db.begin().await?;
            crate::causal_leases::lock_on(&tx, ns).await?;
            for context in &work.contexts {
                assert!(
                    !crate::causal_gc::collect_one_on(
                        &tx,
                        ns,
                        context.context.manifest.manifest_digest,
                        128,
                        65536
                    )
                    .await?,
                    "active build lost a context manifest"
                );
            }
            tx.commit().await?;
            match outcome {
                protocol::WorkOutcome::Incomplete { cursor: next } => cursor = Some(next),
                protocol::WorkOutcome::Ready(snapshot) => {
                    assert_eq!(snapshot.contexts.len(), 3);
                    return Ok(());
                }
            }
        }
        anyhow::bail!("snapshot did not finish")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum SnapshotPhase {
    Context,
    Count,
    Hash,
    Publish,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub(crate) struct SnapshotWork<M: SyncModel> {
    kind: String,
    version: u32,
    source: protocol::StoreIdentity,
    scope: protocol::Scope<M>,
    limits: WorkLimits,
    #[serde(skip)]
    transient_limits: Option<WorkLimits>,
    #[serde(skip)]
    transient_lease_bytes: u64,
    ttl_ms: i64,
    cursor: protocol::WorkCursor,
    collection_index: u32,
    phase: SnapshotPhase,
    context: protocol::ExactContext,
    after: Option<(Uuid, u64)>,
    contexts: Vec<protocol::CollectionContext<M>>,
    count: CandidateCount,
    hash: Option<CandidateHash>,
    roots: Vec<protocol::RangeSummary<M>>,
}
impl<M: SyncModel> SnapshotWork<M> {
    fn work_limits(&self) -> WorkLimits {
        self.transient_limits.unwrap_or(self.limits)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct SnapshotTiming {
    pub now_ms: i64,
    pub ttl_ms: i64,
}

pub(crate) enum SnapshotAdmission<M: SyncModel> {
    Building(Box<SnapshotWork<M>>),
    Ready(Box<protocol::SnapshotDescriptor<M>>),
}

/// Admit or recover the same caller-identified snapshot build. This operation
/// only establishes its captured basis and initial checkpoint; it never claims
/// that the source snapshot has been completed.
pub(crate) async fn admit_snapshot_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    source: protocol::StoreIdentity,
    request_id: Uuid,
    scope: protocol::Scope<M>,
    timing: SnapshotTiming,
    limits: WorkLimits,
    resume: Option<protocol::WorkCursor>,
) -> Result<SnapshotAdmission<M>> {
    let SnapshotTiming { now_ms, ttl_ms } = timing;
    use protocol::SyncError;
    protocol::validate_model::<M>()?;
    protocol::scope_id(&scope)?;
    ensure!(
        !request_id.is_nil()
            && !source.namespace.is_nil()
            && now_ms >= 0
            && ttl_ms > 0
            && ttl_ms <= protocol::MAX_SNAPSHOT_TTL_MS as i64
            && limits.max_rows_scanned > 0
            && limits.max_rows_scanned <= 1024
            && limits.max_mutations > 0
            && limits.max_mutations <= protocol::MAX_MUTATIONS
            && limits.max_bytes > 0
            && limits.max_bytes <= protocol::DEFAULT_LOGICAL_BYTES
            && limits.max_requests > 0
            && limits.max_requests <= protocol::MAX_ROUND_REQUESTS
            && limits.max_depth > 0
            && limits.max_depth <= protocol::MAX_DEPTH
            && source.model.model == M::model_id()
            && source.model.schema == M::schema_id(),
        "invalid snapshot admission"
    );
    let namespace = source.namespace;
    crate::causal_leases::lock_on(tx, namespace).await?;
    let mut existing = crate::causal_leases::load_on(tx, namespace, request_id).await?;
    if existing.is_none() {
        crate::causal_leases::lock_on(tx, namespace).await?;
        existing = crate::causal_leases::load_on(tx, namespace, request_id).await?;
    }
    if let Some(lease) = existing {
        if lease.status == 2 || lease.expires_at_ms <= now_ms {
            return Err(SyncError::SnapshotExpired.into());
        }
        ensure!(
            lease.generation == source.generation.as_bytes(),
            "snapshot generation changed"
        );
        if lease.scope != serde_json::to_vec(&scope)? {
            return Err(SyncError::Invalid("snapshot scope changed".into()).into());
        }
        let mut state: SnapshotWork<M> =
            serde_json::from_slice(lease.build_state.as_deref().ok_or(SyncError::StaleWork)?)?;
        state.transient_lease_bytes = u64::try_from(lease.scope.len())?
            .checked_add(u64::try_from(
                lease.descriptor.as_ref().map_or(0, Vec::len),
            )?)
            .ok_or(SyncError::LogicalLimit)?;
        if state.kind != "snapshot_open"
            || state.version != 2
            || state.source != source
            || state.scope != scope
            || state.limits != limits
            || state.ttl_ms != ttl_ms
            || state.cursor.operation_id != request_id
        {
            return Err(SyncError::Invalid("snapshot request changed".into()).into());
        }
        if let Some(cursor) = &resume
            && (cursor.operation_id != request_id
                || cursor.generation != state.cursor.generation
                || cursor.position > state.cursor.position)
        {
            return Err(SyncError::StaleWork.into());
        }
        if lease.status == 1 {
            let descriptor: protocol::SnapshotDescriptor<M> =
                serde_json::from_slice(lease.descriptor.as_deref().ok_or(SyncError::StaleWork)?)?;
            ensure!(
                descriptor.source == source
                    && descriptor.scope == scope
                    && descriptor.snapshot_id == request_id,
                "snapshot descriptor changed"
            );
            return Ok(SnapshotAdmission::Ready(Box::new(descriptor)));
        }
        ensure!(lease.status == 0, "invalid snapshot status");
        return Ok(SnapshotAdmission::Building(Box::new(state)));
    }
    if resume.is_some() {
        return Err(SyncError::StaleWork.into());
    }
    if limits.max_mutations < 5 {
        return Err(SyncError::LimitTooSmall { required: 5 }.into());
    }
    let expiry = now_ms
        .checked_add(ttl_ms)
        .ok_or_else(|| anyhow::anyhow!("snapshot expiry overflow"))?;
    let lease = crate::causal_leases::create_build_on(
        tx,
        crate::causal_leases::BuildLeaseRequest {
            namespace,
            id: request_id,
            scope: serde_json::to_vec(&scope)?,
            now_ms,
            expires_at_ms: expiry,
            max_active: 16,
            max_ttl_ms: protocol::MAX_SNAPSHOT_TTL_MS as i64,
        },
    )
    .await?;
    ensure!(
        lease.generation == source.generation.as_bytes(),
        "snapshot generation changed"
    );
    let q = Query::select()
        .columns(["replica", "model", "schema"].map(Alias::new))
        .from(Alias::new("sync_meta"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .to_owned();
    let meta = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing namespace"))?;
    ensure!(
        meta.try_get_by_index::<Vec<u8>>(0)? == source.replica.as_bytes()
            && meta.try_get_by_index::<String>(1)? == M::model_id()
            && meta.try_get_by_index::<String>(2)? == M::schema_id(),
        "snapshot source changed"
    );
    let scope_is_empty = scope.collections.is_empty();
    let state = SnapshotWork {
        kind: "snapshot_open".into(),
        version: 2,
        source,
        scope,
        limits,
        transient_limits: None,
        transient_lease_bytes: u64::try_from(lease.scope.len())?,
        ttl_ms,
        cursor: protocol::WorkCursor {
            operation_id: request_id,
            generation: Uuid::new_v4(),
            position: 0,
        },
        collection_index: 0,
        phase: if scope_is_empty {
            SnapshotPhase::Publish
        } else {
            SnapshotPhase::Context
        },
        context: protocol::ExactContext { writers: vec![] },
        after: None,
        contexts: vec![],
        count: CandidateCount::default(),
        hash: None,
        roots: vec![],
    };
    let encoded = serde_json::to_vec(&state)?;
    let required = u64::try_from(encoded.len())?
        .checked_add(u64::try_from(lease.scope.len())?)
        .ok_or(SyncError::LogicalLimit)?;
    if required > limits.max_bytes {
        return Err(SyncError::LimitTooSmall { required }.into());
    }
    crate::causal_leases::checkpoint_on(tx, namespace, request_id, None, encoded, now_ms).await?;
    Ok(SnapshotAdmission::Building(Box::new(state)))
}

/// Original request limits and the conservative allowance for this transaction.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SnapshotStepLimits {
    pub full: WorkLimits,
    pub admitted: WorkLimits,
}

/// A continuation allowance bound to the exact checkpoint that produced it.
#[derive(Clone, Debug)]
pub(crate) struct SnapshotStepPlan {
    pub admitted: WorkLimits,
    pub expected: protocol::WorkCursor,
}

#[derive(Clone, Debug)]
pub(crate) struct SnapshotBatchLimits {
    pub full: WorkLimits,
    pub plan: Option<SnapshotStepPlan>,
}

pub(crate) struct SnapshotBatchResult<M: SyncModel> {
    pub outcome: protocol::WorkOutcome<protocol::SnapshotDescriptor<M>>,
    pub charged: WorkLimits,
    pub next: Option<SnapshotStepPlan>,
}

fn plan_snapshot_step<M: SyncModel>(
    state: &SnapshotWork<M>,
    full_limits: WorkLimits,
) -> Result<SnapshotStepPlan> {
    let fixed = match state.phase {
        SnapshotPhase::Publish => true,
        SnapshotPhase::Context => state.after.is_none(),
        SnapshotPhase::Count => state.count.count == 0,
        SnapshotPhase::Hash => state.count.count == 0,
    };
    // A context step can add five intervals and a new manifest reference before checkpointing.
    let context_growth = if matches!(state.phase, SnapshotPhase::Context) {
        4096
    } else {
        0
    };
    let bytes = (serde_json::to_vec(state)?.len() as u64)
        .checked_mul(2)
        .and_then(|n| n.checked_add(state.transient_lease_bytes + 16_384 + context_growth))
        .ok_or(protocol::SyncError::LogicalLimit)?;
    let mutations = match state.phase {
        SnapshotPhase::Context => state.contexts.len() as u64 + bytes.div_ceil(64 * 1024) + 20,
        SnapshotPhase::Publish => state.contexts.len() as u64 + bytes.div_ceil(64 * 1024) + 16,
        _ => 16,
    };
    let admitted = if fixed
        && full_limits.max_rows_scanned >= 5
        && full_limits.max_mutations >= mutations
        && full_limits.max_bytes >= bytes
    {
        WorkLimits {
            max_rows_scanned: 5,
            max_mutations: mutations,
            max_bytes: bytes,
            ..full_limits
        }
    } else {
        full_limits
    };
    Ok(SnapshotStepPlan {
        admitted,
        expected: state.cursor.clone(),
    })
}
/// Advance a caller-identified source snapshot under one bounded transaction.
/// The caller commits every successful result before returning it remotely.
pub async fn open_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    source: protocol::StoreIdentity,
    request_id: Uuid,
    scope: protocol::Scope<M>,
    timing: SnapshotTiming,
    limits: WorkLimits,
    resume: Option<protocol::WorkCursor>,
) -> Result<protocol::WorkOutcome<protocol::SnapshotDescriptor<M>>> {
    open_step_on(
        tx,
        source,
        request_id,
        scope,
        timing,
        SnapshotStepLimits {
            full: limits,
            admitted: limits,
        },
        resume,
    )
    .await
}

/// Execute exactly one continuation with a transient allowance. The original
/// request limits remain in the durable checkpoint and bind all retries.
pub(crate) async fn open_step_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    source: protocol::StoreIdentity,
    request_id: Uuid,
    scope: protocol::Scope<M>,
    timing: SnapshotTiming,
    limits: SnapshotStepLimits,
    resume: Option<protocol::WorkCursor>,
) -> Result<protocol::WorkOutcome<protocol::SnapshotDescriptor<M>>> {
    let SnapshotStepLimits { full, admitted } = limits;
    ensure!(
        admitted.max_rows_scanned > 0
            && admitted.max_rows_scanned <= full.max_rows_scanned
            && admitted.max_mutations > 0
            && admitted.max_mutations <= full.max_mutations
            && admitted.max_bytes > 0
            && admitted.max_bytes <= full.max_bytes
            && admitted.max_requests == full.max_requests
            && admitted.max_depth == full.max_depth,
        "invalid transient snapshot allowance"
    );
    let SnapshotTiming { now_ms, ttl_ms } = timing;
    crate::causal_leases::lock_on(tx, source.namespace).await?;
    let existed = crate::causal_leases::load_on(tx, source.namespace, request_id)
        .await?
        .is_some();
    let mut state = match admit_snapshot_on(
        tx,
        source,
        request_id,
        scope,
        crate::causal_source::SnapshotTiming { now_ms, ttl_ms },
        full,
        resume,
    )
    .await?
    {
        SnapshotAdmission::Ready(descriptor) => {
            return Ok(protocol::WorkOutcome::Ready(*descriptor));
        }
        SnapshotAdmission::Building(state) => state,
    };
    if !existed {
        ensure!(
            admitted == full,
            "new snapshot requires its full admission allowance"
        );
        return Ok(protocol::WorkOutcome::Incomplete {
            cursor: state.cursor,
        });
    }
    state.transient_limits = Some(admitted);
    match advance_snapshot_step_on(tx, &mut state, now_ms).await? {
        Some(descriptor) => Ok(protocol::WorkOutcome::Ready(descriptor)),
        None => Ok(protocol::WorkOutcome::Incomplete {
            cursor: state.cursor.clone(),
        }),
    }
}

/// Advance exactly one transaction and return an in-memory successor plan.
/// The caller reserves the supplied plan before starting this transaction.
pub(crate) async fn open_batch_step_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    source: protocol::StoreIdentity,
    request_id: Uuid,
    scope: protocol::Scope<M>,
    timing: SnapshotTiming,
    limits: SnapshotBatchLimits,
    resume: Option<protocol::WorkCursor>,
) -> Result<SnapshotBatchResult<M>> {
    use protocol::SyncError;
    let SnapshotBatchLimits { full, plan } = limits;
    crate::causal_leases::lock_on(tx, source.namespace).await?;
    let existed = crate::causal_leases::load_on(tx, source.namespace, request_id)
        .await?
        .is_some();
    let mut state =
        match admit_snapshot_on(tx, source, request_id, scope, timing, full, resume).await? {
            SnapshotAdmission::Ready(descriptor) => {
                if plan.is_some() {
                    return Err(SyncError::StaleWork.into());
                }
                return Ok(SnapshotBatchResult {
                    outcome: protocol::WorkOutcome::Ready(*descriptor),
                    charged: full,
                    next: None,
                });
            }
            SnapshotAdmission::Building(state) => state,
        };
    if !existed {
        if plan.is_some() {
            return Err(SyncError::StaleWork.into());
        }
        let next = plan_snapshot_step(&state, full)?;
        return Ok(SnapshotBatchResult {
            outcome: protocol::WorkOutcome::Incomplete {
                cursor: state.cursor.clone(),
            },
            charged: full,
            next: Some(next),
        });
    }
    let expected = plan_snapshot_step(&state, full)?;
    let admitted = match plan {
        Some(plan) if plan.expected == expected.expected && plan.admitted == expected.admitted => {
            plan.admitted
        }
        Some(_) => return Err(SyncError::StaleWork.into()),
        None => expected.admitted,
    };
    state.transient_limits = Some(admitted);
    match advance_snapshot_step_on(tx, &mut state, timing.now_ms).await? {
        Some(descriptor) => Ok(SnapshotBatchResult {
            outcome: protocol::WorkOutcome::Ready(descriptor),
            charged: admitted,
            next: None,
        }),
        None => {
            let next = plan_snapshot_step(&state, full)?;
            Ok(SnapshotBatchResult {
                outcome: protocol::WorkOutcome::Incomplete {
                    cursor: state.cursor.clone(),
                },
                charged: admitted,
                next: Some(next),
            })
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum RangeWorkPhase {
    CountCandidates,
    HashCandidates,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
struct RangeWork<M: SyncModel> {
    kind: String,
    version: u32,
    snapshot: Uuid,
    range: KeyRange<M>,
    limits: WorkLimits,
    cursor: protocol::WorkCursor,
    phase: RangeWorkPhase,
    candidates: CandidateCount,
    candidate_hash: Option<CandidateHash>,
}

/// Incrementally summarize a pinned ready range. A deterministic child lease
/// keeps the exact parent publication position and stores only bounded work
/// state; it never copies candidate payloads into another relation.
pub async fn summarize_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    snapshot_id: Uuid,
    range: KeyRange<M>,
    now_ms: i64,
    limits: WorkLimits,
    resume: Option<protocol::WorkCursor>,
) -> Result<protocol::WorkOutcome<protocol::RangeSummary<M>>> {
    use protocol::SyncError;
    ensure!(
        limits.max_requests > 0
            && limits.max_rows_scanned > 0
            && limits.max_mutations > 0
            && limits.max_bytes > 0
            && limits.max_depth > 0,
        "invalid summary limits"
    );
    crate::causal_leases::lock_on(tx, namespace).await?;
    let parent = crate::causal_leases::load_on(tx, namespace, snapshot_id)
        .await?
        .ok_or(SyncError::SnapshotExpired)?;
    if parent.status != 1 || parent.expires_at_ms <= now_ms {
        return Err(SyncError::SnapshotExpired.into());
    }
    let descriptor: protocol::SnapshotDescriptor<M> = serde_json::from_slice(
        parent
            .descriptor
            .as_deref()
            .ok_or(SyncError::SnapshotExpired)?,
    )?;

    ensure!(
        descriptor.source.namespace == namespace
            && descriptor.snapshot_id == snapshot_id
            && descriptor.scope.collections.contains(&range.collection)
            && range.lower.zip(range.upper).is_none_or(|(a, b)| a < b),
        "invalid summary range"
    );
    if let Some(summary) = descriptor
        .roots
        .iter()
        .find(|summary| summary.range == range)
    {
        if resume.is_some() {
            return Err(SyncError::StaleWork.into());
        }
        return Ok(protocol::WorkOutcome::Ready(summary.clone()));
    }
    let id = summary_work_id(snapshot_id, &range)?;
    let existing = crate::causal_leases::load_on(tx, namespace, id).await?;
    let required_mutations = if existing.is_some() { 4 } else { 3 };
    if limits.max_mutations < required_mutations {
        return Err(SyncError::LimitTooSmall {
            required: required_mutations,
        }
        .into());
    }
    crate::causal_leases::lock_on(tx, namespace).await?;
    let parent = crate::causal_leases::load_on(tx, namespace, snapshot_id)
        .await?
        .ok_or(SyncError::SnapshotExpired)?;
    if parent.status != 1 || parent.expires_at_ms <= now_ms {
        return Err(SyncError::SnapshotExpired.into());
    }
    let existing = crate::causal_leases::load_on(tx, namespace, id).await?;
    if let Some(child) = existing {
        if child.status == 2 || child.expires_at_ms <= now_ms {
            return Err(SyncError::StaleWork.into());
        }
        ensure!(
            child.parent_lease.as_deref() == Some(snapshot_id.as_bytes().as_slice())
                && child.generation == parent.generation
                && child.state_seq == parent.state_seq
                && child.head == parent.head
                && child.head_anchor == parent.head_anchor
                && child.scope == parent.scope,
            "summary lease position mismatch"
        );
        let saved = child.build_state.as_deref().ok_or(SyncError::StaleWork)?;
        if let Ok(cache) = serde_json::from_slice::<ReadySummary<M>>(saved) {
            ensure!(
                cache.kind == "summary_ready"
                    && cache.version == 1
                    && cache.snapshot == snapshot_id
                    && cache.range == range
                    && cache.limits == limits
                    && cache.summary.snapshot_id == snapshot_id
                    && cache.summary.range == range,
                "invalid cached summary"
            );
            if resume
                .as_ref()
                .is_some_and(|c| c.operation_id != id || c.generation != cache.cursor_generation)
            {
                return Err(SyncError::StaleWork.into());
            }
            return Ok(protocol::WorkOutcome::Ready(cache.summary));
        }
        let mut work: RangeWork<M> = serde_json::from_slice(saved)?;
        ensure!(
            work.kind == "summary_work"
                && work.version == 1
                && work.snapshot == snapshot_id
                && work.range == range
                && work.limits == limits
                && work.cursor.operation_id == id,
            "summary request changed"
        );
        if resume.as_ref().is_some_and(|c| {
            c.operation_id != id
                || c.generation != work.cursor.generation
                || c.position > work.cursor.position
        }) {
            return Err(SyncError::StaleWork.into());
        }
        let expected = saved.to_vec();
        let checkpoint_bound = u64::try_from(expected.len())?
            .checked_add(16384)
            .ok_or(SyncError::LogicalLimit)?;
        let required_bytes = checkpoint_bound
            .checked_add(8192)
            .ok_or(SyncError::LogicalLimit)?;
        if limits.max_bytes < required_bytes {
            return Err(SyncError::LimitTooSmall {
                required: required_bytes,
            }
            .into());
        }
        let mut data_limits = limits;
        data_limits.max_bytes -= checkpoint_bound;
        data_limits.max_mutations -= 4;
        let mut ready = None;
        match work.phase {
            RangeWorkPhase::CountCandidates => {
                scan_candidate_headers_on(
                    tx,
                    namespace,
                    &range,
                    parent.state_seq,
                    &mut work.candidates,
                    data_limits,
                    |_, _| Ok(true),
                )
                .await?;
                if work.candidates.done {
                    let hash = protocol::RangeHasher::<M>::new(
                        &descriptor.scope,
                        range.clone(),
                        work.candidates.count,
                        work.candidates.bytes,
                    )?;
                    work.candidate_hash = Some(CandidateHash::new(&hash)?);
                    work.phase = RangeWorkPhase::HashCandidates;
                }
            }
            RangeWorkPhase::HashCandidates => {
                let hash = work.candidate_hash.as_mut().ok_or(SyncError::StaleWork)?;
                hash_candidate_step_on(
                    tx,
                    namespace,
                    &descriptor.scope,
                    &range,
                    parent.state_seq,
                    hash,
                    data_limits,
                )
                .await?;
                if hash.done() {
                    let aggregate = hash.clone().finish(&descriptor.scope, &range)?;
                    ready = Some(protocol::RangeSummary {
                        snapshot_id,
                        range: range.clone(),
                        candidate_count: work.candidates.count,
                        candidate_bytes: work.candidates.bytes,
                        aggregate,
                    });
                }
            }
        }
        work.cursor.position = work
            .cursor
            .position
            .checked_add(1)
            .ok_or(SyncError::LogicalLimit)?;
        let next = if let Some(summary) = &ready {
            serde_json::to_vec(&ReadySummary {
                kind: "summary_ready".into(),
                version: 1,
                snapshot: snapshot_id,
                range: range.clone(),
                cursor_generation: work.cursor.generation,
                limits,
                summary: summary.clone(),
            })?
        } else {
            serde_json::to_vec(&work)?
        };
        ensure!(
            u64::try_from(next.len())? <= checkpoint_bound,
            "range summary checkpoint {} exceeds reserved {} bytes",
            next.len(),
            checkpoint_bound
        );
        crate::causal_leases::checkpoint_on(tx, namespace, id, Some(&expected), next, now_ms)
            .await?;
        if ready.is_some() {
            crate::causal_leases::complete_child_on(tx, namespace, id).await?;
        }
        return match ready {
            Some(summary) => Ok(protocol::WorkOutcome::Ready(summary)),
            None => Ok(protocol::WorkOutcome::Incomplete {
                cursor: work.cursor,
            }),
        };
    }
    if resume.is_some() {
        return Err(SyncError::StaleWork.into());
    }
    crate::causal_leases::reserve_child_on(
        tx,
        namespace,
        snapshot_id,
        now_ms,
        limits.max_mutations - 3,
    )
    .await?;
    let work = RangeWork {
        kind: "summary_work".into(),
        version: 1,
        snapshot: snapshot_id,
        range: range.clone(),
        limits,
        cursor: protocol::WorkCursor {
            operation_id: id,
            generation: Uuid::new_v4(),
            position: 0,
        },
        phase: RangeWorkPhase::CountCandidates,
        candidates: CandidateCount::default(),
        candidate_hash: None,
    };
    let encoded = serde_json::to_vec(&work)?;
    let required = u64::try_from(encoded.len())?;
    if limits.max_bytes < required {
        return Err(SyncError::LimitTooSmall { required }.into());
    }
    let q = Query::insert()
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
                "retained_root",
                "parent_lease",
                "build_state",
            ]
            .map(Alias::new),
        )
        .values_panic([
            namespace.as_bytes().to_vec().into(),
            id.as_bytes().to_vec().into(),
            parent.generation.into(),
            parent.head.into(),
            parent.head_anchor.into(),
            parent.state_seq.into(),
            parent.scope.into(),
            parent.expires_at_ms.into(),
            0i64.into(),
            Option::<Vec<u8>>::None.into(),
            snapshot_id.as_bytes().to_vec().into(),
            encoded.into(),
        ])
        .to_owned();
    tx.execute(&q).await?;
    Ok(protocol::WorkOutcome::Incomplete {
        cursor: work.cursor,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum SplitPhase {
    CountCandidates,
    FindCandidate,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
struct SplitWork<M: SyncModel> {
    kind: String,
    version: u32,
    snapshot: Uuid,
    range: KeyRange<M>,
    limits: WorkLimits,
    cursor: protocol::WorkCursor,
    phase: SplitPhase,
    scan: CandidateCount,
    distinct: u64,
    total_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    summary_candidates: Option<u64>,
    selected_bytes: u64,
    last_key: Option<Uuid>,
    fallback: Option<Uuid>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
struct ReadySplit<M: SyncModel> {
    kind: String,
    version: u32,
    snapshot: Uuid,
    range: KeyRange<M>,
    limits: WorkLimits,
    cursor_generation: Uuid,
    result: Vec<KeyRange<M>>,
}
fn split_work_id<M: SyncModel>(snapshot: Uuid, range: &KeyRange<M>) -> Result<Uuid> {
    let encoded = serde_json::to_vec(range)?;
    let mut hash = blake3::Hasher::new();
    hash.update(b"sync.source.split.work.v1\0");
    hash.update(snapshot.as_bytes());
    hash.update(&encoded);
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash.finalize().as_bytes()[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(bytes))
}
fn split_ranges<M: SyncModel>(range: &KeyRange<M>, pivot: Option<Uuid>) -> Vec<KeyRange<M>> {
    match pivot {
        None => vec![range.clone()],
        Some(pivot) => vec![
            KeyRange {
                collection: range.collection.clone(),
                lower: range.lower,
                upper: Some(pivot),
            },
            KeyRange {
                collection: range.collection.clone(),
                lower: Some(pivot),
                upper: range.upper,
            },
        ],
    }
}

/// Choose a stable candidate boundary in a captured view without copying payloads.
pub async fn split_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    snapshot_id: Uuid,
    range: KeyRange<M>,
    now_ms: i64,
    limits: WorkLimits,
    resume: Option<protocol::WorkCursor>,
) -> Result<protocol::WorkOutcome<Vec<KeyRange<M>>>> {
    use protocol::SyncError;
    ensure!(
        limits.max_requests > 0
            && limits.max_rows_scanned > 0
            && limits.max_mutations > 0
            && limits.max_bytes > 0
            && limits.max_depth > 0,
        "invalid split limits"
    );
    crate::causal_leases::lock_on(tx, namespace).await?;
    let parent = crate::causal_leases::load_on(tx, namespace, snapshot_id)
        .await?
        .ok_or(SyncError::SnapshotExpired)?;
    if parent.status != 1 || parent.expires_at_ms <= now_ms {
        return Err(SyncError::SnapshotExpired.into());
    }
    let descriptor: protocol::SnapshotDescriptor<M> = serde_json::from_slice(
        parent
            .descriptor
            .as_deref()
            .ok_or(SyncError::SnapshotExpired)?,
    )?;

    ensure!(
        descriptor.source.namespace == namespace
            && descriptor.snapshot_id == snapshot_id
            && descriptor.scope.collections.contains(&range.collection)
            && range.lower.zip(range.upper).is_none_or(|(a, b)| a < b),
        "invalid split range"
    );
    let id = split_work_id(snapshot_id, &range)?;
    let existing = crate::causal_leases::load_on(tx, namespace, id).await?;
    let required_mutations = if existing.is_some() { 4 } else { 3 };
    if limits.max_mutations < required_mutations {
        return Err(SyncError::LimitTooSmall {
            required: required_mutations,
        }
        .into());
    }
    crate::causal_leases::lock_on(tx, namespace).await?;
    let parent = crate::causal_leases::load_on(tx, namespace, snapshot_id)
        .await?
        .ok_or(SyncError::SnapshotExpired)?;
    if parent.status != 1 || parent.expires_at_ms <= now_ms {
        return Err(SyncError::SnapshotExpired.into());
    }
    if let Some(child) = crate::causal_leases::load_on(tx, namespace, id).await? {
        if child.status == 2 || child.expires_at_ms <= now_ms {
            return Err(SyncError::StaleWork.into());
        }
        ensure!(
            child.parent_lease.as_deref() == Some(snapshot_id.as_bytes().as_slice())
                && child.generation == parent.generation
                && child.state_seq == parent.state_seq
                && child.head == parent.head
                && child.head_anchor == parent.head_anchor
                && child.scope == parent.scope,
            "split lease position mismatch"
        );
        let saved = child.build_state.as_deref().ok_or(SyncError::StaleWork)?;
        if let Ok(cache) = serde_json::from_slice::<ReadySplit<M>>(saved) {
            ensure!(
                cache.kind == "split_ready"
                    && cache.version == 1
                    && cache.snapshot == snapshot_id
                    && cache.range == range
                    && cache.limits == limits,
                "invalid cached split"
            );
            if resume
                .as_ref()
                .is_some_and(|c| c.operation_id != id || c.generation != cache.cursor_generation)
            {
                return Err(SyncError::StaleWork.into());
            }
            return Ok(protocol::WorkOutcome::Ready(cache.result));
        }
        let mut work: SplitWork<M> = serde_json::from_slice(saved)?;
        ensure!(
            work.kind == "split_work"
                && work.version == 1
                && work.snapshot == snapshot_id
                && work.range == range
                && work.limits == limits
                && work.cursor.operation_id == id,
            "split request changed"
        );
        if resume.as_ref().is_some_and(|c| {
            c.operation_id != id
                || c.generation != work.cursor.generation
                || c.position > work.cursor.position
        }) {
            return Err(SyncError::StaleWork.into());
        }
        let expected = saved.to_vec();
        let checkpoint_bound = u64::try_from(expected.len())?
            .checked_add(8192)
            .ok_or(SyncError::LogicalLimit)?;
        let required_bytes = checkpoint_bound
            .checked_add(8192)
            .ok_or(SyncError::LogicalLimit)?;
        if limits.max_bytes < required_bytes {
            return Err(SyncError::LimitTooSmall {
                required: required_bytes,
            }
            .into());
        }
        let mut data_limits = limits;
        data_limits.max_bytes -= checkpoint_bound;
        data_limits.max_mutations -= 4;
        let mut result = None;
        match work.phase {
            SplitPhase::CountCandidates => {
                scan_candidate_headers_on(
                    tx,
                    namespace,
                    &range,
                    parent.state_seq,
                    &mut work.scan,
                    data_limits,
                    |position, _| {
                        if work.last_key != Some(position.object) {
                            work.distinct = work
                                .distinct
                                .checked_add(1)
                                .ok_or(SyncError::LogicalLimit)?;
                        }
                        work.last_key = Some(position.object);
                        Ok(true)
                    },
                )
                .await?;
                if work.scan.done {
                    work.total_bytes = work.scan.bytes;
                    work.fallback = work.last_key.filter(|key| {
                        range.lower.is_none_or(|lower| *key > lower)
                            && range.upper.is_none_or(|upper| *key < upper)
                    });
                    if work.distinct == 0 {
                        result = Some(vec![range.clone()]);
                    }
                    work.phase = SplitPhase::FindCandidate;
                    work.scan = CandidateCount::default();
                    work.last_key = None;
                }
            }
            SplitPhase::FindCandidate => {
                scan_candidate_headers_on(
                    tx,
                    namespace,
                    &range,
                    parent.state_seq,
                    &mut work.scan,
                    data_limits,
                    |position, bytes| {
                        if work.last_key != Some(position.object)
                            && range.lower.is_none_or(|lower| position.object > lower)
                            && work.selected_bytes >= work.total_bytes / 2
                        {
                            result = Some(split_ranges(&range, Some(position.object)));
                        }
                        work.selected_bytes = work
                            .selected_bytes
                            .checked_add(bytes)
                            .ok_or(SyncError::LogicalLimit)?;
                        work.last_key = Some(position.object);
                        Ok(result.is_none())
                    },
                )
                .await?;
                if work.scan.done && result.is_none() {
                    let fallback = work.fallback.or_else(|| {
                        work.last_key.filter(|key| {
                            range.lower.is_none_or(|lower| *key > lower)
                                && range.upper.is_none_or(|upper| *key < upper)
                        })
                    });
                    result = Some(split_ranges(&range, fallback));
                }
            }
        }
        work.cursor.position = work
            .cursor
            .position
            .checked_add(1)
            .ok_or(SyncError::LogicalLimit)?;
        let next = if let Some(result) = &result {
            serde_json::to_vec(&ReadySplit {
                kind: "split_ready".into(),
                version: 1,
                snapshot: snapshot_id,
                range: range.clone(),
                limits,
                cursor_generation: work.cursor.generation,
                result: result.clone(),
            })?
        } else {
            serde_json::to_vec(&work)?
        };
        ensure!(
            u64::try_from(next.len())? <= checkpoint_bound,
            "split checkpoint exceeded bound"
        );
        crate::causal_leases::checkpoint_on(tx, namespace, id, Some(&expected), next, now_ms)
            .await?;
        if result.is_some() {
            crate::causal_leases::complete_child_on(tx, namespace, id).await?;
        }
        return match result {
            Some(result) => Ok(protocol::WorkOutcome::Ready(result)),
            None => Ok(protocol::WorkOutcome::Incomplete {
                cursor: work.cursor,
            }),
        };
    }
    if resume.is_some() {
        return Err(SyncError::StaleWork.into());
    }
    crate::causal_leases::reserve_child_on(
        tx,
        namespace,
        snapshot_id,
        now_ms,
        limits.max_mutations - 3,
    )
    .await?;
    let summary = match summary_ready_on(tx, namespace, snapshot_id, &range, now_ms).await {
        Ok(summary) => Some(summary),
        Err(error)
            if matches!(
                error.downcast_ref::<SyncError>(),
                Some(SyncError::StaleWork)
            ) =>
        {
            None
        }
        Err(error) => return Err(error),
    };
    let work = SplitWork {
        kind: "split_work".into(),
        version: 1,
        snapshot: snapshot_id,
        range: range.clone(),
        limits,
        cursor: protocol::WorkCursor {
            operation_id: id,
            generation: Uuid::new_v4(),
            position: 0,
        },
        phase: if summary.is_some() {
            SplitPhase::FindCandidate
        } else {
            SplitPhase::CountCandidates
        },
        scan: CandidateCount::default(),
        distinct: 0,
        total_bytes: summary.as_ref().map_or(0, |value| value.candidate_bytes),
        summary_candidates: summary.as_ref().map(|value| value.candidate_count),
        selected_bytes: 0,
        last_key: None,
        fallback: None,
    };
    let encoded = serde_json::to_vec(&work)?;
    let required = u64::try_from(encoded.len())?;
    if limits.max_bytes < required {
        return Err(SyncError::LimitTooSmall { required }.into());
    }
    let q = Query::insert()
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
                "retained_root",
                "parent_lease",
                "build_state",
            ]
            .map(Alias::new),
        )
        .values_panic([
            namespace.as_bytes().to_vec().into(),
            id.as_bytes().to_vec().into(),
            parent.generation.into(),
            parent.head.into(),
            parent.head_anchor.into(),
            parent.state_seq.into(),
            parent.scope.into(),
            parent.expires_at_ms.into(),
            0i64.into(),
            Option::<Vec<u8>>::None.into(),
            snapshot_id.as_bytes().to_vec().into(),
            encoded.into(),
        ])
        .to_owned();
    tx.execute(&q).await?;
    Ok(protocol::WorkOutcome::Incomplete {
        cursor: work.cursor,
    })
}

async fn advance_snapshot_step_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &mut SnapshotWork<M>,
    now_ms: i64,
) -> Result<Option<protocol::SnapshotDescriptor<M>>> {
    use protocol::{ManifestKind, SyncError};
    let limits = state.work_limits();
    let ns = state.source.namespace;
    crate::causal_leases::lock_on(tx, ns).await?;
    let lease =
        crate::causal_leases::building_on(tx, ns, state.cursor.operation_id, now_ms).await?;
    let expected = serde_json::to_vec(state)?;
    ensure!(
        lease.build_state.as_deref() == Some(expected.as_slice()),
        SyncError::StaleWork
    );
    let reserve = (expected.len() as u64)
        .checked_add(16_384)
        .ok_or(SyncError::LogicalLimit)?;
    if limits.max_bytes <= reserve || limits.max_mutations < 3 {
        return Err(SyncError::LimitTooSmall {
            required: reserve + 128,
        }
        .into());
    }
    let mut data = limits;
    data.max_bytes -= reserve;
    data.max_mutations -= 3;
    let mut collections: Vec<_> = state.scope.collections.iter().cloned().collect();
    collections.sort_by_key(M::collection_tag);
    if matches!(state.phase, SnapshotPhase::Publish) {
        pin_context_root_on(tx, state, state.scope.clone(), now_ms, data).await?;
        let descriptor = protocol::SnapshotDescriptor {
            source: state.source.clone(),
            scope: state.scope.clone(),
            snapshot_id: state.cursor.operation_id,
            head: protocol::ReceiveProgress {
                sequence: u64::try_from(lease.head)?,
                anchor: lease
                    .head_anchor
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("invalid snapshot anchor"))?,
            },
            roots: state.roots.clone(),
            contexts: state.contexts.clone(),
            expires_unix_ms: u64::try_from(lease.expires_at_ms)?,
        };
        crate::causal_leases::publish_on(
            tx,
            ns,
            descriptor.snapshot_id,
            serde_json::to_vec(&descriptor)?,
            now_ms,
        )
        .await?;
        return Ok(Some(descriptor));
    }
    let collection = collections
        .get(state.collection_index as usize)
        .ok_or(SyncError::StaleWork)?
        .clone();
    let range = KeyRange {
        collection: collection.clone(),
        lower: None,
        upper: None,
    };
    match state.phase {
        SnapshotPhase::Context => {
            let capacity = std::cmp::min(limits.max_rows_scanned, data.max_bytes / 128);
            ensure!(
                capacity > 0,
                SyncError::LimitTooSmall {
                    required: reserve + 128
                }
            );
            let rows = crate::causal_observed::page_at_on(
                tx,
                ns,
                M::collection_tag(&collection).as_bytes(),
                lease.state_seq,
                state.after,
                capacity,
            )
            .await?;
            let mut context = protocol::causal_context(&state.context)?;
            for (writer, first, last) in &rows {
                context.insert_interval(*writer, *first, *last)?;
                state.after = Some((*writer, *first));
            }
            state.context = protocol::exact_context(&context);
            if rows.len() < capacity as usize {
                let body = protocol::canonical_context(&state.context)?;
                let (manifest, fragments) = crate::causal_manifests::fragment_body(
                    ManifestKind::Context,
                    &body,
                    state.context.writers.len() as u64,
                    64 * 1024,
                )?;
                ensure!(
                    fragments.len() as u64 + 4 <= limits.max_mutations,
                    SyncError::WorkLimit
                );
                ensure!(body.len() as u64 <= data.max_bytes, SyncError::WorkLimit);
                crate::causal_manifests::publish_on(
                    tx,
                    ns,
                    &manifest,
                    &fragments,
                    data.max_bytes,
                    |body| {
                        protocol::decode_context(body)?;
                        Ok(vec![])
                    },
                )
                .await?;
                state.contexts.push(protocol::CollectionContext {
                    collection,
                    context: protocol::ContextRef {
                        digest: protocol::context_digest(&state.context)?,
                        manifest,
                    },
                });
                let scope = protocol::Scope {
                    collections: state
                        .contexts
                        .iter()
                        .map(|c| c.collection.clone())
                        .collect(),
                };
                let mut root_limits = data;
                root_limits.max_bytes = root_limits
                    .max_bytes
                    .checked_sub(body.len() as u64)
                    .ok_or(SyncError::WorkLimit)?;
                root_limits.max_mutations = limits
                    .max_mutations
                    .checked_sub(fragments.len() as u64 + 8)
                    .ok_or(SyncError::WorkLimit)?;
                pin_context_root_on(tx, state, scope, now_ms, root_limits).await?;
                state.context = protocol::ExactContext { writers: vec![] };
                state.after = None;
                state.phase = SnapshotPhase::Count;
            }
        }
        SnapshotPhase::Count => {
            scan_candidate_headers_on(
                tx,
                ns,
                &range,
                lease.state_seq,
                &mut state.count,
                data,
                |_, _| Ok(true),
            )
            .await?;
            if state.count.done {
                state.hash = Some(CandidateHash::new(&protocol::RangeHasher::new(
                    &state.scope,
                    range.clone(),
                    state.count.count,
                    state.count.bytes,
                )?)?);
                state.phase = SnapshotPhase::Hash;
            }
        }
        SnapshotPhase::Hash => {
            let hash = state.hash.as_mut().ok_or(SyncError::StaleWork)?;
            hash_candidate_step_on(tx, ns, &state.scope, &range, lease.state_seq, hash, data)
                .await?;
            if hash.done() {
                state.roots.push(protocol::RangeSummary {
                    snapshot_id: state.cursor.operation_id,
                    range: range.clone(),
                    candidate_count: state.count.count,
                    candidate_bytes: state.count.bytes,
                    aggregate: hash.clone().finish(&state.scope, &range)?,
                });
                state.collection_index += 1;
                state.count = CandidateCount::default();
                state.hash = None;
                state.phase = if state.collection_index as usize == collections.len() {
                    SnapshotPhase::Publish
                } else {
                    SnapshotPhase::Context
                };
            }
        }
        SnapshotPhase::Publish => return Err(SyncError::StaleWork.into()),
    }
    state.cursor.position = state
        .cursor
        .position
        .checked_add(1)
        .ok_or(SyncError::ClockExhausted)?;
    let encoded = serde_json::to_vec(state)?;
    ensure!(
        encoded.len() as u64 <= limits.max_bytes,
        SyncError::WorkLimit
    );
    crate::causal_leases::checkpoint_bounded_on(
        tx,
        ns,
        state.cursor.operation_id,
        Some(&expected),
        encoded,
        now_ms,
        limits.max_bytes,
    )
    .await?;
    Ok(None)
}

// A private root retains every completed collection while the source is still building.
async fn pin_context_root_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &SnapshotWork<M>,
    scope: protocol::Scope<M>,
    now_ms: i64,
    limits: WorkLimits,
) -> Result<()> {
    let root = protocol::ContextRoot {
        scope,
        contexts: state.contexts.clone(),
    };
    let body = protocol::canonical_context_root(&root)?;
    let (manifest, fragments) = crate::causal_manifests::fragment_body(
        protocol::ManifestKind::ContextRoot,
        &body,
        root.contexts.len() as u64,
        64 * 1024,
    )?;
    ensure!(
        fragments.len() as u64 + root.contexts.len() as u64 + 8 <= limits.max_mutations,
        protocol::SyncError::WorkLimit
    );
    ensure!(
        body.len() as u64 <= limits.max_bytes,
        protocol::SyncError::WorkLimit
    );
    crate::causal_manifests::publish_on(
        tx,
        state.source.namespace,
        &manifest,
        &fragments,
        limits.max_bytes,
        |bytes| {
            Ok(protocol::decode_context_root::<M>(&root.scope, bytes)?
                .contexts
                .iter()
                .map(|c| c.context.manifest.manifest_digest)
                .collect())
        },
    )
    .await?;
    crate::causal_leases::pin_root_on(
        tx,
        state.source.namespace,
        state.cursor.operation_id,
        manifest.manifest_digest,
        now_ms,
    )
    .await?;
    Ok(())
}
