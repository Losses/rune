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
        let mut body_map = std::collections::BTreeMap::new();
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
    async fn context_build_checkpoints_when_remaining_bytes_are_tight() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        causal_schema::install_fresh(&db).await?;
        let namespace = Uuid::new_v4();
        let replica =
            CausalReplica::<Model>::create_empty(db.clone(), namespace, Uuid::new_v4()).await?;
        let limits = WorkLimits {
            max_requests: 1,
            max_bytes: 14_000,
            max_depth: 1,
            max_rows_scanned: 512,
            max_mutations: 1024,
        };
        let tx = db.begin().await?;
        let source = replica.identity_on(&tx).await?;
        let scope: protocol::Scope<Model> = protocol::Scope {
            collections: [Collection::Notes].into_iter().collect(),
        };
        let mut state = match admit_snapshot_on(
            &tx,
            source,
            Uuid::new_v4(),
            scope,
            SnapshotTiming {
                now_ms: 1,
                ttl_ms: 100,
            },
            limits,
            None,
        )
        .await?
        {
            SnapshotAdmission::Building(state) => state,
            _ => panic!("unexpected ready build"),
        };
        advance_snapshot_boundary_on(&tx, &mut state, 1).await?;
        advance_snapshot_context_on(&tx, &mut state, 1).await?;
        // Scratch creation pins a root and ends its request. The next request
        // has room for one step but not its conservative output reserve.
        advance_snapshot_context_on(&tx, &mut state, 1).await?;
        let before = serde_json::to_vec(&state)?;
        let cursor = state.union.as_ref().expect("unfinished context");
        let first = cursor.next_encoded_byte_bound()?;
        let remaining = limits.max_bytes - u64::try_from(before.len())? - 4096;
        assert!(first <= remaining);
        assert!(first + cursor.next_output_growth_bound() > remaining);
        advance_snapshot_context_on(&tx, &mut state, 1).await?;
        assert!(state.pending_context.is_none());
        assert_ne!(serde_json::to_vec(&state)?, before);
        for _ in 0..200 {
            if state.pending_context.is_some() {
                break;
            }
            advance_snapshot_context_on(&tx, &mut state, 1).await?;
        }
        assert!(state.pending_context.is_some());
        tx.rollback().await?;
        Ok(())
    }
    #[tokio::test]
    async fn snapshot_admission_retries_pin_one_view_and_reject_stale_work() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        causal_schema::install_fresh(&db).await?;
        let namespace = Uuid::new_v4();
        let replica =
            CausalReplica::<Model>::create_empty(db.clone(), namespace, Uuid::new_v4()).await?;
        let request = Uuid::new_v4();
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
        let tx = db.begin().await?;
        let source = replica.identity_on(&tx).await?;
        let first = match admit_snapshot_on(
            &tx,
            source.clone(),
            request,
            scope.clone(),
            crate::causal_source::SnapshotTiming {
                now_ms: 1,
                ttl_ms: 20,
            },
            limits,
            None,
        )
        .await?
        {
            SnapshotAdmission::Building(state) => state,
            _ => panic!("unexpected ready build"),
        };
        let initial_checkpoint = serde_json::to_vec(&first)?;
        assert!(!String::from_utf8_lossy(&initial_checkpoint).contains("empty_context"));
        let resumed: SnapshotWork<Model> = serde_json::from_slice(&initial_checkpoint)?;
        assert_eq!(serde_json::to_vec(&resumed)?, initial_checkpoint);
        let mut repeated = match admit_snapshot_on(
            &tx,
            source.clone(),
            request,
            scope.clone(),
            crate::causal_source::SnapshotTiming {
                now_ms: 2,
                ttl_ms: 20,
            },
            limits,
            None,
        )
        .await?
        {
            SnapshotAdmission::Building(state) => state,
            _ => panic!("unexpected ready build"),
        };
        assert_eq!(first.cursor, repeated.cursor);
        advance_snapshot_boundary_on(&tx, &mut repeated, 2).await?;
        assert!(repeated.awaiting_union);
        assert_eq!(repeated.boundary.as_ref().unwrap().next, vec![2]);
        let resumed = match admit_snapshot_on(
            &tx,
            source.clone(),
            request,
            scope.clone(),
            crate::causal_source::SnapshotTiming {
                now_ms: 2,
                ttl_ms: 20,
            },
            limits,
            Some(repeated.cursor.clone()),
        )
        .await?
        {
            SnapshotAdmission::Building(state) => state,
            _ => panic!("unexpected ready build"),
        };
        assert!(resumed.awaiting_union);
        let mut building = resumed;
        for _ in 0..200 {
            if building.pending_context.is_some() {
                break;
            }
            advance_snapshot_context_on(&tx, &mut building, 2).await?;
            building = match admit_snapshot_on(
                &tx,
                source.clone(),
                request,
                scope.clone(),
                crate::causal_source::SnapshotTiming {
                    now_ms: 2,
                    ttl_ms: 20,
                },
                limits,
                Some(building.cursor.clone()),
            )
            .await?
            {
                SnapshotAdmission::Building(state) => state,
                _ => panic!("unexpected ready build"),
            };
        }
        assert!(building.pending_context.is_some());
        advance_snapshot_segment_on(&tx, &mut building, 2).await?;
        assert!(building.collection_end);
        advance_snapshot_segment_on(&tx, &mut building, 2).await?;
        assert_eq!(building.segment_count, 1);
        assert_eq!(building.collection_index, 1);
        let lease = crate::causal_leases::building_on(&tx, namespace, request, 2).await?;
        let root: [u8; 32] = lease.retained_root.unwrap().try_into().unwrap();
        let mut scan = SnapshotSegmentScan::<Model> {
            current: Some(root),
            read: SegmentNodeRead::Header,
            collection: "notes".into(),
            lower: vec![0],
            best: None,
            done: false,
            ordered: false,
        };
        for _ in 0..100 {
            if scan.done {
                break;
            }
            snapshot_segment_scan_step_on(&tx, namespace, &mut scan, limits).await?;
            scan = serde_json::from_slice(&serde_json::to_vec(&scan)?)?;
        }
        assert!(scan.done);
        let segment = scan.best.unwrap();
        assert_eq!(segment.range.lower, None);
        assert_eq!(segment.range.upper, None);
        let mut encoder = PartitionEncoder::new("notes".into());
        let mut encoded = encoder.push(&segment)?;
        encoder = serde_json::from_slice(&serde_json::to_vec(&encoder)?)?;
        encoded.extend(encoder.finish()?);
        assert_eq!(
            encoded,
            protocol::canonical_context_partition(&segment.range, std::slice::from_ref(&segment))?
        );
        let pivot = Uuid::from_u128(42);
        let mut left = segment.clone();
        left.range.upper = Some(pivot);
        let mut right = segment.clone();
        right.range.lower = Some(pivot);
        right.context.digest[0] ^= 1;
        let mut invalid = PartitionEncoder::new("notes".into());
        invalid.push(&left)?;
        let mut duplicate = right.clone();
        duplicate.context.digest = left.context.digest;
        assert!(invalid.push(&duplicate).is_err());
        let mut encoder = PartitionEncoder::new("notes".into());
        let mut encoded = encoder.push(&left)?;
        encoded.extend(encoder.push(&right)?);
        encoded.extend(encoder.finish()?);
        assert_eq!(
            encoded,
            protocol::canonical_context_partition(&segment.range, &[left, right])?
        );
        for _ in 0..200 {
            if building.reversal.as_ref().is_some_and(|r| r.done) {
                break;
            }
            advance_snapshot_reversal_on(&tx, &mut building, 2).await?;
            building = match admit_snapshot_on(
                &tx,
                source.clone(),
                request,
                scope.clone(),
                crate::causal_source::SnapshotTiming {
                    now_ms: 2,
                    ttl_ms: 20,
                },
                limits,
                Some(building.cursor.clone()),
            )
            .await?
            {
                SnapshotAdmission::Building(state) => state,
                _ => panic!("unexpected ready build"),
            };
        }
        let reversal = building.reversal.as_ref().unwrap();
        assert!(reversal.done);
        let reverse_root = reversal.reverse_head.unwrap();
        let mut pass = PartitionPass::<Model>::new(reverse_root, "notes".into());
        for _ in 0..300 {
            if matches!(pass.phase, PartitionPassPhase::Done) {
                break;
            }
            partition_pass_step_on(&tx, namespace, &mut pass, limits).await?;
            pass = serde_json::from_slice(&serde_json::to_vec(&pass)?)?;
        }
        assert!(matches!(pass.phase, PartitionPassPhase::Done));
        let body =
            protocol::canonical_context_partition(&segment.range, std::slice::from_ref(&segment))?;
        let (manifest, _) = crate::causal_manifests::fragment_body(
            protocol::ManifestKind::ContextPartition,
            &body,
            1,
            1024,
        )?;
        assert_eq!(pass.manifest, Some(manifest.clone()));
        let mut step_counts = Vec::new();
        for max_bytes in [4167, 65536] {
            let mut resumed = PartitionPass::<Model>::new(reverse_root, "notes".into());
            let mut steps = 0;
            while !matches!(resumed.phase, PartitionPassPhase::Done) {
                partition_pass_step_on(
                    &tx,
                    namespace,
                    &mut resumed,
                    WorkLimits {
                        max_rows_scanned: 5,
                        max_bytes,
                        ..limits
                    },
                )
                .await?;
                resumed = serde_json::from_slice(&serde_json::to_vec(&resumed)?)?;
                steps += 1;
                assert!(steps < 300);
            }
            assert_eq!(resumed.manifest, Some(manifest.clone()));
            step_counts.push(steps);
        }
        assert_eq!(step_counts[0] - step_counts[1], 2);
        for _ in 0..300 {
            if building
                .partition
                .as_ref()
                .is_some_and(|pass| matches!(pass.phase, PartitionPassPhase::Done))
            {
                break;
            }
            advance_snapshot_partition_on(&tx, &mut building, 2).await?;
            building = match admit_snapshot_on(
                &tx,
                source.clone(),
                request,
                scope.clone(),
                crate::causal_source::SnapshotTiming {
                    now_ms: 2,
                    ttl_ms: 20,
                },
                limits,
                Some(building.cursor.clone()),
            )
            .await?
            {
                SnapshotAdmission::Building(state) => state,
                _ => panic!("unexpected ready build"),
            };
        }
        assert_eq!(
            building
                .partition
                .as_ref()
                .and_then(|pass| pass.manifest.clone()),
            Some(manifest.clone())
        );
        for _ in 0..300 {
            if building
                .partition_emission
                .as_ref()
                .is_some_and(|e| matches!(e.phase, PartitionEmissionPhase::Verified))
            {
                break;
            }
            advance_snapshot_partition_emission_on(&tx, &mut building, 2).await?;
            building = match admit_snapshot_on(
                &tx,
                source.clone(),
                request,
                scope.clone(),
                crate::causal_source::SnapshotTiming {
                    now_ms: 2,
                    ttl_ms: 20,
                },
                limits,
                Some(building.cursor.clone()),
            )
            .await?
            {
                SnapshotAdmission::Building(state) => state,
                _ => panic!("unexpected ready build"),
            };
        }
        assert!(
            building
                .partition_emission
                .as_ref()
                .is_some_and(|e| matches!(e.phase, PartitionEmissionPhase::Verified))
        );
        assert!(
            crate::causal_manifests::read_on(&tx, namespace, manifest.manifest_digest, 65536, 100)
                .await
                .is_err()
        );
        advance_snapshot_partition_promotion_on(&tx, &mut building, 2).await?;
        assert_eq!(building.partition_count, 1);
        for _ in 0..200 {
            if building.reference_reversal.as_ref().is_some_and(|r| r.done) {
                break;
            }
            advance_snapshot_reference_reversal_on(&tx, &mut building, 2).await?;
            building = match admit_snapshot_on(
                &tx,
                source.clone(),
                request,
                scope.clone(),
                crate::causal_source::SnapshotTiming {
                    now_ms: 2,
                    ttl_ms: 20,
                },
                limits,
                Some(building.cursor.clone()),
            )
            .await?
            {
                SnapshotAdmission::Building(state) => state,
                _ => panic!("unexpected ready build"),
            };
        }
        assert!(
            building
                .reference_reversal
                .as_ref()
                .is_some_and(|r| r.done && r.count == 1)
        );
        let (_, published) =
            crate::causal_manifests::read_on(&tx, namespace, manifest.manifest_digest, 65536, 100)
                .await?;
        assert_eq!(
            protocol::decode_context_partition(&segment.range, &published)?,
            vec![segment.clone()]
        );
        let mut root_encoder = RootEncoder::new(&scope)?;
        let mut root_bytes =
            root_encoder.push::<Model>("notes", Collection::Notes, manifest.clone())?;
        root_encoder = serde_json::from_slice(&serde_json::to_vec(&root_encoder)?)?;
        root_bytes.extend(root_encoder.finish(1)?);
        let root = protocol::ContextRoot {
            scope: scope.clone(),
            partitions: vec![protocol::ContextPartitionRef {
                collection: Collection::Notes,
                partition: manifest,
            }],
        };
        assert_eq!(root_bytes, protocol::canonical_context_root(&root)?);
        let reference_head = building
            .reference_reversal
            .as_ref()
            .unwrap()
            .reverse_head
            .unwrap();
        let mut root_pass = RootPass::new(reference_head, &scope)?;
        for _ in 0..200 {
            if matches!(root_pass.phase, PartitionPassPhase::Done) {
                break;
            }
            root_pass_step_on(&tx, namespace, &scope, &mut root_pass, limits).await?;
            root_pass = serde_json::from_slice(&serde_json::to_vec(&root_pass)?)?;
        }
        let (expected_root, _) = crate::causal_manifests::fragment_body(
            protocol::ManifestKind::ContextRoot,
            &root_bytes,
            1,
            1024,
        )?;
        assert_eq!(root_pass.manifest, Some(expected_root.clone()));
        for _ in 0..200 {
            if building
                .root_pass
                .as_ref()
                .is_some_and(|p| matches!(p.phase, PartitionPassPhase::Done))
            {
                break;
            }
            advance_snapshot_root_pass_on(&tx, &mut building, 2).await?;
            building = match admit_snapshot_on(
                &tx,
                source.clone(),
                request,
                scope.clone(),
                crate::causal_source::SnapshotTiming {
                    now_ms: 2,
                    ttl_ms: 20,
                },
                limits,
                Some(building.cursor.clone()),
            )
            .await?
            {
                SnapshotAdmission::Building(state) => state,
                _ => panic!("unexpected ready build"),
            };
        }
        assert_eq!(
            building.root_pass.as_ref().and_then(|p| p.manifest.clone()),
            Some(expected_root.clone())
        );
        for _ in 0..300 {
            if building
                .root_emission
                .as_ref()
                .is_some_and(|e| matches!(e.phase, PartitionEmissionPhase::Verified))
            {
                break;
            }
            advance_snapshot_root_emission_on(&tx, &mut building, 2).await?;
            building = match admit_snapshot_on(
                &tx,
                source.clone(),
                request,
                scope.clone(),
                crate::causal_source::SnapshotTiming {
                    now_ms: 2,
                    ttl_ms: 20,
                },
                limits,
                Some(building.cursor.clone()),
            )
            .await?
            {
                SnapshotAdmission::Building(state) => state,
                _ => panic!("unexpected ready build"),
            };
        }
        assert!(
            building
                .root_emission
                .as_ref()
                .is_some_and(|e| matches!(e.phase, PartitionEmissionPhase::Verified))
        );
        assert!(
            crate::causal_manifests::read_on(
                &tx,
                namespace,
                expected_root.manifest_digest,
                65536,
                100
            )
            .await
            .is_err()
        );
        advance_snapshot_root_promotion_on(&tx, &mut building, 2).await?;
        assert!(building.root_promoted);
        let (_, root_body) = crate::causal_manifests::read_on(
            &tx,
            namespace,
            expected_root.manifest_digest,
            65536,
            100,
        )
        .await?;
        assert_eq!(protocol::decode_context_root(&scope, &root_body)?, root);
        let mut summary = SummaryPass::<Model>::new(request, Collection::Notes, 1, reverse_root)?;
        let lease = crate::causal_leases::building_on(&tx, namespace, request, 2).await?;
        for _ in 0..300 {
            if matches!(summary.phase, SummaryPhase::Done) {
                break;
            }
            summary_pass_step_on(
                &tx,
                namespace,
                &scope,
                lease.state_seq,
                &mut summary,
                limits,
            )
            .await?;
            summary = serde_json::from_slice(&serde_json::to_vec(&summary)?)?;
        }
        let summary = summary.result.unwrap();
        let mut expected_hash =
            protocol::RangeHasher::<Model>::new(&scope, summary.range.clone(), 0, 0, 1)?;
        expected_hash.push_segment(&segment.range, segment.context.digest)?;
        let (aggregate, commitment) = expected_hash.finish_with_context()?;
        assert_eq!(
            (summary.aggregate, summary.context_commitment),
            (aggregate, commitment)
        );
        let mut stale = first.cursor.clone();
        stale.generation = Uuid::new_v4();
        let err = match admit_snapshot_on(
            &tx,
            source.clone(),
            request,
            scope.clone(),
            crate::causal_source::SnapshotTiming {
                now_ms: 2,
                ttl_ms: 20,
            },
            limits,
            Some(stale),
        )
        .await
        {
            Err(err) => err,
            Ok(_) => panic!("accepted stale cursor"),
        };
        assert!(matches!(
            err.downcast_ref::<protocol::SyncError>(),
            Some(protocol::SyncError::StaleWork)
        ));
        let mut changed = limits;
        changed.max_bytes += 1;
        assert!(
            admit_snapshot_on(
                &tx,
                source.clone(),
                request,
                scope.clone(),
                crate::causal_source::SnapshotTiming {
                    now_ms: 2,
                    ttl_ms: 20
                },
                changed,
                None
            )
            .await
            .is_err()
        );
        let err = match admit_snapshot_on(
            &tx,
            source,
            request,
            scope,
            crate::causal_source::SnapshotTiming {
                now_ms: 21,
                ttl_ms: 20,
            },
            limits,
            None,
        )
        .await
        {
            Err(err) => err,
            Ok(_) => panic!("accepted expired lease"),
        };
        assert!(matches!(
            err.downcast_ref::<protocol::SyncError>(),
            Some(protocol::SyncError::SnapshotExpired)
        ));
        tx.commit().await?;
        Ok(())
    }
    #[tokio::test]
    async fn source_root_rejects_missing_and_extra_persisted_dependencies() -> Result<()> {
        for missing in [true, false] {
            let db = Database::connect("sqlite::memory:").await?;
            causal_schema::install_fresh(&db).await?;
            let namespace = Uuid::new_v4();
            let replica =
                CausalReplica::<Model>::create_empty(db.clone(), namespace, Uuid::new_v4()).await?;
            let source = replica.identity_on(&db).await?;
            let scope: protocol::Scope<Model> = protocol::Scope {
                collections: [Collection::Notes].into_iter().collect(),
            };
            let limits = WorkLimits {
                max_requests: 1,
                max_bytes: 65536,
                max_depth: 1,
                max_rows_scanned: 8,
                max_mutations: 32,
            };
            let request = Uuid::new_v4();
            let mut cursor = None;
            let mut injected = false;
            let mut rejected = false;
            for _ in 0..1000 {
                let tx = db.begin().await?;
                let before = crate::causal_leases::load_on(&tx, namespace, request).await?;
                let result = open_on(
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
                .await;
                if let Err(error) = result {
                    assert!(injected, "{error:#}");
                    assert!(
                        error.to_string().contains(if missing {
                            "missing root dependency"
                        } else {
                            "unexpected root dependency"
                        }),
                        "{error:#}"
                    );
                    tx.rollback().await?;
                    let tx = db.begin().await?;
                    assert_eq!(
                        crate::causal_leases::load_on(&tx, namespace, request).await?,
                        before
                    );
                    tx.commit().await?;
                    rejected = true;
                    break;
                }
                let protocol::WorkOutcome::Incomplete { cursor: next } = result? else {
                    anyhow::bail!("published a damaged dependency set");
                };
                cursor = Some(next);
                let lease = crate::causal_leases::load_on(&tx, namespace, request)
                    .await?
                    .unwrap();
                let state: SnapshotWork<Model> =
                    serde_json::from_slice(lease.build_state.as_deref().unwrap())?;
                if !injected
                    && state
                        .root_emission
                        .as_ref()
                        .is_some_and(|e| matches!(e.phase, PartitionEmissionPhase::VerifyEdges))
                {
                    let root = state.root_emission.as_ref().unwrap();
                    assert_eq!(root.unverified_edges.len(), 1);
                    if missing {
                        tx.execute(
                            &Query::delete()
                                .from_table(Alias::new("sync_manifest_edges"))
                                .and_where(
                                    Expr::col(Alias::new("namespace"))
                                        .eq(namespace.as_bytes().to_vec()),
                                )
                                .and_where(
                                    Expr::col(Alias::new("parent_manifest"))
                                        .eq(root.manifest.manifest_digest.to_vec()),
                                )
                                .to_owned(),
                        )
                        .await?;
                    } else {
                        tx.execute(
                            &Query::insert()
                                .into_table(Alias::new("sync_manifest_edges"))
                                .columns(
                                    ["namespace", "parent_manifest", "child_manifest"]
                                        .map(Alias::new),
                                )
                                .values_panic([
                                    namespace.as_bytes().to_vec().into(),
                                    root.manifest.manifest_digest.to_vec().into(),
                                    root.manifest.manifest_digest.to_vec().into(),
                                ])
                                .to_owned(),
                        )
                        .await?;
                    }
                    injected = true;
                }
                tx.commit().await?;
            }
            assert!(rejected);
        }
        Ok(())
    }

    #[test]
    fn empty_context_reuse_requires_zero_floor_and_completed_empty_scan() -> Result<()> {
        let mut scan = BoundaryScan::new(vec![0])?;
        assert!(!can_reuse_empty_context(Some(0), &scan, 1, 3, true));
        scan.done = true;
        assert!(can_reuse_empty_context(Some(0), &scan, 1, 3, true));
        assert!(!can_reuse_empty_context(Some(1), &scan, 1, 3, true));
        assert!(!can_reuse_empty_context(Some(0), &scan, 1, 2, true));
        assert!(!can_reuse_empty_context(Some(0), &scan, 0, 3, true));
        scan.after = Some((1, 0));
        assert!(!can_reuse_empty_context(Some(0), &scan, 1, 3, true));
        Ok(())
    }

    #[tokio::test]
    async fn empty_context_reuse_authenticates_and_preserves_captured_removals() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        causal_schema::install_fresh(&db).await?;
        let namespace = Uuid::new_v4();
        let replica =
            CausalReplica::<Model>::create_empty(db.clone(), namespace, Uuid::new_v4()).await?;
        let tx = db.begin().await?;
        let key = ObjectKey {
            collection: Collection::Third,
            id: Uuid::new_v4(),
        };
        replica
            .write_observed_on(
                &tx,
                key.clone(),
                Some(Note {
                    text: "removed".into(),
                }),
                Observed::AllCurrent,
            )
            .await?;
        replica
            .write_observed_on(&tx, key, None, Observed::AllCurrent)
            .await?;
        let source = replica.identity_on(&tx).await?;
        let scope: protocol::Scope<Model> = protocol::Scope {
            collections: [Collection::Notes, Collection::Other, Collection::Third]
                .into_iter()
                .collect(),
        };
        let limits = WorkLimits {
            max_requests: 1,
            max_bytes: 65536,
            max_depth: 1,
            max_rows_scanned: 128,
            max_mutations: 32,
        };
        let request = Uuid::new_v4();
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
            protocol::WorkOutcome::Incomplete { cursor } => cursor,
            _ => panic!("unexpected ready"),
        };
        tx.commit().await?;
        let tx = db.begin().await?;
        replica
            .write_observed_on(
                &tx,
                ObjectKey {
                    collection: Collection::Other,
                    id: Uuid::new_v4(),
                },
                Some(Note {
                    text: "later".into(),
                }),
                Observed::AllCurrent,
            )
            .await?;
        tx.commit().await?;
        let mut checked_reuse = false;
        let mut checked_removal = false;
        for _ in 0..1000 {
            let tx = db.begin().await?;
            let lease = crate::causal_leases::building_on(&tx, namespace, request, 2).await?;
            let state: SnapshotWork<Model> =
                serde_json::from_slice(lease.build_state.as_deref().unwrap())?;
            if state.collection_index == 1
                && state.awaiting_union
                && state.union.is_none()
                && state.pending_context.is_none()
                && !checked_reuse
            {
                let context = state.empty_context.as_ref().unwrap();
                let query = Query::update()
                    .table(Alias::new("sync_chunks"))
                    .value(Alias::new("bytes"), vec![42u8])
                    .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
                    .and_where(
                        Expr::col(Alias::new("manifest"))
                            .eq(context.manifest.manifest_digest.to_vec()),
                    )
                    .to_owned();
                tx.execute(&query).await?;
                assert!(
                    open_on(
                        &tx,
                        source.clone(),
                        request,
                        scope.clone(),
                        crate::causal_source::SnapshotTiming {
                            now_ms: 2,
                            ttl_ms: 100
                        },
                        limits,
                        Some(cursor.clone())
                    )
                    .await
                    .is_err()
                );
                tx.rollback().await?;
                let tx = db.begin().await?;
                let outcome = open_on(
                    &tx,
                    source.clone(),
                    request,
                    scope.clone(),
                    crate::causal_source::SnapshotTiming {
                        now_ms: 2,
                        ttl_ms: 100,
                    },
                    limits,
                    Some(cursor.clone()),
                )
                .await?;
                cursor = match outcome {
                    protocol::WorkOutcome::Incomplete { cursor } => {
                        serde_json::from_slice(&serde_json::to_vec(&cursor)?)?
                    }
                    _ => panic!("unexpected ready"),
                };
                let lease = crate::causal_leases::building_on(&tx, namespace, request, 2).await?;
                let next: SnapshotWork<Model> =
                    serde_json::from_slice(lease.build_state.as_deref().unwrap())?;
                assert!(next.pending_context.is_some() && next.union.is_none());
                checked_reuse = true;
                tx.commit().await?;
                continue;
            }
            if state.collection_index == 2
                && state.awaiting_union
                && state.union.is_none()
                && state.pending_context.is_none()
            {
                assert!(state.boundary.as_ref().unwrap().after.is_some());
                checked_removal = true;
            }
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
                Some(cursor.clone()),
            )
            .await?
            {
                protocol::WorkOutcome::Incomplete { cursor: next } => {
                    cursor = serde_json::from_slice(&serde_json::to_vec(&next)?)?
                }
                protocol::WorkOutcome::Ready(descriptor) => {
                    assert!(checked_reuse && checked_removal);
                    assert_eq!(descriptor.roots.len(), 3);
                    tx.commit().await?;
                    return Ok(());
                }
            }
            tx.commit().await?;
        }
        anyhow::bail!("snapshot did not finish")
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
            let mut saw_nonempty_boundary = false;
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
                        return Ok((descriptor, saw_nonempty_boundary));
                    }
                    protocol::WorkOutcome::Incomplete { cursor: next } => {
                        cursor = next;
                        plan = result.next.unwrap();
                        let lease = crate::causal_leases::load_on(&tx, source.namespace, request)
                            .await?
                            .unwrap();
                        let state: SnapshotWork<Model> =
                            serde_json::from_slice(lease.build_state.as_deref().unwrap())?;
                        if !state.awaiting_union
                            && state.boundary.as_ref().is_some_and(|b| b.after.is_some())
                        {
                            saw_nonempty_boundary = true;
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
            let (wide_descriptor, saw_nonempty_boundary) = finish(
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
            assert_eq!(saw_nonempty_boundary, populated);
            assert_eq!(wide_descriptor.head, small_descriptor.head);
            let actual = &wide_descriptor.roots[0];
            let expected = &small_descriptor.roots[0];
            assert_eq!(actual.candidate_count, if populated { 8 } else { 0 });
            assert_eq!(actual.candidate_count, expected.candidate_count);
            assert_eq!(actual.candidate_bytes, expected.candidate_bytes);
            assert_eq!(actual.context_segments, expected.context_segments);
            assert_eq!(actual.context_commitment, expected.context_commitment);
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
    async fn ready_generation_is_pinned_across_connections_until_revoked() -> Result<()> {
        let path = std::env::temp_dir().join(format!("sync-view-pin-{}.sqlite", Uuid::new_v4()));
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let a = Database::connect(&url).await?;
        causal_schema::install_fresh(&a).await?;
        let b = Database::connect(&url).await?;
        let ns = Uuid::new_v4();
        let replica = CausalReplica::<Model>::create_empty(a.clone(), ns, Uuid::new_v4()).await?;
        let tx = a.begin().await?;
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
        let mut ready = false;
        for _ in 0..1000 {
            let tx = a.begin().await?;
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
                protocol::WorkOutcome::Incomplete { cursor: next } => cursor = Some(next),
                protocol::WorkOutcome::Ready(_) => {
                    ready = true;
                    tx.commit().await?;
                    break;
                }
            }
            tx.commit().await?;
        }
        assert!(ready);
        let tx = b.begin().await?;
        let lease = crate::causal_leases::ready_on(&tx, ns, request, 3).await?;
        assert!(lease.output_generation.is_some());
        assert_eq!(
            crate::causal_gc::reclaim_basis_step_on(&tx, ns, None, 1)
                .await?
                .removed,
            0
        );
        tx.commit().await?;
        let tx = a.begin().await?;
        crate::causal_leases::revoke_on(&tx, ns, request).await?;
        tx.commit().await?;
        let tx = b.begin().await?;
        let revoked = crate::causal_leases::load_on(&tx, ns, request)
            .await?
            .unwrap();
        assert_eq!(revoked.status, 2);
        assert!(revoked.output_generation.is_none());
        assert_eq!(
            crate::causal_gc::reclaim_basis_step_on(&tx, ns, None, 1)
                .await?
                .removed,
            1
        );
        tx.commit().await?;
        drop(replica);
        drop(a);
        drop(b);
        let _ = std::fs::remove_file(path);
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
        assert_eq!(
            descriptor.roots[0].context_segments,
            atomic.roots[0].context_segments
        );
        assert_eq!(
            descriptor.roots[0].context_commitment,
            atomic.roots[0].context_commitment
        );
        assert_eq!(descriptor.roots[0].aggregate, atomic.roots[0].aggregate);
        let range = protocol::KeyRange::<Model> {
            collection: Collection::Notes,
            lower: None,
            upper: None,
        };
        let initial = protocol::ContextCursor {
            segment_index: 0,
            next_lower: None,
        };
        let tx = db.begin().await?;
        let reference = crate::causal_snapshot::context_page_on(
            &tx,
            namespace,
            atomic.snapshot_id,
            2,
            range.clone(),
            initial.clone(),
            protocol::PageLimits {
                max_scanned: 100,
                max_bytes: 65536,
            },
        )
        .await?;
        assert!(reference.done);
        let mut cursor = initial.clone();
        let mut actual = Vec::new();
        for _ in 0..100 {
            let page = crate::causal_snapshot::context_page_on(
                &tx,
                namespace,
                request,
                2,
                range.clone(),
                cursor.clone(),
                protocol::PageLimits {
                    max_scanned: 1,
                    max_bytes: 65536,
                },
            )
            .await?;
            assert_eq!(page.after, cursor);
            assert_eq!(page.segments.len(), 1);
            actual.extend(page.segments);
            cursor = page.next;
            if page.done {
                break;
            }
        }
        assert_eq!(actual, reference.segments);
        let invalid = protocol::ContextCursor {
            segment_index: 0,
            next_lower: Some(Uuid::from_u128(5)),
        };
        let err = crate::causal_snapshot::context_page_on(
            &tx,
            namespace,
            request,
            2,
            range.clone(),
            invalid,
            protocol::PageLimits {
                max_scanned: 1,
                max_bytes: 65536,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err.downcast_ref::<protocol::SyncError>(),
            Some(protocol::SyncError::StaleWork)
        ));
        let forged = protocol::ContextCursor {
            segment_index: 7,
            next_lower: Some(Uuid::from_u128(5)),
        };
        let clipped = crate::causal_snapshot::context_page_on(
            &tx,
            namespace,
            request,
            2,
            range,
            forged,
            protocol::PageLimits {
                max_scanned: 1,
                max_bytes: 65536,
            },
        )
        .await?;
        assert_eq!(clipped.segments[0].range.lower, Some(Uuid::from_u128(5)));
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
        let page = crate::causal_snapshot::context_page_on(
            &tx,
            namespace,
            atomic.snapshot_id,
            2,
            subrange.clone(),
            protocol::ContextCursor {
                segment_index: 0,
                next_lower: subrange.lower,
            },
            protocol::PageLimits {
                max_scanned: 100,
                max_bytes: 65536,
            },
        )
        .await?;
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
            page.segments.len() as u64,
        )?;
        for segment in &page.segments {
            hash.push_segment(&segment.range, segment.context.digest)?;
        }
        for row in &candidates.rows {
            hash.push_candidate(&protocol::decode_candidate::<Model>(&row.body)?)?;
        }
        let (aggregate, context_commitment) = hash.finish_with_context()?;
        assert_eq!(
            (summary.aggregate, summary.context_commitment),
            (aggregate, context_commitment)
        );
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
    async fn reversal_orders_multiple_segments_for_linear_partition_passes() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        causal_schema::install_fresh(&db).await?;
        let ns = Uuid::new_v4();
        let replica = CausalReplica::<Model>::create_empty(db.clone(), ns, Uuid::new_v4()).await?;
        let tx = db.begin().await?;
        let source = replica.identity_on(&tx).await?;
        let request = Uuid::new_v4();
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
        let mut state = match admit_snapshot_on(
            &tx,
            source,
            request,
            scope,
            crate::causal_source::SnapshotTiming {
                now_ms: 1,
                ttl_ms: 100,
            },
            limits,
            None,
        )
        .await?
        {
            SnapshotAdmission::Building(state) => state,
            _ => panic!("unexpected ready build"),
        };
        let context_body =
            protocol::canonical_context(&protocol::ExactContext { writers: vec![] })?;
        let (manifest, _) = crate::causal_manifests::fragment_body(
            protocol::ManifestKind::Context,
            &context_body,
            0,
            1024,
        )?;
        let mut expected_segments = Vec::new();
        let bounds = [
            (None, Some(Uuid::from_u128(10))),
            (Some(Uuid::from_u128(10)), Some(Uuid::from_u128(20))),
            (Some(Uuid::from_u128(20)), None),
        ];
        let mut root = None;
        for (index, (lower, upper)) in bounds.into_iter().enumerate() {
            let segment = protocol::ContextSegment {
                range: protocol::KeyRange::<Model> {
                    collection: Collection::Notes,
                    lower,
                    upper,
                },
                context: protocol::ContextRef {
                    digest: [u8::try_from(index + 1)?; 32],
                    manifest: manifest.clone(),
                },
            };
            let payload = serde_json::to_vec(&SnapshotSegment {
                kind: "snapshot_segment".into(),
                version: 1,
                segment: segment.clone(),
            })?;
            root = Some(
                crate::causal_manifests::publish_work_node_on(
                    &tx, ns, root, None, &payload, 65536, 8,
                )
                .await?,
            );
            expected_segments.push(segment);
        }
        let previous = serde_json::to_vec(&state)?;
        state.collection_index = 1;
        state.segment_count = 3;
        crate::causal_leases::pin_root_on(&tx, ns, request, root.unwrap(), 2).await?;
        crate::causal_leases::checkpoint_on(
            &tx,
            ns,
            request,
            Some(&previous),
            serde_json::to_vec(&state)?,
            2,
        )
        .await?;
        for _ in 0..300 {
            if state.reversal.as_ref().is_some_and(|r| r.done) {
                break;
            }
            advance_snapshot_reversal_on(&tx, &mut state, 2).await?;
            state = serde_json::from_slice(&serde_json::to_vec(&state)?)?;
        }
        let reversal = state.reversal.as_ref().unwrap();
        assert!(reversal.done);
        assert_eq!(reversal.count, 3);
        let mut pass = PartitionPass::<Model>::new(reversal.reverse_head.unwrap(), "notes".into());
        let mut steps = 0;
        while !matches!(pass.phase, PartitionPassPhase::Done) {
            partition_pass_step_on(&tx, ns, &mut pass, limits).await?;
            pass = serde_json::from_slice(&serde_json::to_vec(&pass)?)?;
            steps += 1;
            assert!(steps < 100, "ordered pass must remain linear");
        }
        let whole = protocol::KeyRange::<Model> {
            collection: Collection::Notes,
            lower: None,
            upper: None,
        };
        let body = protocol::canonical_context_partition(&whole, &expected_segments)?;
        let (expected, _) = crate::causal_manifests::fragment_body(
            protocol::ManifestKind::ContextPartition,
            &body,
            3,
            1024,
        )?;
        assert_eq!(pass.manifest, Some(expected));
        tx.commit().await?;
        Ok(())
    }
    #[tokio::test]
    async fn interval_scan_resumes_one_row_and_rejects_overlaps() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        causal_schema::install_fresh(&db).await?;
        let namespace = Uuid::new_v4();
        let writer = Uuid::new_v4();
        let context = [3u8; 32];
        for (first, last) in [(1i64, 2i64), (5, 7)] {
            let q = Query::insert()
                .into_table(Alias::new("sync_context_intervals"))
                .columns(
                    ["namespace", "context_manifest", "writer", "first", "last"].map(Alias::new),
                )
                .values_panic([
                    namespace.as_bytes().to_vec().into(),
                    context.to_vec().into(),
                    writer.as_bytes().to_vec().into(),
                    first.into(),
                    last.into(),
                ])
                .to_owned();
            db.execute(&q).await?;
        }
        let limits = WorkLimits {
            max_requests: 1,
            max_bytes: 32,
            max_depth: 1,
            max_rows_scanned: 1,
            max_mutations: 1,
        };
        let tx = db.begin().await?;
        let first = interval_step_on(&tx, namespace, context, None, limits)
            .await?
            .unwrap();
        let second = interval_step_on(&tx, namespace, context, Some(&first), limits)
            .await?
            .unwrap();
        assert_eq!(
            (first.first, first.last, second.first, second.last),
            (1, 2, 5, 7)
        );
        assert!(
            interval_step_on(&tx, namespace, context, Some(&second), limits)
                .await?
                .is_none()
        );
        let invalid = IndexedInterval {
            writer,
            first: 1,
            last: 5,
        };
        assert!(
            interval_step_on(&tx, namespace, context, Some(&invalid), limits)
                .await
                .is_err()
        );
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
            let mut hasher = protocol::RangeHasher::new(&scope, range.clone(), 8, totals[0], 1)?;
            hasher.push_segment(
                &range,
                protocol::context_digest(&protocol::ExactContext { writers: vec![] })?,
            )?;
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
        let empty = protocol::ExactContext {
            writers: Vec::new(),
        };
        let mut hasher = protocol::RangeHasher::<Model>::new(
            &scope,
            range.clone(),
            state.count,
            state.bytes,
            1,
        )?;
        hasher.push_segment(&range, protocol::context_digest(&empty)?)?;
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
        let expected = direct.finish_with_context()?;
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
    async fn boundary_scan_resumes_and_ignores_post_snapshot_assertions() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        causal_schema::install_fresh(&db).await?;
        let namespace = Uuid::new_v4();
        let midpoint = Uuid::from_u128(100);
        let late = Uuid::from_u128(20);
        let bound = |id: Uuid| {
            let mut b = vec![1];
            b.extend_from_slice(id.as_bytes());
            b
        };
        for (seq, lower, upper) in [
            (1i64, vec![0], bound(midpoint)),
            (2, bound(midpoint), vec![2]),
            (3, bound(late), vec![2]),
        ] {
            let q = Query::insert()
                .into_table(Alias::new("sync_context_ranges"))
                .columns(
                    [
                        "namespace",
                        "collection",
                        "basis_generation",
                        "state_seq",
                        "ordinal",
                        "lower_bound",
                        "upper_bound",
                        "context_manifest",
                    ]
                    .map(Alias::new),
                )
                .values_panic([
                    namespace.as_bytes().to_vec().into(),
                    b"notes".to_vec().into(),
                    Uuid::nil().as_bytes().to_vec().into(),
                    seq.into(),
                    0i64.into(),
                    lower.into(),
                    upper.into(),
                    vec![1u8; 32].into(),
                ])
                .to_owned();
            db.execute(&q).await?;
        }
        let limits = WorkLimits {
            max_requests: 1,
            max_bytes: 1024,
            max_depth: 1,
            max_rows_scanned: 1,
            max_mutations: 2,
        };
        let mut state = BoundaryScan::new(vec![0])?;
        let mut steps = 0;
        while !state.done {
            let tx = db.begin().await?;
            boundary_step_on(
                &tx,
                namespace,
                b"notes",
                crate::causal_source::CapturedBasis {
                    floor: 0,
                    generation: Uuid::nil().as_bytes(),
                    state_seq: 2,
                },
                &mut state,
                limits,
            )
            .await?;
            tx.commit().await?;
            state = serde_json::from_slice(&serde_json::to_vec(&state)?)?;
            steps += 1;
            assert!(steps < 6);
        }
        assert_eq!(steps, 3);
        assert_eq!(state.next, bound(midpoint));
        for (row_budget, byte_budget, expected_steps) in [(64, 4096, 1), (64, 64, 3), (2, 128, 2)] {
            let mut batched = BoundaryScan::new(vec![0])?;
            let mut steps = 0;
            while !batched.done {
                let tx = db.begin().await?;
                boundary_step_on(
                    &tx,
                    namespace,
                    b"notes",
                    crate::causal_source::CapturedBasis {
                        floor: 0,
                        generation: Uuid::nil().as_bytes(),
                        state_seq: 2,
                    },
                    &mut batched,
                    WorkLimits {
                        max_rows_scanned: row_budget,
                        max_bytes: byte_budget,
                        ..limits
                    },
                )
                .await?;
                tx.commit().await?;
                batched = serde_json::from_slice(&serde_json::to_vec(&batched)?)?;
                steps += 1;
                assert!(steps <= 3);
            }
            assert_eq!(steps, expected_steps);
            assert_eq!(batched.next, bound(midpoint));
        }
        let mut covering = CoveringScan::new(vec![0], bound(midpoint))?;
        let mut digests = Vec::new();
        while !covering.done {
            let tx = db.begin().await?;
            if let Some(digest) = covering_step_on(
                &tx,
                namespace,
                b"notes",
                crate::causal_source::CapturedBasis {
                    floor: 0,
                    generation: Uuid::nil().as_bytes(),
                    state_seq: 2,
                },
                &mut covering,
                limits,
            )
            .await?
            {
                digests.push(digest);
            }
            tx.commit().await?;
            covering = serde_json::from_slice(&serde_json::to_vec(&covering)?)?;
        }
        assert_eq!(digests, vec![[1u8; 32]]);
        let mut invalid_partition = CoveringScan::new(vec![0], vec![2])?;
        let tx = db.begin().await?;
        assert!(
            covering_step_on(
                &tx,
                namespace,
                b"notes",
                crate::causal_source::CapturedBasis {
                    floor: 0,
                    generation: Uuid::nil().as_bytes(),
                    state_seq: 2
                },
                &mut invalid_partition,
                limits
            )
            .await
            .is_err()
        );
        tx.rollback().await?;
        let absent_basis = Uuid::new_v4();
        let mut missing = CoveringScan::new(vec![0], bound(midpoint))?;
        let tx = db.begin().await?;
        assert!(
            covering_step_on(
                &tx,
                namespace,
                b"notes",
                crate::causal_source::CapturedBasis {
                    floor: 1,
                    generation: absent_basis.as_bytes(),
                    state_seq: 1
                },
                &mut missing,
                limits
            )
            .await
            .is_err()
        );
        tx.rollback().await?;
        let q = Query::insert()
            .into_table(Alias::new("sync_context_ranges"))
            .columns(
                [
                    "namespace",
                    "collection",
                    "basis_generation",
                    "state_seq",
                    "ordinal",
                    "lower_bound",
                    "upper_bound",
                    "context_manifest",
                ]
                .map(Alias::new),
            )
            .values_panic([
                namespace.as_bytes().to_vec().into(),
                b"notes".to_vec().into(),
                absent_basis.as_bytes().to_vec().into(),
                1i64.into(),
                0i64.into(),
                vec![0u8].into(),
                vec![2u8].into(),
                vec![2u8; 32].into(),
            ])
            .to_owned();
        db.execute(&q).await?;
        let mut with_basis = CoveringScan::new(vec![0], bound(midpoint))?;
        let mut observed = Vec::new();
        while !with_basis.done {
            let tx = db.begin().await?;
            if let Some(digest) = covering_step_on(
                &tx,
                namespace,
                b"notes",
                crate::causal_source::CapturedBasis {
                    floor: 1,
                    generation: absent_basis.as_bytes(),
                    state_seq: 2,
                },
                &mut with_basis,
                limits,
            )
            .await?
            {
                observed.push(digest);
            }
            tx.commit().await?;
            with_basis = serde_json::from_slice(&serde_json::to_vec(&with_basis)?)?;
        }
        assert_eq!(observed, vec![[2u8; 32]]);
        for (row_budget, expected_steps) in [(1, 4), (64, 2)] {
            let mut with_basis = BoundaryScan::new(vec![0])?;
            let mut steps = 0;
            while !with_basis.done {
                let tx = db.begin().await?;
                boundary_step_on(
                    &tx,
                    namespace,
                    b"notes",
                    crate::causal_source::CapturedBasis {
                        floor: 1,
                        generation: absent_basis.as_bytes(),
                        state_seq: 2,
                    },
                    &mut with_basis,
                    WorkLimits {
                        max_rows_scanned: row_budget,
                        ..limits
                    },
                )
                .await?;
                tx.commit().await?;
                with_basis = serde_json::from_slice(&serde_json::to_vec(&with_basis)?)?;
                steps += 1;
                assert!(steps <= 4);
            }
            assert_eq!(steps, expected_steps);
            assert_eq!(with_basis.next, bound(midpoint));
        }
        Ok(())
    }
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

/// Resumable candidate-feed phase once the exact context segments have been fed.
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
    ) -> Result<(protocol::Digest, protocol::Digest)> {
        ensure!(self.scan.done, "candidate hash pass is incomplete");
        Ok(
            protocol::RangeHasher::<M>::from_checkpoint(scope, range, &self.hasher_checkpoint)?
                .finish_with_context()?,
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

/// Incremental sweep to find the next context boundary at a captured basis.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BoundaryScan {
    pub lower: Vec<u8>,
    pub next: Vec<u8>,
    after: Option<(i64, i64)>,
    baseline_done: bool,
    pub done: bool,
}
impl BoundaryScan {
    pub(crate) fn new(lower: Vec<u8>) -> Result<Self> {
        ensure!(
            crate::causal_compaction::valid_bound(&lower) && lower < vec![2],
            "invalid boundary"
        );
        Ok(Self {
            lower,
            next: vec![2],
            after: None,
            baseline_done: false,
            done: false,
        })
    }
}
#[derive(FromQueryResult)]
struct BoundaryRow {
    state_seq: i64,
    ordinal: i64,
    lower_bound: Vec<u8>,
    upper_bound: Vec<u8>,
}

#[derive(Clone, Copy)]
pub(crate) struct CapturedBasis<'a> {
    pub floor: i64,
    pub generation: &'a [u8],
    pub state_seq: i64,
}

/// Scan one bounded page of retained context assertion headers. Baseline and
/// later assertions use separate indexed passes, with a checkpointable cursor.
pub(crate) async fn boundary_step_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    collection: &[u8],
    captured: CapturedBasis<'_>,
    state: &mut BoundaryScan,
    limits: WorkLimits,
) -> Result<bool> {
    let CapturedBasis {
        floor,
        generation: basis,
        state_seq,
    } = captured;
    ensure!(
        !namespace.is_nil()
            && !collection.is_empty()
            && basis.len() == 16
            && floor >= 0
            && state_seq >= floor
            && limits.max_rows_scanned > 0,
        "invalid boundary step"
    );
    if state.done {
        return Ok(false);
    }
    const HEADER_BYTES: u64 = 64;
    if limits.max_bytes < HEADER_BYTES {
        return Err(protocol::SyncError::LimitTooSmall {
            required: HEADER_BYTES,
        }
        .into());
    }
    let page_size = std::cmp::min(
        std::cmp::min(limits.max_rows_scanned, limits.max_bytes / HEADER_BYTES),
        1024,
    );
    let mut q = Query::select();
    q.columns(["state_seq", "ordinal", "lower_bound", "upper_bound"].map(Alias::new))
        .from(Alias::new("sync_context_ranges"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("collection")).eq(collection.to_vec()))
        .cond_where(scan_basis_condition(floor, basis, state.baseline_done))
        .and_where(Expr::col(Alias::new("state_seq")).lte(state_seq));
    if let Some((version, ordinal)) = state.after {
        q.cond_where(
            Condition::any()
                .add(Expr::col(Alias::new("state_seq")).gt(version))
                .add(
                    Condition::all()
                        .add(Expr::col(Alias::new("state_seq")).eq(version))
                        .add(Expr::col(Alias::new("ordinal")).gt(ordinal)),
                ),
        );
    }
    q.order_by(Alias::new("state_seq"), Order::Asc)
        .order_by(Alias::new("ordinal"), Order::Asc)
        .limit(page_size);
    let rows = BoundaryRow::find_by_statement(tx.get_database_backend().build(&q.to_owned()))
        .all(tx)
        .await?;
    // Validate the whole page before updating the caller's continuation.
    for row in &rows {
        ensure!(
            row.state_seq >= floor
                && row.state_seq <= state_seq
                && row.ordinal >= 0
                && crate::causal_compaction::valid_bound(&row.lower_bound)
                && crate::causal_compaction::valid_bound(&row.upper_bound)
                && row.lower_bound < row.upper_bound,
            "invalid context assertion"
        );
    }
    for row in &rows {
        for bound in [&row.lower_bound, &row.upper_bound] {
            if bound > &state.lower && bound < &state.next {
                state.next = bound.clone();
            }
        }
        state.after = Some((row.state_seq, row.ordinal));
    }
    if (rows.len() as u64) < page_size {
        if floor > 0 && !state.baseline_done {
            state.baseline_done = true;
            state.after = None;
        } else {
            state.done = true;
        }
    }
    Ok(true)
}

/// A restartable scan of the assertions covering one already-selected partition.
/// Each result identifies a context manifest; the caller must merge its exact
/// contents before advancing the durable checkpoint past that assertion.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CoveringScan {
    lower: Vec<u8>,
    upper: Vec<u8>,
    after: Option<(i64, i64)>,
    baseline_done: bool,
    baseline_seen: bool,
    pub done: bool,
}

impl CoveringScan {
    pub(crate) fn new(lower: Vec<u8>, upper: Vec<u8>) -> Result<Self> {
        ensure!(
            crate::causal_compaction::valid_bound(&lower)
                && crate::causal_compaction::valid_bound(&upper)
                && lower < upper,
            "invalid partition bounds"
        );
        Ok(Self {
            lower,
            upper,
            after: None,
            baseline_done: false,
            baseline_seen: false,
            done: false,
        })
    }
}

#[derive(FromQueryResult)]
struct CoveringRow {
    state_seq: i64,
    ordinal: i64,
    lower_bound: Vec<u8>,
    upper_bound: Vec<u8>,
    context_manifest: Vec<u8>,
}

pub(crate) async fn covering_step_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    collection: &[u8],
    captured: CapturedBasis<'_>,
    state: &mut CoveringScan,
    limits: WorkLimits,
) -> Result<Option<[u8; 32]>> {
    let CapturedBasis {
        floor,
        generation: basis,
        state_seq,
    } = captured;
    ensure!(
        !namespace.is_nil()
            && !collection.is_empty()
            && basis.len() == 16
            && floor >= 0
            && state_seq >= floor
            && limits.max_rows_scanned > 0,
        "invalid covering scan"
    );
    if state.done {
        return Ok(None);
    }
    const ROW_BYTES: u64 = 96;
    if limits.max_bytes < ROW_BYTES {
        return Err(protocol::SyncError::LimitTooSmall {
            required: ROW_BYTES,
        }
        .into());
    }
    let mut q = Query::select();
    q.columns(
        [
            "state_seq",
            "ordinal",
            "lower_bound",
            "upper_bound",
            "context_manifest",
        ]
        .map(Alias::new),
    )
    .from(Alias::new("sync_context_ranges"))
    .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
    .and_where(Expr::col(Alias::new("collection")).eq(collection.to_vec()))
    .cond_where(scan_basis_condition(floor, basis, state.baseline_done))
    .and_where(Expr::col(Alias::new("state_seq")).lte(state_seq));
    if let Some((version, ordinal)) = state.after {
        q.cond_where(
            Condition::any()
                .add(Expr::col(Alias::new("state_seq")).gt(version))
                .add(
                    Condition::all()
                        .add(Expr::col(Alias::new("state_seq")).eq(version))
                        .add(Expr::col(Alias::new("ordinal")).gt(ordinal)),
                ),
        );
    }
    q.order_by(Alias::new("state_seq"), Order::Asc)
        .order_by(Alias::new("ordinal"), Order::Asc)
        .limit(1);
    let Some(row) = CoveringRow::find_by_statement(tx.get_database_backend().build(&q.to_owned()))
        .one(tx)
        .await?
    else {
        ensure!(
            floor == 0 || state.baseline_seen,
            "missing baseline coverage"
        );
        if floor > 0 && !state.baseline_done {
            state.baseline_done = true;
            state.after = None;
            return Ok(None);
        }
        state.done = true;
        return Ok(None);
    };
    ensure!(
        row.state_seq >= floor
            && row.state_seq <= state_seq
            && row.ordinal >= 0
            && crate::causal_compaction::valid_bound(&row.lower_bound)
            && crate::causal_compaction::valid_bound(&row.upper_bound)
            && row.lower_bound < row.upper_bound,
        "invalid covering assertion"
    );
    state.after = Some((row.state_seq, row.ordinal));
    if row.upper_bound <= state.lower || row.lower_bound >= state.upper {
        return Ok(None);
    }
    ensure!(
        row.lower_bound <= state.lower && row.upper_bound >= state.upper,
        "partition crosses an assertion boundary"
    );
    if floor > 0 && row.state_seq == floor {
        ensure!(!state.baseline_seen, "overlapping baseline coverage");
        state.baseline_seen = true;
    } else {
        ensure!(
            floor == 0 || state.baseline_seen,
            "missing baseline coverage"
        );
    }
    let digest = row
        .context_manifest
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid context digest"))?;
    Ok(Some(digest))
}

// Each phase fixes the generation, allowing indexed cursor advancement.
fn scan_basis_condition(floor: i64, basis: &[u8], baseline_done: bool) -> Condition {
    let is_baseline = floor > 0 && !baseline_done;
    let generation = if is_baseline {
        basis.to_vec()
    } else {
        Uuid::nil().as_bytes().to_vec()
    };
    let sequence = if is_baseline {
        Expr::col(Alias::new("state_seq")).eq(floor)
    } else {
        Expr::col(Alias::new("state_seq")).gt(floor)
    };
    Condition::all()
        .add(Expr::col(Alias::new("basis_generation")).eq(generation))
        .add(sequence)
}

/// Release a snapshot request by the caller's stable request ID. An unknown or
/// previously revoked request is already released; no permanent receipt is
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

/// One entry in a retained context's ordered interval index. Context indexes
/// are immutable and are retained by the same captured-basis pin as their
/// owning assertion.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(crate) struct IndexedInterval {
    pub writer: Uuid,
    pub first: i64,
    pub last: i64,
}

/// Read the next indexed interval after a cursor, consuming at most one row.
/// This is a building block for a resumable ordered union; it does not itself
/// establish completeness or publish a context.
pub(crate) async fn interval_step_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    context: [u8; 32],
    after: Option<&IndexedInterval>,
    limits: WorkLimits,
) -> Result<Option<IndexedInterval>> {
    ensure!(
        !namespace.is_nil() && limits.max_rows_scanned > 0,
        "invalid interval scan"
    );
    const ROW_BYTES: u64 = 32;
    if limits.max_bytes < ROW_BYTES {
        return Err(protocol::SyncError::LimitTooSmall {
            required: ROW_BYTES,
        }
        .into());
    }
    let mut q = Query::select();
    q.columns(["writer", "first", "last"].map(Alias::new))
        .from(Alias::new("sync_context_intervals"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("context_manifest")).eq(context.to_vec()));
    if let Some(after) = after {
        ensure!(
            !after.writer.is_nil() && after.first > 0 && after.last >= after.first,
            "invalid interval cursor"
        );
        q.cond_where(
            Condition::any()
                .add(Expr::col(Alias::new("writer")).gt(after.writer.as_bytes().to_vec()))
                .add(
                    Condition::all()
                        .add(Expr::col(Alias::new("writer")).eq(after.writer.as_bytes().to_vec()))
                        .add(Expr::col(Alias::new("first")).gt(after.first)),
                ),
        );
    }
    q.order_by(Alias::new("writer"), Order::Asc)
        .order_by(Alias::new("first"), Order::Asc)
        .limit(1);
    let Some(row) = tx.query_one(&q.to_owned()).await? else {
        return Ok(None);
    };
    let writer = Uuid::from_slice(&row.try_get_by_index::<Vec<u8>>(0)?)?;
    let first: i64 = row.try_get_by_index(1)?;
    let last: i64 = row.try_get_by_index(2)?;
    ensure!(
        !writer.is_nil() && first > 0 && last >= first,
        "invalid indexed interval"
    );
    if let Some(after) = after {
        ensure!(
            writer > after.writer || (writer == after.writer && first > after.last),
            "overlapping or unordered context intervals"
        );
    }
    Ok(Some(IndexedInterval {
        writer,
        first,
        last,
    }))
}

/// Durable admission envelope for a source snapshot. Its cursor generation is
/// independent of the replica generation, so a forgotten request ID cannot
/// accidentally accept an old continuation after a new build is created.
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
    lower: Vec<u8>,
    boundary: Option<BoundaryScan>,
    awaiting_union: bool,
    union: Option<crate::causal_context_builder::ContextBuildCursor>,
    pending_context: Option<protocol::ContextRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    empty_context: Option<protocol::ContextRef>,
    open_segment: Option<protocol::ContextSegment<M>>,
    collection_end: bool,
    segment_count: u64,
    partition_index: u32,
    partition: Option<PartitionPass<M>>,
    partition_emission: Option<PartitionEmission<M>>,
    partition_base_root: Option<protocol::Digest>,
    partition_count: u32,
    reversal: Option<SnapshotReversal>,
    reference_reversal: Option<ReferenceReversal>,
    root_pass: Option<RootPass>,
    root_emission: Option<RootEmission>,
    root_promoted: bool,
    staging: Option<SourceStaging<M>>,
    summary_refs: Option<SummaryReferenceScan>,
    summary: Option<SummaryPass<M>>,
    summary_count: u32,
    descriptor_started: bool,
    descriptor_complete: bool,
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
    Ready(protocol::SnapshotDescriptor<M>),
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
            || state.version != 1
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
            return Ok(SnapshotAdmission::Ready(descriptor));
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
    let state = SnapshotWork {
        kind: "snapshot_open".into(),
        version: 1,
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
        lower: vec![0],
        boundary: None,
        awaiting_union: false,
        union: None,
        pending_context: None,
        empty_context: None,
        open_segment: None,
        collection_end: false,
        segment_count: 0,
        partition_index: 0,
        partition: None,
        partition_emission: None,
        partition_base_root: None,
        partition_count: 0,
        reversal: None,
        reference_reversal: None,
        root_pass: None,
        root_emission: None,
        root_promoted: false,
        staging: None,
        summary_refs: None,
        summary: None,
        summary_count: 0,
        descriptor_started: false,
        descriptor_complete: false,
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

/// Advance the next partition-boundary search and checkpoint the resulting
/// cursor in the same caller transaction. Once `awaiting_union` is set, the
/// selected half-open interval is `lower..boundary.next`; callers must finish
/// and attach its context before selecting another boundary.
pub(crate) async fn advance_snapshot_boundary_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &mut SnapshotWork<M>,
    now_ms: i64,
) -> Result<()> {
    let work_limits = state.work_limits();
    use protocol::SyncError;
    ensure!(
        !state.awaiting_union && !state.collection_end,
        "snapshot partition awaits context union or final segment"
    );
    if work_limits.max_mutations < 3 {
        return Err(SyncError::LimitTooSmall { required: 3 }.into());
    }
    let namespace = state.source.namespace;
    let expected = serde_json::to_vec(state)?;
    let checkpoint_bound = u64::try_from(expected.len())?
        .checked_add(512)
        .ok_or(SyncError::LogicalLimit)?;
    let required_bytes = checkpoint_bound
        .checked_add(64)
        .ok_or(SyncError::LogicalLimit)?;
    if work_limits.max_bytes < required_bytes {
        return Err(SyncError::LimitTooSmall {
            required: required_bytes,
        }
        .into());
    }
    let mut next = state.clone();
    let lease =
        crate::causal_leases::building_on(tx, namespace, state.cursor.operation_id, now_ms).await?;
    ensure!(
        lease.build_state.as_deref() == Some(expected.as_slice()),
        "stale snapshot checkpoint"
    );
    ensure!(
        lease.generation == state.source.generation.as_bytes(),
        "snapshot generation changed"
    );
    let floor = lease.basis_state_seq.ok_or(SyncError::StaleWork)?;
    let basis = lease
        .basis_generation
        .as_deref()
        .ok_or(SyncError::StaleWork)?;
    let mut collections: Vec<_> = next.scope.collections.iter().collect();
    collections.sort_by_key(|value| M::collection_tag(value));
    let collection = collections
        .get(usize::try_from(next.collection_index)?)
        .ok_or_else(|| anyhow::anyhow!("snapshot collection cursor exhausted"))?;
    if next.boundary.is_none() {
        next.boundary = Some(BoundaryScan::new(next.lower.clone())?);
    }
    let boundary = next.boundary.as_mut().ok_or(SyncError::StaleWork)?;
    let mut read_limits = work_limits;
    read_limits.max_bytes -= checkpoint_bound;
    boundary_step_on(
        tx,
        namespace,
        M::collection_tag(collection).as_bytes(),
        crate::causal_source::CapturedBasis {
            floor,
            generation: basis,
            state_seq: lease.state_seq,
        },
        boundary,
        read_limits,
    )
    .await?;
    if boundary.done {
        ensure!(
            boundary.lower == next.lower && boundary.next > next.lower,
            "invalid snapshot boundary"
        );
        next.awaiting_union = true;
    }
    next.cursor.position = state
        .cursor
        .position
        .checked_add(1)
        .ok_or(SyncError::LogicalLimit)?;
    let encoded = serde_json::to_vec(&next)?;
    ensure!(
        u64::try_from(encoded.len())? <= checkpoint_bound,
        "boundary checkpoint grew beyond its admitted bound"
    );
    crate::causal_leases::checkpoint_on(
        tx,
        namespace,
        next.cursor.operation_id,
        Some(&expected),
        encoded,
        now_ms,
    )
    .await?;
    *state = next;
    Ok(())
}

fn can_reuse_empty_context(
    floor: Option<i64>,
    boundary: &BoundaryScan,
    segment_count: u64,
    rows: u64,
    cached: bool,
) -> bool {
    floor == Some(0)
        && boundary.done
        && boundary.after.is_none()
        && segment_count > 0
        && rows >= 3
        && cached
}

/// Advance the context for the selected partition. The context builder owns
/// its nested cursor; any returned root and the enclosing cursor are committed
/// together by this transaction before the next continuation is returned.
pub(crate) async fn advance_snapshot_context_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &mut SnapshotWork<M>,
    now_ms: i64,
) -> Result<()> {
    let work_limits = state.work_limits();
    use crate::causal_context_builder::{
        ContextBuildCursor, ContextBuildSession, ContextBuildStep,
    };
    use protocol::SyncError;
    ensure!(
        state.awaiting_union && state.pending_context.is_none(),
        "snapshot has no pending context"
    );
    let namespace = state.source.namespace;
    let request = state.cursor.operation_id;
    let expected = serde_json::to_vec(state)?;
    let mut next = state.clone();
    let (builder_mutations, will_pin) = next
        .union
        .as_ref()
        .map(ContextBuildCursor::next_mutation_bound)
        .unwrap_or((1, false));
    let checkpoint_bound = u64::try_from(expected.len())?
        .checked_add(4096)
        .ok_or(SyncError::LogicalLimit)?;
    let builder_bytes = match &next.union {
        Some(cursor) => cursor.next_encoded_byte_bound()?,
        None => 4096,
    };
    let required_bytes = checkpoint_bound
        .checked_add(builder_bytes)
        .ok_or(SyncError::LogicalLimit)?;
    if work_limits.max_bytes < required_bytes {
        return Err(SyncError::LimitTooSmall {
            required: required_bytes,
        }
        .into());
    }
    let required_mutations = 3u64
        .checked_add(builder_mutations)
        .and_then(|n| n.checked_add(if will_pin { 2 } else { 0 }))
        .ok_or(SyncError::LogicalLimit)?;
    if work_limits.max_mutations < required_mutations {
        return Err(SyncError::LimitTooSmall {
            required: required_mutations,
        }
        .into());
    }
    let lease = crate::causal_leases::building_on(tx, namespace, request, now_ms).await?;
    ensure!(
        lease.build_state.as_deref() == Some(expected.as_slice())
            && lease.generation == next.source.generation.as_bytes(),
        "stale snapshot checkpoint"
    );
    let mut root = None;
    if let Some(mut cursor) = next.union.take() {
        let mut session = ContextBuildSession::begin(tx, &cursor, now_ms).await?;
        let mut remaining_rows = work_limits.max_rows_scanned;
        let mut remaining_bytes = work_limits.max_bytes - checkpoint_bound;
        let mut remaining_mutations = work_limits.max_mutations - 3;
        let mut advanced = 0u64;
        loop {
            let rows = cursor.next_row_bound();
            let bytes = cursor.next_encoded_byte_bound()?;
            // Reserve possible cursor growth after an earlier step. The
            // reserve is not charged against the cumulative work budget.
            let output_fits = advanced == 0
                || bytes
                    .checked_add(cursor.next_output_growth_bound())
                    .is_some_and(|required| required <= remaining_bytes);
            let (mutations, pin_step) = cursor.next_mutation_bound();
            let writes = mutations
                .checked_add(if pin_step { 2 } else { 0 })
                .ok_or(SyncError::LogicalLimit)?;
            if remaining_rows == 0
                || rows > remaining_rows
                || bytes > remaining_bytes
                || !output_fits
                || writes > remaining_mutations
                || advanced >= 1024
            {
                ensure!(
                    advanced > 0,
                    "context step does not fit its preflight budget"
                );
                next.union = Some(cursor);
                break;
            }
            let mut builder_limits = work_limits;
            builder_limits.max_rows_scanned = remaining_rows;
            builder_limits.max_bytes = remaining_bytes;
            builder_limits.max_mutations = remaining_mutations - if pin_step { 2 } else { 0 };
            let step = session.step(cursor, builder_limits).await?;
            remaining_rows -= rows;
            remaining_bytes -= bytes;
            remaining_mutations -= writes;
            advanced += 1;
            match step {
                ContextBuildStep::Progress {
                    cursor: after,
                    root: new_root,
                } => {
                    ensure!(
                        new_root.is_some() == pin_step,
                        "unexpected context root transition"
                    );
                    if new_root.is_some() {
                        next.union = Some(*after);
                        root = new_root;
                        break;
                    }
                    cursor = *after;
                }
                ContextBuildStep::Ready(context) => {
                    ensure!(!pin_step, "unexpected completed context root transition");
                    let empty = protocol::ExactContext {
                        writers: Vec::new(),
                    };
                    if context.digest == protocol::context_digest(&empty)? {
                        next.empty_context = Some(context.clone());
                    }
                    next.pending_context = Some(context);
                    break;
                }
            }
        }
    } else {
        let boundary = next.boundary.as_ref().ok_or(SyncError::StaleWork)?;
        ensure!(
            boundary.done && boundary.lower == next.lower,
            "missing snapshot partition"
        );
        let can_reuse = can_reuse_empty_context(
            lease.basis_state_seq,
            boundary,
            next.segment_count,
            work_limits.max_rows_scanned,
            next.empty_context.is_some(),
        );
        if can_reuse {
            let context = next.empty_context.as_ref().ok_or(SyncError::StaleWork)?;
            let empty = protocol::ExactContext {
                writers: Vec::new(),
            };
            let body = protocol::canonical_context(&empty)?;
            let (expected_manifest, _) = crate::causal_manifests::fragment_body(
                protocol::ManifestKind::Context,
                &body,
                0,
                1024,
            )?;
            ensure!(
                context.digest == protocol::context_digest(&empty)?
                    && context.manifest == expected_manifest,
                "invalid cached empty context"
            );
            let (stored, stored_body) = crate::causal_manifests::read_on(
                tx,
                namespace,
                context.manifest.manifest_digest,
                4096,
                1,
            )
            .await?;
            ensure!(
                stored == expected_manifest && stored_body == body,
                "invalid retained empty context"
            );
            next.pending_context = Some(context.clone());
        } else {
            let mut collections: Vec<_> = next.scope.collections.iter().collect();
            collections.sort_by_key(|value| M::collection_tag(value));
            let collection = collections
                .get(usize::try_from(next.collection_index)?)
                .ok_or_else(|| anyhow::anyhow!("snapshot collection cursor exhausted"))?;
            next.union = Some(
                ContextBuildCursor::begin_on(
                    tx,
                    namespace,
                    request,
                    now_ms,
                    M::collection_tag(collection).as_bytes().to_vec(),
                    boundary.lower.clone(),
                    boundary.next.clone(),
                )
                .await?,
            );
        }
    }
    next.cursor.position = next
        .cursor
        .position
        .checked_add(1)
        .ok_or(SyncError::LogicalLimit)?;
    let encoded = serde_json::to_vec(&next)?;
    ensure!(
        u64::try_from(encoded.len())? <= checkpoint_bound,
        "context checkpoint grew beyond its admitted bound"
    );
    if let Some(root) = root {
        crate::causal_leases::pin_root_on(tx, namespace, request, root, now_ms).await?;
    }
    crate::causal_leases::checkpoint_on(tx, namespace, request, Some(&expected), encoded, now_ms)
        .await?;
    *state = next;
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
struct SnapshotSegment<M: SyncModel> {
    kind: String,
    version: u32,
    segment: protocol::ContextSegment<M>,
}

/// Coalesce the selected context with its predecessor or root a completed
/// segment as a private work node. The latter and the enclosing cursor advance
/// in the same transaction, so a crash cannot expose uncheckpointed output.
pub(crate) async fn advance_snapshot_segment_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &mut SnapshotWork<M>,
    now_ms: i64,
) -> Result<()> {
    let work_limits = state.work_limits();
    use protocol::SyncError;
    let namespace = state.source.namespace;
    let expected = serde_json::to_vec(state)?;
    let mut next = state.clone();
    let flush = match (&next.open_segment, &next.pending_context) {
        (Some(_), _) if next.collection_end => true,
        (Some(segment), Some(context)) => segment.context.digest != context.digest,
        _ => false,
    };
    let lease = if flush {
        let required = 10; // namespace lock plus work node, pin and checkpoint with two edges
        if work_limits.max_mutations < required {
            return Err(SyncError::LimitTooSmall { required }.into());
        }
        crate::causal_leases::building_on(tx, namespace, next.cursor.operation_id, now_ms).await?
    } else {
        if work_limits.max_mutations < 3 {
            return Err(SyncError::LimitTooSmall { required: 3 }.into());
        }
        crate::causal_leases::building_on(tx, namespace, next.cursor.operation_id, now_ms).await?
    };
    ensure!(
        lease.build_state.as_deref() == Some(expected.as_slice())
            && lease.generation == next.source.generation.as_bytes(),
        "stale snapshot checkpoint"
    );
    if flush {
        let segment = next.open_segment.take().ok_or(SyncError::StaleWork)?;
        let child = segment.context.manifest.manifest_digest;
        let node = SnapshotSegment {
            kind: "snapshot_segment".into(),
            version: 1,
            segment,
        };
        let payload = serde_json::to_vec(&node)?;
        next.segment_count = next
            .segment_count
            .checked_add(1)
            .ok_or(SyncError::LogicalLimit)?;
        if next.collection_end {
            next.collection_index = next
                .collection_index
                .checked_add(1)
                .ok_or(SyncError::LogicalLimit)?;
            next.lower = vec![0];
            next.collection_end = false;
        }
        next.cursor.position = next
            .cursor
            .position
            .checked_add(1)
            .ok_or(SyncError::LogicalLimit)?;
        let previous = lease
            .retained_root
            .map(|v| <[u8; 32]>::try_from(v).map_err(|_| SyncError::StaleWork))
            .transpose()?;
        let mut limits = work_limits;
        limits.max_mutations -= 1;
        crate::causal_manifests::advance_work_node_on(
            tx,
            crate::causal_manifests::WorkNodeUpdate {
                namespace,
                lease_id: next.cursor.operation_id,
                previous,
                child: Some(child),
                payload: &payload,
                expected_state: Some(&expected),
                next_state: serde_json::to_vec(&next)?,
                now_ms,
                limits,
            },
        )
        .await?;
    } else {
        let context = next.pending_context.take().ok_or(SyncError::StaleWork)?;
        let boundary = next.boundary.take().ok_or(SyncError::StaleWork)?;
        ensure!(
            next.awaiting_union && boundary.done && boundary.lower == next.lower,
            "invalid selected partition"
        );
        let lower = crate::causal_compaction::decode_bound(&boundary.lower, true)?;
        let upper = crate::causal_compaction::decode_bound(&boundary.next, false)?;
        if let Some(segment) = &mut next.open_segment {
            ensure!(
                segment.context.digest == context.digest && segment.range.upper == lower,
                "nonadjacent context coalescing"
            );
            segment.range.upper = upper;
        } else {
            let mut collections: Vec<_> = next.scope.collections.iter().collect();
            collections.sort_by_key(|value| M::collection_tag(value));
            let collection = (*collections
                .get(usize::try_from(next.collection_index)?)
                .ok_or(SyncError::StaleWork)?)
            .clone();
            next.open_segment = Some(protocol::ContextSegment {
                range: protocol::KeyRange {
                    collection,
                    lower,
                    upper,
                },
                context,
            });
        }
        next.lower = boundary.next;
        next.collection_end = next.lower == [2];
        next.awaiting_union = false;
        next.union = None;
        next.cursor.position = next
            .cursor
            .position
            .checked_add(1)
            .ok_or(SyncError::LogicalLimit)?;
        let encoded = serde_json::to_vec(&next)?;
        let required = u64::try_from(encoded.len())?;
        if work_limits.max_bytes < required {
            return Err(SyncError::LimitTooSmall { required }.into());
        }
        crate::causal_leases::checkpoint_on(
            tx,
            namespace,
            next.cursor.operation_id,
            Some(&expected),
            encoded,
            now_ms,
        )
        .await?;
    }
    *state = next;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum SegmentNodeRead {
    Header,
    Body(crate::causal_manifests::WorkNodeHeader),
    Edges(crate::causal_manifests::WorkNodeEdges),
}

/// A bounded selection pass over the private work-node chain. Repeated passes
/// yield segments in ascending key order without keeping an unbounded segment
/// list in a lease checkpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub(crate) struct SnapshotSegmentScan<M: SyncModel> {
    current: Option<protocol::Digest>,
    read: SegmentNodeRead,
    collection: String,
    lower: Vec<u8>,
    best: Option<protocol::ContextSegment<M>>,
    done: bool,
    ordered: bool,
}

impl<M: SyncModel> SnapshotSegmentScan<M> {
    pub(crate) fn ordered(root: protocol::Digest, collection: String) -> Self {
        Self {
            current: Some(root),
            read: SegmentNodeRead::Header,
            collection,
            lower: vec![0],
            best: None,
            done: false,
            ordered: true,
        }
    }
}

pub(crate) fn snapshot_bound(value: Option<Uuid>, lower: bool) -> Vec<u8> {
    match value {
        None if lower => vec![0],
        None => vec![2],
        Some(id) => {
            let mut bytes = vec![1];
            bytes.extend_from_slice(id.as_bytes());
            bytes
        }
    }
}

/// Validate one private node or advance its bounded component cursor.
async fn read_private_node_step_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    digest: protocol::Digest,
    read: &mut SegmentNodeRead,
    max_rows: u64,
    max_bytes: u64,
) -> Result<Option<crate::causal_manifests::WorkNode>> {
    ensure!(max_rows > 0, "private node read has no row allowance");
    if matches!(read, SegmentNodeRead::Header) && max_rows >= 5 && max_bytes >= 4167 {
        return Ok(Some(
            crate::causal_manifests::read_work_node_on(tx, namespace, digest, max_bytes, 5).await?,
        ));
    }
    match read {
        SegmentNodeRead::Header => {
            *read = SegmentNodeRead::Body(
                crate::causal_manifests::work_node_header_on(tx, namespace, digest, max_bytes)
                    .await?,
            );
            Ok(None)
        }
        SegmentNodeRead::Body(header) => {
            *read = SegmentNodeRead::Edges(
                crate::causal_manifests::work_node_body_on(tx, namespace, header, max_bytes)
                    .await?,
            );
            Ok(None)
        }
        SegmentNodeRead::Edges(edges) => {
            crate::causal_manifests::work_node_edge_step_on(tx, namespace, edges, max_bytes).await
        }
    }
}

pub(crate) async fn snapshot_segment_scan_step_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    state: &mut SnapshotSegmentScan<M>,
    limits: WorkLimits,
) -> Result<()> {
    ensure!(
        limits.max_rows_scanned > 0 && !state.done,
        "invalid segment scan"
    );
    let Some(digest) = state.current else {
        state.done = true;
        return Ok(());
    };
    if let Some(node) = read_private_node_step_on(
        tx,
        namespace,
        digest,
        &mut state.read,
        limits.max_rows_scanned,
        limits.max_bytes,
    )
    .await?
    {
        accept_segment_node(state, node)?;
    }
    Ok(())
}

fn accept_segment_node<M: SyncModel>(
    state: &mut SnapshotSegmentScan<M>,
    node: crate::causal_manifests::WorkNode,
) -> Result<()> {
    if node.payload.starts_with(b"{\"kind\":\"snapshot_segment\"") {
        let entry: SnapshotSegment<M> = serde_json::from_slice(&node.payload)?;
        ensure!(
            entry.kind == "snapshot_segment" && entry.version == 1,
            "invalid snapshot segment node"
        );
        let lower = snapshot_bound(entry.segment.range.lower, true);
        let upper = snapshot_bound(entry.segment.range.upper, false);
        ensure!(lower < upper, "invalid snapshot segment bounds");
        if M::collection_tag(&entry.segment.range.collection) == state.collection
            && lower >= state.lower
        {
            if let Some(best) = &state.best {
                ensure!(
                    lower != snapshot_bound(best.range.lower, true),
                    "duplicate snapshot segment boundary"
                );
            }
            if state
                .best
                .as_ref()
                .is_none_or(|best| lower < snapshot_bound(best.range.lower, true))
            {
                state.best = Some(entry.segment);
            }
        }
    }
    state.current = node.previous;
    state.read = SegmentNodeRead::Header;
    if state.current.is_none() || (state.ordered && state.best.is_some()) {
        state.done = true;
    }
    Ok(())
}

/// Incrementally encode a canonical whole-collection partition while checking
/// adjacency and coalescing. Each push materializes only one bounded segment.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartitionEncoder {
    collection: String,
    next: Vec<u8>,
    previous_context: Option<protocol::Digest>,
    count: u64,
    ended: bool,
}
impl PartitionEncoder {
    fn new(collection: String) -> Self {
        Self {
            collection,
            next: vec![0],
            previous_context: None,
            count: 0,
            ended: false,
        }
    }
    fn push<M: SyncModel>(&mut self, segment: &protocol::ContextSegment<M>) -> Result<Vec<u8>> {
        ensure!(
            !self.ended
                && M::collection_tag(&segment.range.collection) == self.collection
                && snapshot_bound(segment.range.lower, true) == self.next
                && self.previous_context != Some(segment.context.digest),
            "invalid ordered partition segment"
        );
        let single =
            protocol::canonical_context_partition(&segment.range, std::slice::from_ref(segment))?;
        const PREFIX: &[u8] = b"{\"segments\":[";
        const SUFFIX: &[u8] = b"]}";
        ensure!(
            single.starts_with(PREFIX) && single.ends_with(SUFFIX),
            "unexpected canonical partition envelope"
        );
        let mut encoded = if self.count == 0 {
            PREFIX.to_vec()
        } else {
            vec![b',']
        };
        encoded.extend_from_slice(&single[PREFIX.len()..single.len() - SUFFIX.len()]);
        self.next = snapshot_bound(segment.range.upper, false);
        self.previous_context = Some(segment.context.digest);
        self.count = self
            .count
            .checked_add(1)
            .ok_or(protocol::SyncError::LogicalLimit)?;
        ensure!(
            self.count <= protocol::MAX_LOGICAL_ITEMS,
            "partition segment limit"
        );
        self.ended = segment.range.upper.is_none();
        Ok(encoded)
    }
    fn finish(&self) -> Result<Vec<u8>> {
        ensure!(
            self.count > 0 && self.ended && self.next == [2],
            "incomplete partition"
        );
        Ok(b"]}".to_vec())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum PartitionPassPhase {
    Count,
    Hash,
    Done,
}

/// Restartable count and hash passes over rooted segment nodes. This state
/// derives the immutable partition reference without materializing its body.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
struct PartitionPass<M: SyncModel> {
    root: protocol::Digest,
    collection: String,
    scan: SnapshotSegmentScan<M>,
    encoder: PartitionEncoder,
    phase: PartitionPassPhase,
    total_bytes: u64,
    counted_segments: u64,
    hash: Option<protocol::PortableHasher>,
    manifest: Option<protocol::ManifestRef>,
}
impl<M: SyncModel> PartitionPass<M> {
    fn new(root: protocol::Digest, collection: String) -> Self {
        Self {
            root,
            collection: collection.clone(),
            scan: SnapshotSegmentScan {
                current: Some(root),
                read: SegmentNodeRead::Header,
                collection: collection.clone(),
                lower: vec![0],
                best: None,
                done: false,
                ordered: true,
            },
            encoder: PartitionEncoder::new(collection),
            phase: PartitionPassPhase::Count,
            total_bytes: 0,
            counted_segments: 0,
            hash: None,
            manifest: None,
        }
    }
    fn restart_scan(&mut self, lower: Vec<u8>, restart: bool) {
        let current = if restart {
            Some(self.root)
        } else {
            self.scan.current
        };
        self.scan = SnapshotSegmentScan {
            current,
            read: SegmentNodeRead::Header,
            collection: self.collection.clone(),
            lower,
            best: None,
            done: false,
            ordered: true,
        };
    }
}

async fn partition_pass_step_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    state: &mut PartitionPass<M>,
    limits: WorkLimits,
) -> Result<()> {
    ensure!(
        !matches!(state.phase, PartitionPassPhase::Done),
        "partition pass already complete"
    );
    let mut prepared = None;
    if !state.scan.done {
        let complete_read = matches!(state.scan.read, SegmentNodeRead::Header)
            && state.scan.current.is_some()
            && limits.max_rows_scanned >= 5
            && limits.max_bytes >= 4167;
        snapshot_segment_scan_step_on(tx, namespace, &mut state.scan, limits).await?;
        if !complete_read || !state.scan.done {
            return Ok(());
        }
        // Reserve the maximum private node size before encoding its segment.
        // If the remainder is too small, checkpoint the scan and encode later.
        let mut encoder = state.encoder.clone();
        let segment = state
            .scan
            .best
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing partition segment"))?;
        let mut bytes = encoder.push(segment)?;
        if encoder.ended {
            bytes.extend(encoder.finish()?);
        }
        if u64::try_from(bytes.len())? > limits.max_bytes - 4167 {
            return Ok(());
        }
        prepared = Some((encoder, bytes));
    }
    let segment = state
        .scan
        .best
        .take()
        .ok_or_else(|| anyhow::anyhow!("missing partition segment"))?;
    let bytes = if let Some((encoder, bytes)) = prepared {
        state.encoder = encoder;
        bytes
    } else {
        let mut bytes = state.encoder.push(&segment)?;
        if state.encoder.ended {
            bytes.extend(state.encoder.finish()?);
        }
        bytes
    };
    let ended = state.encoder.ended;
    if bytes.len() as u64 > limits.max_bytes {
        return Err(protocol::SyncError::LimitTooSmall {
            required: bytes.len() as u64,
        }
        .into());
    }
    match state.phase {
        PartitionPassPhase::Count => {
            state.total_bytes = state
                .total_bytes
                .checked_add(bytes.len() as u64)
                .ok_or(protocol::SyncError::LogicalLimit)?;
            ensure!(
                state.total_bytes <= protocol::MAX_LOGICAL_BYTES,
                "partition exceeds logical limit"
            );
            if ended {
                state.counted_segments = state.encoder.count;
                let mut hash = protocol::PortableHasher::new();
                let tag = b"sync.manifest.content";
                hash.update(&u32::try_from(tag.len())?.to_be_bytes())?;
                hash.update(tag)?;
                hash.update(&u32::try_from(state.total_bytes)?.to_be_bytes())?;
                state.hash = Some(hash);
                state.encoder = PartitionEncoder::new(state.collection.clone());
                state.phase = PartitionPassPhase::Hash;
                state.restart_scan(vec![0], true);
            } else {
                state.restart_scan(state.encoder.next.clone(), false);
            }
        }
        PartitionPassPhase::Hash => {
            state
                .hash
                .as_mut()
                .ok_or(protocol::SyncError::StaleWork)?
                .update(&bytes)?;
            if ended {
                ensure!(
                    state.encoder.count == state.counted_segments,
                    "partition changed during hash"
                );
                let hash = state.hash.as_ref().ok_or(protocol::SyncError::StaleWork)?;
                let tag_len = 4 + b"sync.manifest.content".len() as u64 + 4;
                ensure!(
                    hash.len() == tag_len + state.total_bytes,
                    "partition size changed during hash"
                );
                let mut manifest = protocol::ManifestRef {
                    kind: protocol::ManifestKind::ContextPartition,
                    manifest_digest: [0; 32],
                    content_digest: hash.finish(),
                    encoded_bytes: state.total_bytes,
                    item_count: state.counted_segments,
                    fragment_count: u32::try_from(state.total_bytes.div_ceil(1024))?,
                };
                manifest.manifest_digest = protocol::manifest_digest(&manifest)?;
                state.manifest = Some(manifest);
                state.phase = PartitionPassPhase::Done;
            } else {
                state.restart_scan(state.encoder.next.clone(), false);
            }
        }
        PartitionPassPhase::Done => unreachable!(),
    }
    Ok(())
}

/// Count/hash one collection partition under the pinned source build and
/// checkpoint each bounded scan step with the outer source cursor.
pub(crate) async fn advance_snapshot_partition_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &mut SnapshotWork<M>,
    now_ms: i64,
) -> Result<()> {
    let work_limits = state.work_limits();
    use protocol::SyncError;
    let expected = serde_json::to_vec(state)?;
    let mut next = state.clone();
    ensure!(
        usize::try_from(next.collection_index)? == next.scope.collections.len()
            && next.open_segment.is_none()
            && !next.collection_end,
        "snapshot segments are incomplete"
    );
    if work_limits.max_mutations < 3 {
        return Err(SyncError::LimitTooSmall { required: 3 }.into());
    }
    let checkpoint_bound = u64::try_from(expected.len())?
        .checked_add(8192)
        .ok_or(SyncError::LogicalLimit)?;
    let required_bytes = checkpoint_bound
        .checked_add(8192)
        .ok_or(SyncError::LogicalLimit)?;
    if work_limits.max_bytes < required_bytes {
        return Err(SyncError::LimitTooSmall {
            required: required_bytes,
        }
        .into());
    }
    let namespace = next.source.namespace;
    let lease =
        crate::causal_leases::building_on(tx, namespace, next.cursor.operation_id, now_ms).await?;
    ensure!(
        lease.build_state.as_deref() == Some(expected.as_slice())
            && lease.generation == next.source.generation.as_bytes(),
        "stale snapshot checkpoint"
    );
    if let Some(pass) = &mut next.partition {
        ensure!(
            !matches!(pass.phase, PartitionPassPhase::Done),
            "partition reference awaits publication"
        );
        let mut limits = work_limits;
        limits.max_mutations -= 3;
        limits.max_bytes -= checkpoint_bound;
        partition_pass_step_on(tx, namespace, pass, limits).await?;
    } else {
        let reversal = next.reversal.as_ref().ok_or(SyncError::StaleWork)?;
        ensure!(reversal.done, "snapshot reversal incomplete");
        let mut collections: Vec<_> = next.scope.collections.iter().collect();
        collections.sort_by_key(|value| M::collection_tag(value));
        let collection = collections
            .get(usize::try_from(next.partition_index)?)
            .ok_or(SyncError::StaleWork)?;
        let tag = M::collection_tag(collection);
        let root = *reversal
            .collection_roots
            .get(tag)
            .ok_or(SyncError::StaleWork)?;
        next.partition = Some(PartitionPass::new(root, tag.to_owned()));
    }
    next.cursor.position = next
        .cursor
        .position
        .checked_add(1)
        .ok_or(SyncError::LogicalLimit)?;
    let encoded = serde_json::to_vec(&next)?;
    ensure!(
        u64::try_from(encoded.len())? <= checkpoint_bound,
        "partition checkpoint grew beyond admitted bound"
    );
    crate::causal_leases::checkpoint_on(
        tx,
        namespace,
        next.cursor.operation_id,
        Some(&expected),
        encoded,
        now_ms,
    )
    .await?;
    *state = next;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum PartitionEmissionPhase {
    Reserve,
    Stream,
    VerifyHeader,
    VerifyBody,
    VerifyEdges,
    Verified,
}

/// Private partition emission state. Its reservation stays pinned by the
/// enclosing lease until verification and promotion complete.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
struct PartitionEmission<M: SyncModel> {
    manifest: protocol::ManifestRef,
    root: protocol::Digest,
    collection: String,
    scan: SnapshotSegmentScan<M>,
    encoder: PartitionEncoder,
    phase: PartitionEmissionPhase,
    pending: Vec<u8>,
    generated: u64,
    ordinal: u32,
    written: u64,
    stream_hash: protocol::PortableHasher,
    verification_hash: protocol::PortableHasher,
    verify_ordinal: u32,
    verify_offset: u64,
    verify_header: Option<crate::causal_manifests::ReservedFragmentHeader>,
    edge_after: Option<protocol::Digest>,
    edge_candidate: Option<protocol::Digest>,
    edge_scan: Option<SnapshotSegmentScan<M>>,
    work_root: Option<protocol::Digest>,
}
fn manifest_content_hasher(size: u64) -> Result<protocol::PortableHasher> {
    let tag = b"sync.manifest.content";
    let mut hash = protocol::PortableHasher::new();
    hash.update(&u32::try_from(tag.len())?.to_be_bytes())?;
    hash.update(tag)?;
    hash.update(&u32::try_from(size)?.to_be_bytes())?;
    Ok(hash)
}
impl<M: SyncModel> PartitionEmission<M> {
    fn new(pass: &PartitionPass<M>) -> Result<Self> {
        ensure!(
            matches!(pass.phase, PartitionPassPhase::Done),
            "partition count/hash incomplete"
        );
        let manifest = pass
            .manifest
            .clone()
            .ok_or(protocol::SyncError::StaleWork)?;
        Ok(Self {
            root: pass.root,
            collection: pass.collection.clone(),
            scan: SnapshotSegmentScan {
                current: Some(pass.root),
                read: SegmentNodeRead::Header,
                collection: pass.collection.clone(),
                lower: vec![0],
                best: None,
                done: false,
                ordered: true,
            },
            encoder: PartitionEncoder::new(pass.collection.clone()),
            phase: PartitionEmissionPhase::Reserve,
            pending: vec![],
            generated: 0,
            ordinal: 0,
            written: 0,
            stream_hash: manifest_content_hasher(manifest.encoded_bytes)?,
            verification_hash: manifest_content_hasher(manifest.encoded_bytes)?,
            verify_ordinal: 0,
            verify_offset: 0,
            verify_header: None,
            edge_after: None,
            edge_candidate: None,
            edge_scan: None,
            work_root: None,
            manifest,
        })
    }
    fn reset_scan(&mut self, lower: Vec<u8>) {
        let current = self.scan.current;
        self.scan = SnapshotSegmentScan {
            current,
            read: SegmentNodeRead::Header,
            collection: self.collection.clone(),
            lower,
            best: None,
            done: false,
            ordered: true,
        };
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingReversal {
    source: protocol::Digest,
    payload: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotReversal {
    original_root: protocol::Digest,
    current: Option<protocol::Digest>,
    read: SegmentNodeRead,
    reverse_head: Option<protocol::Digest>,
    collection_roots: std::collections::BTreeMap<String, protocol::Digest>,
    pending: Option<PendingReversal>,
    count: u64,
    done: bool,
}

/// Reverse the interleaved output chain once. Every append pins a wrapper that
/// retains both the original chain and the newly ordered chain, and commits the
/// cursor in the same transaction. Small budgets retain component cursors; larger ones validate one complete node.
pub(crate) async fn advance_snapshot_reversal_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &mut SnapshotWork<M>,
    now_ms: i64,
) -> Result<()> {
    let work_limits = state.work_limits();
    use protocol::SyncError;
    let expected = serde_json::to_vec(state)?;
    let mut next = state.clone();
    ensure!(
        usize::try_from(next.collection_index)? == next.scope.collections.len()
            && next.open_segment.is_none()
            && next.segment_count > 0,
        "snapshot segments incomplete"
    );
    let append = next.reversal.as_ref().is_some_and(|r| r.pending.is_some());
    let required_mutations = if append { 13 } else { 3 };
    if work_limits.max_mutations < required_mutations {
        return Err(SyncError::LimitTooSmall {
            required: required_mutations,
        }
        .into());
    }
    let checkpoint_bound = u64::try_from(expected.len())?
        .checked_add(8192)
        .ok_or(SyncError::LogicalLimit)?;
    let required_bytes = checkpoint_bound
        .checked_add(8192)
        .ok_or(SyncError::LogicalLimit)?;
    if work_limits.max_bytes < required_bytes {
        return Err(SyncError::LimitTooSmall {
            required: required_bytes,
        }
        .into());
    }
    let namespace = next.source.namespace;
    let lease =
        crate::causal_leases::building_on(tx, namespace, next.cursor.operation_id, now_ms).await?;
    ensure!(
        lease.build_state.as_deref() == Some(expected.as_slice())
            && lease.generation == next.source.generation.as_bytes(),
        "stale snapshot checkpoint"
    );
    let mut pin = None;
    if let Some(reversal) = &mut next.reversal {
        ensure!(!reversal.done, "snapshot reversal already complete");
        if let Some(pending) = reversal.pending.take() {
            let reverse = crate::causal_manifests::publish_work_node_on(
                tx,
                namespace,
                reversal.reverse_head,
                Some(pending.source),
                &pending.payload,
                work_limits.max_bytes,
                4,
            )
            .await?;
            let wrapper = crate::causal_manifests::publish_work_node_on(
                tx,
                namespace,
                Some(reversal.original_root),
                Some(reverse),
                b"snapshot-reversal-root-v1",
                work_limits.max_bytes,
                4,
            )
            .await?;
            reversal.reverse_head = Some(reverse);
            let entry: SnapshotSegment<M> = serde_json::from_slice(&pending.payload)?;
            ensure!(
                next.scope
                    .collections
                    .contains(&entry.segment.range.collection),
                "unexpected segment collection"
            );
            reversal.collection_roots.insert(
                M::collection_tag(&entry.segment.range.collection).to_owned(),
                reverse,
            );
            reversal.count = reversal
                .count
                .checked_add(1)
                .ok_or(SyncError::LogicalLimit)?;
            pin = Some(wrapper);
        } else if let Some(digest) = reversal.current {
            if let Some(node) = read_private_node_step_on(
                tx,
                namespace,
                digest,
                &mut reversal.read,
                work_limits.max_rows_scanned,
                work_limits.max_bytes - checkpoint_bound,
            )
            .await?
            {
                if node.payload.starts_with(b"{\"kind\":\"snapshot_segment\"") {
                    let entry: SnapshotSegment<M> = serde_json::from_slice(&node.payload)?;
                    ensure!(
                        entry.kind == "snapshot_segment" && entry.version == 1,
                        "invalid segment node"
                    );
                    reversal.pending = Some(PendingReversal {
                        source: digest,
                        payload: node.payload,
                    });
                }
                reversal.current = node.previous;
                reversal.read = SegmentNodeRead::Header;
            }
        } else {
            ensure!(
                reversal.count == next.segment_count
                    && reversal.reverse_head.is_some()
                    && reversal.collection_roots.len() == next.scope.collections.len(),
                "incomplete segment reversal"
            );
            reversal.done = true;
        }
    } else {
        let root: protocol::Digest = lease
            .retained_root
            .ok_or(SyncError::StaleWork)?
            .try_into()
            .map_err(|_| SyncError::StaleWork)?;
        next.reversal = Some(SnapshotReversal {
            original_root: root,
            current: Some(root),
            read: SegmentNodeRead::Header,
            reverse_head: None,
            collection_roots: std::collections::BTreeMap::new(),
            pending: None,
            count: 0,
            done: false,
        });
    }
    next.cursor.position = next
        .cursor
        .position
        .checked_add(1)
        .ok_or(SyncError::LogicalLimit)?;
    let encoded = serde_json::to_vec(&next)?;
    ensure!(
        u64::try_from(encoded.len())? <= checkpoint_bound,
        "reversal checkpoint grew beyond admitted bound"
    );
    if let Some(root) = pin {
        crate::causal_leases::pin_root_on(tx, namespace, next.cursor.operation_id, root, now_ms)
            .await?;
    }
    crate::causal_leases::checkpoint_on(
        tx,
        namespace,
        next.cursor.operation_id,
        Some(&expected),
        encoded,
        now_ms,
    )
    .await?;
    *state = next;
    Ok(())
}

/// Emit and reread a counted partition into an unserviceable reservation.
/// `Verified` means its fragments match the counted content; dependency-set
/// verification and atomic promotion are separate required steps.
pub(crate) async fn advance_snapshot_partition_emission_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &mut SnapshotWork<M>,
    now_ms: i64,
) -> Result<()> {
    let work_limits = state.work_limits();
    use protocol::SyncError;
    let expected = serde_json::to_vec(state)?;
    let mut next = state.clone();
    let checkpoint_bound = u64::try_from(expected.len())?
        .checked_add(8192)
        .ok_or(SyncError::LogicalLimit)?;
    let required_bytes = checkpoint_bound
        .checked_add(8192)
        .ok_or(SyncError::LogicalLimit)?;
    if work_limits.max_bytes < required_bytes {
        return Err(SyncError::LimitTooSmall {
            required: required_bytes,
        }
        .into());
    }
    let phase = next.partition_emission.as_ref().map(|e| &e.phase);
    let required_mutations = match phase {
        None => 3,
        Some(PartitionEmissionPhase::Reserve) => 11,
        Some(PartitionEmissionPhase::Stream) => 6,
        Some(
            PartitionEmissionPhase::VerifyHeader
            | PartitionEmissionPhase::VerifyBody
            | PartitionEmissionPhase::VerifyEdges,
        ) => 3,
        Some(PartitionEmissionPhase::Verified) => anyhow::bail!("partition already verified"),
    };
    if work_limits.max_mutations < required_mutations {
        return Err(SyncError::LimitTooSmall {
            required: required_mutations,
        }
        .into());
    }
    let namespace = next.source.namespace;
    let request = next.cursor.operation_id;
    let lease = crate::causal_leases::building_on(tx, namespace, request, now_ms).await?;
    ensure!(
        lease.build_state.as_deref() == Some(expected.as_slice())
            && lease.generation == next.source.generation.as_bytes(),
        "stale snapshot checkpoint"
    );
    let mut pin = None;
    if let Some(emission) = &mut next.partition_emission {
        match emission.phase {
            PartitionEmissionPhase::Reserve => {
                crate::causal_manifests::reserve_manifest_on(tx, namespace, &emission.manifest)
                    .await?;
                let previous = lease
                    .retained_root
                    .clone()
                    .map(|v| <[u8; 32]>::try_from(v).map_err(|_| SyncError::StaleWork))
                    .transpose()?;
                let root = crate::causal_manifests::publish_work_node_on(
                    tx,
                    namespace,
                    previous,
                    Some(emission.manifest.manifest_digest),
                    b"partition-reservation-v1",
                    work_limits.max_bytes - checkpoint_bound,
                    4,
                )
                .await?;
                emission.work_root = Some(root);
                emission.phase = PartitionEmissionPhase::Stream;
                pin = Some(root);
            }
            PartitionEmissionPhase::Stream => {
                let fragment_target = if emission.ordinal < emission.manifest.fragment_count {
                    let (start, end) =
                        protocol::fragment_bounds(&emission.manifest, emission.ordinal)?;
                    ensure!(
                        start == emission.written,
                        "partition fragment offset changed"
                    );
                    usize::try_from(end - start)?
                } else {
                    0
                };
                if fragment_target > 0 && emission.pending.len() >= fragment_target {
                    let bytes: Vec<u8> = emission.pending.drain(..fragment_target).collect();
                    let mut fragment = protocol::Fragment {
                        manifest: emission.manifest.clone(),
                        ordinal: emission.ordinal,
                        offset: emission.written,
                        digest: [0; 32],
                        bytes,
                    };
                    fragment.digest = protocol::fragment_digest(&fragment)?;
                    crate::causal_manifests::reserve_manifest_fragment_on(tx, namespace, &fragment)
                        .await?;
                    emission.written = emission
                        .written
                        .checked_add(fragment.bytes.len() as u64)
                        .ok_or(SyncError::LogicalLimit)?;
                    emission.ordinal = emission
                        .ordinal
                        .checked_add(1)
                        .ok_or(SyncError::LogicalLimit)?;
                } else if emission.encoder.ended {
                    ensure!(
                        emission.pending.is_empty()
                            && emission.written == emission.manifest.encoded_bytes
                            && emission.ordinal == emission.manifest.fragment_count
                            && emission.generated == emission.manifest.encoded_bytes
                            && emission.stream_hash.finish() == emission.manifest.content_digest,
                        "partition emission mismatch"
                    );
                    emission.phase = PartitionEmissionPhase::VerifyHeader;
                } else if !emission.scan.done {
                    let mut limits = work_limits;
                    limits.max_bytes -= checkpoint_bound;
                    snapshot_segment_scan_step_on(tx, namespace, &mut emission.scan, limits)
                        .await?;
                } else {
                    let segment = emission
                        .scan
                        .best
                        .take()
                        .ok_or_else(|| anyhow::anyhow!("missing ordered partition segment"))?;
                    crate::causal_manifests::reserve_manifest_edge_on(
                        tx,
                        namespace,
                        &emission.manifest,
                        segment.context.manifest.manifest_digest,
                    )
                    .await?;
                    let mut bytes = emission.encoder.push(&segment)?;
                    if emission.encoder.ended {
                        bytes.extend(emission.encoder.finish()?);
                    }
                    emission.generated = emission
                        .generated
                        .checked_add(bytes.len() as u64)
                        .ok_or(SyncError::LogicalLimit)?;
                    ensure!(
                        emission.generated <= emission.manifest.encoded_bytes,
                        "partition emission overflow"
                    );
                    emission.stream_hash.update(&bytes)?;
                    emission.pending.extend(bytes);
                    if !emission.encoder.ended {
                        emission.reset_scan(emission.encoder.next.clone());
                    }
                }
            }
            PartitionEmissionPhase::VerifyHeader => {
                if emission.verify_ordinal == emission.manifest.fragment_count {
                    ensure!(
                        emission.verify_offset == emission.manifest.encoded_bytes
                            && emission.verification_hash.finish()
                                == emission.manifest.content_digest,
                        "stored partition content mismatch"
                    );
                    emission.phase = PartitionEmissionPhase::VerifyEdges;
                } else {
                    let header = crate::causal_manifests::read_reserved_fragment_header_on(
                        tx,
                        namespace,
                        &emission.manifest,
                        emission.verify_ordinal,
                        work_limits.max_bytes - checkpoint_bound,
                    )
                    .await?;
                    emission.verify_header = Some(header);
                    emission.phase = PartitionEmissionPhase::VerifyBody;
                }
            }
            PartitionEmissionPhase::VerifyBody => {
                let header = emission.verify_header.take().ok_or(SyncError::StaleWork)?;
                let fragment = crate::causal_manifests::read_reserved_fragment_body_on(
                    tx,
                    namespace,
                    &emission.manifest,
                    &header,
                    work_limits.max_bytes - checkpoint_bound,
                )
                .await?;
                let (start, end) =
                    protocol::fragment_bounds(&emission.manifest, emission.verify_ordinal)?;
                ensure!(
                    fragment.offset == start
                        && start == emission.verify_offset
                        && end - start == fragment.bytes.len() as u64,
                    "stored partition fragment bounds mismatch"
                );
                emission.verification_hash.update(&fragment.bytes)?;
                emission.verify_offset = end;
                emission.verify_ordinal += 1;
                emission.phase = PartitionEmissionPhase::VerifyHeader;
            }
            PartitionEmissionPhase::VerifyEdges => {
                if let Some(candidate) = emission.edge_candidate {
                    let scan = emission.edge_scan.as_mut().ok_or(SyncError::StaleWork)?;
                    if !scan.done {
                        let mut limits = work_limits;
                        limits.max_bytes -= checkpoint_bound;
                        snapshot_segment_scan_step_on(tx, namespace, scan, limits).await?;
                    } else {
                        let segment = scan
                            .best
                            .take()
                            .ok_or_else(|| anyhow::anyhow!("unexpected partition dependency"))?;
                        if segment.context.manifest.manifest_digest == candidate {
                            emission.edge_after = Some(candidate);
                            emission.edge_candidate = None;
                            emission.edge_scan = None;
                        } else {
                            ensure!(
                                segment.range.upper.is_some(),
                                "unexpected partition dependency"
                            );
                            let lower = snapshot_bound(segment.range.upper, false);
                            *scan = SnapshotSegmentScan {
                                current: scan.current,
                                read: SegmentNodeRead::Header,
                                collection: emission.collection.clone(),
                                lower,
                                best: None,
                                done: false,
                                ordered: true,
                            };
                        }
                    }
                } else {
                    let mut q = Query::select();
                    q.column(Alias::new("child_manifest"))
                        .from(Alias::new("sync_manifest_edges"))
                        .and_where(
                            Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()),
                        )
                        .and_where(
                            Expr::col(Alias::new("parent_manifest"))
                                .eq(emission.manifest.manifest_digest.to_vec()),
                        );
                    if let Some(after) = emission.edge_after {
                        q.and_where(Expr::col(Alias::new("child_manifest")).gt(after.to_vec()));
                    }
                    q.order_by(Alias::new("child_manifest"), Order::Asc)
                        .limit(1);
                    if let Some(row) = tx.query_one(&q.to_owned()).await? {
                        let candidate: protocol::Digest = row
                            .try_get_by_index::<Vec<u8>>(0)?
                            .try_into()
                            .map_err(|_| SyncError::StaleWork)?;
                        emission.edge_candidate = Some(candidate);
                        emission.edge_scan = Some(SnapshotSegmentScan {
                            current: Some(emission.root),
                            read: SegmentNodeRead::Header,
                            collection: emission.collection.clone(),
                            lower: vec![0],
                            best: None,
                            done: false,
                            ordered: true,
                        });
                    } else {
                        emission.phase = PartitionEmissionPhase::Verified;
                    }
                }
            }
            PartitionEmissionPhase::Verified => unreachable!(),
        }
    } else {
        next.partition_emission = Some(PartitionEmission::new(
            next.partition.as_ref().ok_or(SyncError::StaleWork)?,
        )?);
    }
    next.cursor.position = next
        .cursor
        .position
        .checked_add(1)
        .ok_or(SyncError::LogicalLimit)?;
    let encoded = serde_json::to_vec(&next)?;
    ensure!(
        u64::try_from(encoded.len())? <= checkpoint_bound,
        "partition emission checkpoint exceeded bound"
    );
    if let Some(root) = pin {
        crate::causal_leases::pin_root_on(tx, namespace, request, root, now_ms).await?;
    }
    crate::causal_leases::checkpoint_on(tx, namespace, request, Some(&expected), encoded, now_ms)
        .await?;
    *state = next;
    Ok(())
}

/// Sealed evidence produced only after the complete partition stream and every
/// retained dependency have been checked under the bound source lease.
pub(crate) struct PartitionCompletionProof {
    namespace: Uuid,
    lease: Uuid,
    root: protocol::Digest,
    manifest: protocol::ManifestRef,
}
impl PartitionCompletionProof {
    pub(crate) fn namespace(&self) -> Uuid {
        self.namespace
    }
    pub(crate) fn lease(&self) -> Uuid {
        self.lease
    }
    pub(crate) fn root(&self) -> protocol::Digest {
        self.root
    }
    pub(crate) fn manifest(&self) -> &protocol::ManifestRef {
        &self.manifest
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartitionReferenceNode {
    kind: String,
    version: u32,
    collection: String,
    manifest: protocol::ManifestRef,
}

/// Promote one completely checked partition and append its durable reference to
/// the source build. Promotion, reference rooting, and checkpoint commit as one.
pub(crate) async fn advance_snapshot_partition_promotion_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &mut SnapshotWork<M>,
    now_ms: i64,
) -> Result<()> {
    let work_limits = state.work_limits();
    use protocol::SyncError;
    const MUTATIONS: u64 = 12;
    if work_limits.max_mutations < MUTATIONS {
        return Err(SyncError::LimitTooSmall {
            required: MUTATIONS,
        }
        .into());
    }
    let expected = serde_json::to_vec(state)?;
    let mut next = state.clone();
    let namespace = next.source.namespace;
    let request = next.cursor.operation_id;
    let lease = crate::causal_leases::building_on(tx, namespace, request, now_ms).await?;
    ensure!(
        lease.build_state.as_deref() == Some(expected.as_slice())
            && lease.generation == next.source.generation.as_bytes(),
        "stale snapshot checkpoint"
    );
    let current_root: protocol::Digest = lease
        .retained_root
        .ok_or(SyncError::StaleWork)?
        .try_into()
        .map_err(|_| SyncError::StaleWork)?;
    let emission = next
        .partition_emission
        .as_ref()
        .ok_or(SyncError::StaleWork)?;
    ensure!(
        matches!(emission.phase, PartitionEmissionPhase::Verified)
            && emission.work_root == Some(current_root),
        "partition verification incomplete or unpinned"
    );
    let proof = PartitionCompletionProof {
        namespace,
        lease: request,
        root: current_root,
        manifest: emission.manifest.clone(),
    };
    let manifest =
        crate::causal_manifests::promote_partition_on(tx, namespace, &proof, now_ms).await?;
    let payload = serde_json::to_vec(&PartitionReferenceNode {
        kind: "partition_reference".into(),
        version: 1,
        collection: emission.collection.clone(),
        manifest: manifest.clone(),
    })?;
    if next.partition_base_root.is_none() {
        next.partition_base_root = Some(current_root);
    }
    next.partition_count = next
        .partition_count
        .checked_add(1)
        .ok_or(SyncError::LogicalLimit)?;
    next.partition_index = next
        .partition_index
        .checked_add(1)
        .ok_or(SyncError::LogicalLimit)?;
    next.partition = None;
    next.partition_emission = None;
    next.cursor.position = next
        .cursor
        .position
        .checked_add(1)
        .ok_or(SyncError::LogicalLimit)?;
    let mut limits = work_limits;
    limits.max_mutations -= 3;
    crate::causal_manifests::advance_work_node_on(
        tx,
        crate::causal_manifests::WorkNodeUpdate {
            namespace,
            lease_id: request,
            previous: Some(current_root),
            child: Some(manifest.manifest_digest),
            payload: &payload,
            expected_state: Some(&expected),
            next_state: serde_json::to_vec(&next)?,
            now_ms,
            limits,
        },
    )
    .await?;
    *state = next;
    Ok(())
}

/// Stream canonical ContextRoot entries in stable collection-tag order. The
/// caller supplies the expected tag from its validated scope for each entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RootEncoder {
    scope_digest: protocol::Digest,
    previous_tag: Option<String>,
    count: u32,
}
impl RootEncoder {
    fn new<M: SyncModel>(scope: &protocol::Scope<M>) -> Result<Self> {
        Ok(Self {
            scope_digest: protocol::scope_id(scope)?,
            previous_tag: None,
            count: 0,
        })
    }
    fn push<M: SyncModel>(
        &mut self,
        expected_tag: &str,
        collection: M::Collection,
        manifest: protocol::ManifestRef,
    ) -> Result<Vec<u8>> {
        let tag = M::collection_tag(&collection);
        ensure!(
            tag == expected_tag && self.previous_tag.as_deref().is_none_or(|p| p < tag),
            "invalid root collection order"
        );
        let singleton: protocol::Scope<M> = protocol::Scope {
            collections: [collection.clone()].into_iter().collect(),
        };
        let bytes = protocol::canonical_context_root(&protocol::ContextRoot {
            scope: singleton,
            partitions: vec![protocol::ContextPartitionRef {
                collection,
                partition: manifest,
            }],
        })?;
        const PREFIX: &[u8] = b"{\"partitions\":[";
        const MARKER: &[u8] = b"],\"scope\":";
        ensure!(
            bytes.starts_with(PREFIX),
            "unexpected canonical root envelope"
        );
        let end = bytes
            .windows(MARKER.len())
            .position(|window| window == MARKER)
            .ok_or_else(|| anyhow::anyhow!("invalid root envelope"))?;
        let mut out = if self.count == 0 {
            PREFIX.to_vec()
        } else {
            vec![b',']
        };
        out.extend_from_slice(&bytes[PREFIX.len()..end]);
        self.previous_tag = Some(tag.to_owned());
        self.count = self
            .count
            .checked_add(1)
            .ok_or(protocol::SyncError::LogicalLimit)?;
        Ok(out)
    }
    fn finish(&self, expected_count: usize) -> Result<Vec<u8>> {
        ensure!(
            usize::try_from(self.count)? == expected_count && expected_count > 0,
            "incomplete root coverage"
        );
        let mut suffix = b"],\"scope\":\"".to_vec();
        for byte in self.scope_digest {
            suffix.extend_from_slice(format!("{byte:02x}").as_bytes());
        }
        suffix.extend_from_slice(b"\"}");
        Ok(suffix)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReferenceReversal {
    base: protocol::Digest,
    original_root: protocol::Digest,
    current: Option<protocol::Digest>,
    read: SegmentNodeRead,
    reverse_head: Option<protocol::Digest>,
    pending: Option<PendingReversal>,
    count: u32,
    done: bool,
}

/// Reverse the bounded list of completed collection partition references while
/// retaining both chains until the enclosing source cursor is checkpointed.
pub(crate) async fn advance_snapshot_reference_reversal_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &mut SnapshotWork<M>,
    now_ms: i64,
) -> Result<()> {
    let work_limits = state.work_limits();
    use protocol::SyncError;
    let expected = serde_json::to_vec(state)?;
    let mut next = state.clone();
    ensure!(
        usize::try_from(next.partition_count)? == next.scope.collections.len()
            && next.partition.is_none()
            && next.partition_emission.is_none(),
        "snapshot partitions incomplete"
    );
    let append = next
        .reference_reversal
        .as_ref()
        .is_some_and(|r| r.pending.is_some());
    let required_mutations = if append { 13 } else { 3 };
    if work_limits.max_mutations < required_mutations {
        return Err(SyncError::LimitTooSmall {
            required: required_mutations,
        }
        .into());
    }
    let checkpoint_bound = u64::try_from(expected.len())?
        .checked_add(8192)
        .ok_or(SyncError::LogicalLimit)?;
    let required_bytes = checkpoint_bound
        .checked_add(8192)
        .ok_or(SyncError::LogicalLimit)?;
    if work_limits.max_bytes < required_bytes {
        return Err(SyncError::LimitTooSmall {
            required: required_bytes,
        }
        .into());
    }
    let namespace = next.source.namespace;
    let lease =
        crate::causal_leases::building_on(tx, namespace, next.cursor.operation_id, now_ms).await?;
    ensure!(
        lease.build_state.as_deref() == Some(expected.as_slice())
            && lease.generation == next.source.generation.as_bytes(),
        "stale snapshot checkpoint"
    );
    let mut pin = None;
    if let Some(reversal) = &mut next.reference_reversal {
        ensure!(!reversal.done, "reference reversal already complete");
        if let Some(pending) = reversal.pending.take() {
            let reverse = crate::causal_manifests::publish_work_node_on(
                tx,
                namespace,
                reversal.reverse_head,
                Some(pending.source),
                &pending.payload,
                work_limits.max_bytes,
                4,
            )
            .await?;
            let wrapper = crate::causal_manifests::publish_work_node_on(
                tx,
                namespace,
                Some(reversal.original_root),
                Some(reverse),
                b"partition-reversal-root-v1",
                work_limits.max_bytes,
                4,
            )
            .await?;
            reversal.reverse_head = Some(reverse);
            reversal.count = reversal
                .count
                .checked_add(1)
                .ok_or(SyncError::LogicalLimit)?;
            pin = Some(wrapper);
        } else if let Some(digest) = reversal.current.filter(|digest| *digest != reversal.base) {
            if let Some(node) = read_private_node_step_on(
                tx,
                namespace,
                digest,
                &mut reversal.read,
                work_limits.max_rows_scanned,
                work_limits.max_bytes - checkpoint_bound,
            )
            .await?
            {
                if node
                    .payload
                    .starts_with(b"{\"kind\":\"partition_reference\"")
                {
                    let entry: PartitionReferenceNode = serde_json::from_slice(&node.payload)?;
                    ensure!(
                        entry.kind == "partition_reference" && entry.version == 1,
                        "invalid partition reference node"
                    );
                    reversal.pending = Some(PendingReversal {
                        source: digest,
                        payload: node.payload,
                    });
                }
                reversal.current = node.previous;
                reversal.read = SegmentNodeRead::Header;
            }
        } else {
            ensure!(
                reversal.current == Some(reversal.base)
                    && reversal.count == next.partition_count
                    && reversal.reverse_head.is_some(),
                "incomplete partition reference reversal"
            );
            reversal.done = true;
        }
    } else {
        let original_root: protocol::Digest = lease
            .retained_root
            .ok_or(SyncError::StaleWork)?
            .try_into()
            .map_err(|_| SyncError::StaleWork)?;
        next.reference_reversal = Some(ReferenceReversal {
            base: next.partition_base_root.ok_or(SyncError::StaleWork)?,
            original_root,
            current: Some(original_root),
            read: SegmentNodeRead::Header,
            reverse_head: None,
            pending: None,
            count: 0,
            done: false,
        });
    }
    next.cursor.position = next
        .cursor
        .position
        .checked_add(1)
        .ok_or(SyncError::LogicalLimit)?;
    let encoded = serde_json::to_vec(&next)?;
    ensure!(
        u64::try_from(encoded.len())? <= checkpoint_bound,
        "reference reversal checkpoint exceeded bound"
    );
    if let Some(root) = pin {
        crate::causal_leases::pin_root_on(tx, namespace, next.cursor.operation_id, root, now_ms)
            .await?;
    }
    crate::causal_leases::checkpoint_on(
        tx,
        namespace,
        next.cursor.operation_id,
        Some(&expected),
        encoded,
        now_ms,
    )
    .await?;
    *state = next;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RootPass {
    chain: protocol::Digest,
    current: Option<protocol::Digest>,
    read: SegmentNodeRead,
    encoder: RootEncoder,
    phase: PartitionPassPhase,
    total_bytes: u64,
    hash: Option<protocol::PortableHasher>,
    manifest: Option<protocol::ManifestRef>,
}
impl RootPass {
    fn new<M: SyncModel>(chain: protocol::Digest, scope: &protocol::Scope<M>) -> Result<Self> {
        Ok(Self {
            chain,
            current: Some(chain),
            read: SegmentNodeRead::Header,
            encoder: RootEncoder::new(scope)?,
            phase: PartitionPassPhase::Count,
            total_bytes: 0,
            hash: None,
            manifest: None,
        })
    }
}

async fn root_pass_step_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    scope: &protocol::Scope<M>,
    state: &mut RootPass,
    limits: WorkLimits,
) -> Result<()> {
    use protocol::SyncError;
    ensure!(
        !matches!(state.phase, PartitionPassPhase::Done) && limits.max_rows_scanned > 0,
        "invalid root pass"
    );
    if let Some(digest) = state.current {
        if let Some(node) = read_private_node_step_on(
            tx,
            namespace,
            digest,
            &mut state.read,
            limits.max_rows_scanned,
            limits.max_bytes,
        )
        .await?
        {
            let entry: PartitionReferenceNode = serde_json::from_slice(&node.payload)?;
            ensure!(
                entry.kind == "partition_reference" && entry.version == 1,
                "invalid ordered partition reference"
            );
            let mut collections: Vec<_> = scope.collections.iter().collect();
            collections.sort_by_key(|value| M::collection_tag(value));
            let expected = collections
                .get(usize::try_from(state.encoder.count)?)
                .ok_or(SyncError::StaleWork)?;
            let bytes = state.encoder.push::<M>(
                M::collection_tag(expected),
                (*expected).clone(),
                entry.manifest,
            )?;
            ensure!(
                entry.collection == M::collection_tag(expected),
                "root partition collection mismatch"
            );
            if bytes.len() as u64 > limits.max_bytes {
                return Err(SyncError::LimitTooSmall {
                    required: bytes.len() as u64,
                }
                .into());
            }
            match state.phase {
                PartitionPassPhase::Count => {
                    state.total_bytes = state
                        .total_bytes
                        .checked_add(bytes.len() as u64)
                        .ok_or(SyncError::LogicalLimit)?
                }
                PartitionPassPhase::Hash => state
                    .hash
                    .as_mut()
                    .ok_or(SyncError::StaleWork)?
                    .update(&bytes)?,
                PartitionPassPhase::Done => unreachable!(),
            }
            state.current = node.previous;
            state.read = SegmentNodeRead::Header;
        }
    } else {
        let suffix = state.encoder.finish(scope.collections.len())?;
        match state.phase {
            PartitionPassPhase::Count => {
                state.total_bytes = state
                    .total_bytes
                    .checked_add(suffix.len() as u64)
                    .ok_or(SyncError::LogicalLimit)?;
                ensure!(
                    state.total_bytes <= protocol::MAX_LOGICAL_BYTES,
                    "root exceeds logical limit"
                );
                state.hash = Some(manifest_content_hasher(state.total_bytes)?);
                state.encoder = RootEncoder::new(scope)?;
                state.phase = PartitionPassPhase::Hash;
                state.current = Some(state.chain);
            }
            PartitionPassPhase::Hash => {
                let hash = state.hash.as_mut().ok_or(SyncError::StaleWork)?;
                hash.update(&suffix)?;
                let tag_len = 4 + b"sync.manifest.content".len() as u64 + 4;
                ensure!(
                    hash.len() == tag_len + state.total_bytes,
                    "root size changed during hash"
                );
                let mut manifest = protocol::ManifestRef {
                    kind: protocol::ManifestKind::ContextRoot,
                    manifest_digest: [0; 32],
                    content_digest: hash.finish(),
                    encoded_bytes: state.total_bytes,
                    item_count: u64::try_from(scope.collections.len())?,
                    fragment_count: u32::try_from(state.total_bytes.div_ceil(1024))?,
                };
                manifest.manifest_digest = protocol::manifest_digest(&manifest)?;
                state.manifest = Some(manifest);
                state.phase = PartitionPassPhase::Done;
            }
            PartitionPassPhase::Done => unreachable!(),
        }
    }
    Ok(())
}

/// Count/hash the canonical root over the ordered private partition-reference
/// chain and checkpoint each bounded scan in the enclosing source lease.
pub(crate) async fn advance_snapshot_root_pass_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &mut SnapshotWork<M>,
    now_ms: i64,
) -> Result<()> {
    let work_limits = state.work_limits();
    use protocol::SyncError;
    let expected = serde_json::to_vec(state)?;
    let mut next = state.clone();
    if work_limits.max_mutations < 3 {
        return Err(SyncError::LimitTooSmall { required: 3 }.into());
    }
    let checkpoint_bound = u64::try_from(expected.len())?
        .checked_add(8192)
        .ok_or(SyncError::LogicalLimit)?;
    let required_bytes = checkpoint_bound
        .checked_add(8192)
        .ok_or(SyncError::LogicalLimit)?;
    if work_limits.max_bytes < required_bytes {
        return Err(SyncError::LimitTooSmall {
            required: required_bytes,
        }
        .into());
    }
    let namespace = next.source.namespace;
    let lease =
        crate::causal_leases::building_on(tx, namespace, next.cursor.operation_id, now_ms).await?;
    ensure!(
        lease.build_state.as_deref() == Some(expected.as_slice())
            && lease.generation == next.source.generation.as_bytes(),
        "stale snapshot checkpoint"
    );
    if let Some(pass) = &mut next.root_pass {
        ensure!(
            !matches!(pass.phase, PartitionPassPhase::Done),
            "root reference awaits publication"
        );
        let mut limits = work_limits;
        limits.max_mutations -= 3;
        limits.max_bytes -= checkpoint_bound;
        root_pass_step_on(tx, namespace, &next.scope, pass, limits).await?;
    } else {
        let reversal = next
            .reference_reversal
            .as_ref()
            .ok_or(SyncError::StaleWork)?;
        ensure!(reversal.done, "partition reference reversal incomplete");
        next.root_pass = Some(RootPass::new(
            reversal.reverse_head.ok_or(SyncError::StaleWork)?,
            &next.scope,
        )?);
    }
    next.cursor.position = next
        .cursor
        .position
        .checked_add(1)
        .ok_or(SyncError::LogicalLimit)?;
    let encoded = serde_json::to_vec(&next)?;
    ensure!(
        u64::try_from(encoded.len())? <= checkpoint_bound,
        "root checkpoint exceeded bound"
    );
    crate::causal_leases::checkpoint_on(
        tx,
        namespace,
        next.cursor.operation_id,
        Some(&expected),
        encoded,
        now_ms,
    )
    .await?;
    *state = next;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RootEmission {
    manifest: protocol::ManifestRef,
    current: Option<protocol::Digest>,
    read: SegmentNodeRead,
    encoder: RootEncoder,
    phase: PartitionEmissionPhase,
    pending: Vec<u8>,
    generated: u64,
    ordinal: u32,
    written: u64,
    stream_hash: protocol::PortableHasher,
    verification_hash: protocol::PortableHasher,
    verify_ordinal: u32,
    verify_offset: u64,
    verify_header: Option<crate::causal_manifests::ReservedFragmentHeader>,
    edge_after: Option<protocol::Digest>,
    unverified_edges: std::collections::BTreeSet<protocol::Digest>,
    work_root: Option<protocol::Digest>,
    stream_finished: bool,
}
impl RootEmission {
    fn new<M: SyncModel>(pass: &RootPass, scope: &protocol::Scope<M>) -> Result<Self> {
        ensure!(
            matches!(pass.phase, PartitionPassPhase::Done),
            "root count/hash incomplete"
        );
        let manifest = pass
            .manifest
            .clone()
            .ok_or(protocol::SyncError::StaleWork)?;
        Ok(Self {
            current: Some(pass.chain),
            read: SegmentNodeRead::Header,
            encoder: RootEncoder::new(scope)?,
            phase: PartitionEmissionPhase::Reserve,
            pending: vec![],
            generated: 0,
            ordinal: 0,
            written: 0,
            stream_hash: manifest_content_hasher(manifest.encoded_bytes)?,
            verification_hash: manifest_content_hasher(manifest.encoded_bytes)?,
            verify_ordinal: 0,
            verify_offset: 0,
            verify_header: None,
            edge_after: None,
            unverified_edges: std::collections::BTreeSet::new(),
            work_root: None,
            stream_finished: false,
            manifest,
        })
    }
}

/// Emit and reread the canonical ContextRoot under a private reservation.
pub(crate) async fn advance_snapshot_root_emission_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &mut SnapshotWork<M>,
    now_ms: i64,
) -> Result<()> {
    let work_limits = state.work_limits();
    use protocol::SyncError;
    let expected = serde_json::to_vec(state)?;
    let mut next = state.clone();
    let checkpoint_bound = u64::try_from(expected.len())?
        .checked_add(8192)
        .ok_or(SyncError::LogicalLimit)?;
    let required_bytes = checkpoint_bound
        .checked_add(8192)
        .ok_or(SyncError::LogicalLimit)?;
    if work_limits.max_bytes < required_bytes {
        return Err(SyncError::LimitTooSmall {
            required: required_bytes,
        }
        .into());
    }
    let required_mutations = match next.root_emission.as_ref().map(|e| &e.phase) {
        None => 3,
        Some(PartitionEmissionPhase::Reserve) => 11,
        Some(PartitionEmissionPhase::Stream) => 6,
        Some(
            PartitionEmissionPhase::VerifyHeader
            | PartitionEmissionPhase::VerifyBody
            | PartitionEmissionPhase::VerifyEdges,
        ) => 3,
        Some(PartitionEmissionPhase::Verified) => anyhow::bail!("root already verified"),
    };
    if work_limits.max_mutations < required_mutations {
        return Err(SyncError::LimitTooSmall {
            required: required_mutations,
        }
        .into());
    }
    let namespace = next.source.namespace;
    let request = next.cursor.operation_id;
    let lease = crate::causal_leases::building_on(tx, namespace, request, now_ms).await?;
    ensure!(
        lease.build_state.as_deref() == Some(expected.as_slice())
            && lease.generation == next.source.generation.as_bytes(),
        "stale snapshot checkpoint"
    );
    let mut pin = None;
    if let Some(emission) = &mut next.root_emission {
        match emission.phase {
            PartitionEmissionPhase::Reserve => {
                crate::causal_manifests::reserve_manifest_on(tx, namespace, &emission.manifest)
                    .await?;
                let previous = lease
                    .retained_root
                    .clone()
                    .map(|v| <[u8; 32]>::try_from(v).map_err(|_| SyncError::StaleWork))
                    .transpose()?;
                let root = crate::causal_manifests::publish_work_node_on(
                    tx,
                    namespace,
                    previous,
                    Some(emission.manifest.manifest_digest),
                    b"root-reservation-v1",
                    work_limits.max_bytes - checkpoint_bound,
                    4,
                )
                .await?;
                emission.work_root = Some(root);
                emission.phase = PartitionEmissionPhase::Stream;
                pin = Some(root);
            }
            PartitionEmissionPhase::Stream => {
                let fragment_target = if emission.ordinal < emission.manifest.fragment_count {
                    let (start, end) =
                        protocol::fragment_bounds(&emission.manifest, emission.ordinal)?;
                    ensure!(start == emission.written, "root fragment offset changed");
                    usize::try_from(end - start)?
                } else {
                    0
                };
                if fragment_target > 0 && emission.pending.len() >= fragment_target {
                    let bytes: Vec<u8> = emission.pending.drain(..fragment_target).collect();
                    let mut fragment = protocol::Fragment {
                        manifest: emission.manifest.clone(),
                        ordinal: emission.ordinal,
                        offset: emission.written,
                        digest: [0; 32],
                        bytes,
                    };
                    fragment.digest = protocol::fragment_digest(&fragment)?;
                    crate::causal_manifests::reserve_manifest_fragment_on(tx, namespace, &fragment)
                        .await?;
                    emission.written = emission
                        .written
                        .checked_add(fragment.bytes.len() as u64)
                        .ok_or(SyncError::LogicalLimit)?;
                    emission.ordinal = emission
                        .ordinal
                        .checked_add(1)
                        .ok_or(SyncError::LogicalLimit)?;
                } else if emission.stream_finished {
                    ensure!(
                        emission.pending.is_empty()
                            && emission.written == emission.manifest.encoded_bytes
                            && emission.ordinal == emission.manifest.fragment_count
                            && emission.generated == emission.manifest.encoded_bytes
                            && emission.stream_hash.finish() == emission.manifest.content_digest,
                        "root emission mismatch"
                    );
                    emission.phase = PartitionEmissionPhase::VerifyHeader;
                } else if let Some(digest) = emission.current {
                    if let Some(node) = read_private_node_step_on(
                        tx,
                        namespace,
                        digest,
                        &mut emission.read,
                        work_limits.max_rows_scanned,
                        work_limits.max_bytes - checkpoint_bound,
                    )
                    .await?
                    {
                        let entry: PartitionReferenceNode = serde_json::from_slice(&node.payload)?;
                        ensure!(
                            entry.kind == "partition_reference" && entry.version == 1,
                            "invalid ordered partition reference"
                        );
                        let mut collections: Vec<_> = next.scope.collections.iter().collect();
                        collections.sort_by_key(|v| M::collection_tag(v));
                        let collection = collections
                            .get(usize::try_from(emission.encoder.count)?)
                            .ok_or(SyncError::StaleWork)?;
                        ensure!(
                            entry.collection == M::collection_tag(collection),
                            "root collection mismatch"
                        );
                        crate::causal_manifests::reserve_manifest_edge_on(
                            tx,
                            namespace,
                            &emission.manifest,
                            entry.manifest.manifest_digest,
                        )
                        .await?;
                        ensure!(
                            emission
                                .unverified_edges
                                .insert(entry.manifest.manifest_digest),
                            "duplicate root dependency"
                        );
                        ensure!(
                            emission.unverified_edges.len() <= next.scope.collections.len(),
                            "root dependency bound exceeded"
                        );
                        let bytes = emission.encoder.push::<M>(
                            M::collection_tag(collection),
                            (*collection).clone(),
                            entry.manifest,
                        )?;
                        emission.generated = emission
                            .generated
                            .checked_add(bytes.len() as u64)
                            .ok_or(SyncError::LogicalLimit)?;
                        ensure!(
                            emission.generated <= emission.manifest.encoded_bytes,
                            "root emission overflow"
                        );
                        emission.stream_hash.update(&bytes)?;
                        emission.pending.extend(bytes);
                        emission.current = node.previous;
                        emission.read = SegmentNodeRead::Header;
                    }
                } else {
                    let suffix = emission.encoder.finish(next.scope.collections.len())?;
                    emission.generated = emission
                        .generated
                        .checked_add(suffix.len() as u64)
                        .ok_or(SyncError::LogicalLimit)?;
                    emission.stream_hash.update(&suffix)?;
                    emission.pending.extend(suffix);
                    emission.stream_finished = true;
                }
            }
            PartitionEmissionPhase::VerifyHeader => {
                if emission.verify_ordinal == emission.manifest.fragment_count {
                    ensure!(
                        emission.verify_offset == emission.manifest.encoded_bytes
                            && emission.verification_hash.finish()
                                == emission.manifest.content_digest,
                        "stored root content mismatch"
                    );
                    emission.phase = PartitionEmissionPhase::VerifyEdges;
                } else {
                    emission.verify_header = Some(
                        crate::causal_manifests::read_reserved_fragment_header_on(
                            tx,
                            namespace,
                            &emission.manifest,
                            emission.verify_ordinal,
                            work_limits.max_bytes - checkpoint_bound,
                        )
                        .await?,
                    );
                    emission.phase = PartitionEmissionPhase::VerifyBody;
                }
            }
            PartitionEmissionPhase::VerifyBody => {
                let header = emission.verify_header.take().ok_or(SyncError::StaleWork)?;
                let fragment = crate::causal_manifests::read_reserved_fragment_body_on(
                    tx,
                    namespace,
                    &emission.manifest,
                    &header,
                    work_limits.max_bytes - checkpoint_bound,
                )
                .await?;
                let (start, end) =
                    protocol::fragment_bounds(&emission.manifest, emission.verify_ordinal)?;
                ensure!(
                    fragment.offset == start
                        && start == emission.verify_offset
                        && end - start == fragment.bytes.len() as u64,
                    "stored root fragment bounds mismatch"
                );
                emission.verification_hash.update(&fragment.bytes)?;
                emission.verify_offset = end;
                emission.verify_ordinal += 1;
                emission.phase = PartitionEmissionPhase::VerifyHeader;
            }
            PartitionEmissionPhase::VerifyEdges => {
                // The authenticated root stream contributes at most one dependency
                // per declared collection. Verify the persisted set in one ordered
                // pass without rescanning the complete reference chain per edge.
                let mut q = Query::select();
                q.column(Alias::new("child_manifest"))
                    .from(Alias::new("sync_manifest_edges"))
                    .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
                    .and_where(
                        Expr::col(Alias::new("parent_manifest"))
                            .eq(emission.manifest.manifest_digest.to_vec()),
                    );
                if let Some(after) = emission.edge_after {
                    q.and_where(Expr::col(Alias::new("child_manifest")).gt(after.to_vec()));
                }
                q.order_by(Alias::new("child_manifest"), Order::Asc)
                    .limit(1);
                if let Some(row) = tx.query_one(&q.to_owned()).await? {
                    let candidate: protocol::Digest = row
                        .try_get_by_index::<Vec<u8>>(0)?
                        .try_into()
                        .map_err(|_| SyncError::StaleWork)?;
                    ensure!(
                        emission.unverified_edges.remove(&candidate),
                        "unexpected root dependency"
                    );
                    emission.edge_after = Some(candidate);
                } else {
                    ensure!(
                        emission.unverified_edges.is_empty(),
                        "missing root dependency"
                    );
                    emission.phase = PartitionEmissionPhase::Verified;
                }
            }
            PartitionEmissionPhase::Verified => unreachable!(),
        }
    } else {
        next.root_emission = Some(RootEmission::new(
            next.root_pass.as_ref().ok_or(SyncError::StaleWork)?,
            &next.scope,
        )?);
    }
    next.cursor.position = next
        .cursor
        .position
        .checked_add(1)
        .ok_or(SyncError::LogicalLimit)?;
    let encoded = serde_json::to_vec(&next)?;
    ensure!(
        u64::try_from(encoded.len())? <= checkpoint_bound,
        "root emission checkpoint exceeded bound"
    );
    if let Some(root) = pin {
        crate::causal_leases::pin_root_on(tx, namespace, request, root, now_ms).await?;
    }
    crate::causal_leases::checkpoint_on(tx, namespace, request, Some(&expected), encoded, now_ms)
        .await?;
    *state = next;
    Ok(())
}

/// Sealed ContextRoot promotion evidence, bound to the verified source lease.
pub(crate) struct RootCompletionProof {
    namespace: Uuid,
    lease: Uuid,
    root: protocol::Digest,
    manifest: protocol::ManifestRef,
}
impl RootCompletionProof {
    pub(crate) fn namespace(&self) -> Uuid {
        self.namespace
    }
    pub(crate) fn lease(&self) -> Uuid {
        self.lease
    }
    pub(crate) fn root(&self) -> protocol::Digest {
        self.root
    }
    pub(crate) fn manifest(&self) -> &protocol::ManifestRef {
        &self.manifest
    }
}

/// Promote a completely verified ContextRoot while preserving its enclosing
/// work root and checkpoint for subsequent summary construction.
pub(crate) async fn advance_snapshot_root_promotion_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &mut SnapshotWork<M>,
    now_ms: i64,
) -> Result<()> {
    let work_limits = state.work_limits();
    use protocol::SyncError;
    const REQUIRED_MUTATIONS: u64 = 5;
    if work_limits.max_mutations < REQUIRED_MUTATIONS {
        return Err(SyncError::LimitTooSmall {
            required: REQUIRED_MUTATIONS,
        }
        .into());
    }
    ensure!(!state.root_promoted, "snapshot root already promoted");
    let expected = serde_json::to_vec(state)?;
    let mut next = state.clone();
    let namespace = next.source.namespace;
    let request = next.cursor.operation_id;
    let lease = crate::causal_leases::building_on(tx, namespace, request, now_ms).await?;
    ensure!(
        lease.build_state.as_deref() == Some(expected.as_slice())
            && lease.generation == next.source.generation.as_bytes(),
        "stale snapshot checkpoint"
    );
    let root: protocol::Digest = lease
        .retained_root
        .ok_or(SyncError::StaleWork)?
        .try_into()
        .map_err(|_| SyncError::StaleWork)?;
    let emission = next.root_emission.as_ref().ok_or(SyncError::StaleWork)?;
    ensure!(
        matches!(emission.phase, PartitionEmissionPhase::Verified)
            && emission.work_root == Some(root),
        "root verification incomplete or unpinned"
    );
    let proof = RootCompletionProof {
        namespace,
        lease: request,
        root,
        manifest: emission.manifest.clone(),
    };
    crate::causal_manifests::promote_root_on(tx, namespace, &proof, now_ms).await?;
    next.root_promoted = true;
    next.cursor.position = next
        .cursor
        .position
        .checked_add(1)
        .ok_or(SyncError::LogicalLimit)?;
    let encoded = serde_json::to_vec(&next)?;
    let required = u64::try_from(encoded.len())?;
    if work_limits.max_bytes < required {
        return Err(SyncError::LimitTooSmall { required }.into());
    }
    crate::causal_leases::checkpoint_on(tx, namespace, request, Some(&expected), encoded, now_ms)
        .await?;
    *state = next;
    Ok(())
}

/// Sealed evidence of a fully verified source root and ordered segment chain.
/// It does not assert that a new baseline has been staged or published.
pub(crate) struct CompletedSourceProof {
    namespace: Uuid,
    lease: Uuid,
    generation: Uuid,
    floor: i64,
    basis: Uuid,
    target: i64,
    head: i64,
    anchor: protocol::Digest,
    root: protocol::ManifestRef,
    scope: protocol::Digest,
}
impl CompletedSourceProof {
    pub(crate) fn namespace(&self) -> Uuid {
        self.namespace
    }
    pub(crate) fn lease(&self) -> Uuid {
        self.lease
    }
    pub(crate) fn generation(&self) -> Uuid {
        self.generation
    }
    pub(crate) fn floor(&self) -> i64 {
        self.floor
    }
    pub(crate) fn basis(&self) -> Uuid {
        self.basis
    }
    pub(crate) fn target(&self) -> i64 {
        self.target
    }
    pub(crate) fn head(&self) -> i64 {
        self.head
    }
    pub(crate) fn anchor(&self) -> protocol::Digest {
        self.anchor
    }
    pub(crate) fn root(&self) -> &protocol::ManifestRef {
        &self.root
    }
    pub(crate) fn scope(&self) -> protocol::Digest {
        self.scope
    }
}

/// Check the complete saved source cursor and lease before exposing its
/// verified root and ordered segment iterator to bounded baseline staging.
pub(crate) async fn completed_source_proof_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &SnapshotWork<M>,
    now_ms: i64,
) -> Result<CompletedSourceProof> {
    use protocol::SyncError;
    ensure!(state.root_promoted, "source root is not complete");
    let namespace = state.source.namespace;
    let lease =
        crate::causal_leases::building_on(tx, namespace, state.cursor.operation_id, now_ms).await?;
    ensure!(
        lease.build_state.as_deref() == Some(serde_json::to_vec(state)?.as_slice())
            && lease.generation == state.source.generation.as_bytes(),
        "stale source proof"
    );
    let emission = state.root_emission.as_ref().ok_or(SyncError::StaleWork)?;
    let work_root: protocol::Digest = lease
        .retained_root
        .clone()
        .ok_or(SyncError::StaleWork)?
        .try_into()
        .map_err(|_| SyncError::StaleWork)?;
    ensure!(
        matches!(emission.phase, PartitionEmissionPhase::Verified)
            && emission.work_root == Some(work_root),
        "source root proof mismatch"
    );
    let reversal = state.reversal.as_ref().ok_or(SyncError::StaleWork)?;
    ensure!(
        reversal.done && reversal.count == state.segment_count && reversal.reverse_head.is_some(),
        "ordered source segments incomplete"
    );
    let q = Query::select()
        .column(Alias::new("kind"))
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("digest")).eq(emission.manifest.manifest_digest.to_vec()))
        .to_owned();
    let row = tx.query_one(&q).await?.ok_or(SyncError::StaleWork)?;
    ensure!(
        row.try_get_by_index::<i64>(0)? == 5
            && emission.manifest.kind == protocol::ManifestKind::ContextRoot,
        "source root is not published"
    );
    Ok(CompletedSourceProof {
        namespace,
        lease: state.cursor.operation_id,
        generation: state.source.generation,
        floor: lease.basis_state_seq.ok_or(SyncError::StaleWork)?,
        basis: Uuid::from_slice(
            lease
                .basis_generation
                .as_deref()
                .ok_or(SyncError::StaleWork)?,
        )?,
        target: lease.state_seq,
        head: lease.head,
        anchor: lease
            .head_anchor
            .try_into()
            .map_err(|_| SyncError::StaleWork)?,
        root: emission.manifest.clone(),
        scope: protocol::scope_id(&state.scope)?,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum SummaryPhase {
    Count,
    Segments,
    Candidates,
    Done,
}

/// Resumable root-range summary for one collection at captured state S.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
struct SummaryPass<M: SyncModel> {
    snapshot: Uuid,
    range: protocol::KeyRange<M>,
    segment_count: u64,
    count: CandidateCount,
    segment_scan: SnapshotSegmentScan<M>,
    hasher: Option<Vec<u8>>,
    candidates: Option<CandidateHash>,
    phase: SummaryPhase,
    result: Option<protocol::RangeSummary<M>>,
}
impl<M: SyncModel> SummaryPass<M> {
    fn new(
        snapshot: Uuid,
        collection: M::Collection,
        segment_count: u64,
        ordered_root: protocol::Digest,
    ) -> Result<Self> {
        ensure!(segment_count > 0, "empty summary partition");
        let tag = M::collection_tag(&collection).to_owned();
        Ok(Self {
            snapshot,
            range: protocol::KeyRange {
                collection,
                lower: None,
                upper: None,
            },
            segment_count,
            count: CandidateCount::default(),
            segment_scan: SnapshotSegmentScan::ordered(ordered_root, tag),
            hasher: None,
            candidates: None,
            phase: SummaryPhase::Count,
            result: None,
        })
    }
}

async fn summary_pass_step_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    scope: &protocol::Scope<M>,
    state_seq: i64,
    state: &mut SummaryPass<M>,
    limits: WorkLimits,
) -> Result<()> {
    use protocol::SyncError;
    match state.phase {
        SummaryPhase::Count => {
            scan_candidate_headers_on(
                tx,
                namespace,
                &state.range,
                state_seq,
                &mut state.count,
                limits,
                |_, _| Ok(true),
            )
            .await?;
            if state.count.done {
                let hasher = protocol::RangeHasher::<M>::new(
                    scope,
                    state.range.clone(),
                    state.count.count,
                    state.count.bytes,
                    state.segment_count,
                )?;
                state.hasher = Some(hasher.checkpoint()?);
                state.phase = SummaryPhase::Segments;
            }
        }
        SummaryPhase::Segments => {
            if !state.segment_scan.done {
                snapshot_segment_scan_step_on(tx, namespace, &mut state.segment_scan, limits)
                    .await?;
            } else {
                let segment = state
                    .segment_scan
                    .best
                    .take()
                    .ok_or_else(|| anyhow::anyhow!("missing summary segment"))?;
                let mut hasher = protocol::RangeHasher::<M>::from_checkpoint(
                    scope,
                    &state.range,
                    state.hasher.as_deref().ok_or(SyncError::StaleWork)?,
                )?;
                hasher.push_segment(&segment.range, segment.context.digest)?;
                state.hasher = Some(hasher.checkpoint()?);
                if segment.range.upper.is_none() {
                    state.candidates = Some(CandidateHash::new(&hasher)?);
                    state.hasher = None;
                    state.phase = SummaryPhase::Candidates;
                } else {
                    let lower = snapshot_bound(segment.range.upper, false);
                    state.segment_scan = SnapshotSegmentScan {
                        current: state.segment_scan.current,
                        read: SegmentNodeRead::Header,
                        collection: M::collection_tag(&state.range.collection).to_owned(),
                        lower,
                        best: None,
                        done: false,
                        ordered: true,
                    };
                }
            }
        }
        SummaryPhase::Candidates => {
            let candidates = state.candidates.as_mut().ok_or(SyncError::StaleWork)?;
            hash_candidate_step_on(
                tx,
                namespace,
                scope,
                &state.range,
                state_seq,
                candidates,
                limits,
            )
            .await?;
            if candidates.done() {
                let (aggregate, context_commitment) =
                    candidates.clone().finish(scope, &state.range)?;
                state.result = Some(protocol::RangeSummary {
                    snapshot_id: state.snapshot,
                    range: state.range.clone(),
                    candidate_count: state.count.count,
                    candidate_bytes: state.count.bytes,
                    context_segments: state.segment_count,
                    context_commitment,
                    aggregate,
                });
                state.phase = SummaryPhase::Done;
            }
        }
        SummaryPhase::Done => anyhow::bail!("summary already complete"),
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SummaryReferenceScan {
    current: Option<protocol::Digest>,
    read: SegmentNodeRead,
    pending: Option<PartitionReferenceNode>,
}

fn descriptor_envelope<M: SyncModel>(
    state: &SnapshotWork<M>,
    lease: &crate::causal_leases::Lease,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let descriptor = protocol::SnapshotDescriptor::<M> {
        source: state.source.clone(),
        scope: state.scope.clone(),
        snapshot_id: state.cursor.operation_id,
        head: protocol::ReceiveProgress {
            sequence: u64::try_from(lease.head)?,
            anchor: lease
                .head_anchor
                .clone()
                .try_into()
                .map_err(|_| protocol::SyncError::StaleWork)?,
        },
        roots: vec![],
        expires_unix_ms: u64::try_from(lease.expires_at_ms)?,
    };
    let bytes = serde_json::to_vec(&descriptor)?;
    const MARKER: &[u8] = b"\"roots\":[]";
    let position = bytes
        .windows(MARKER.len())
        .position(|window| window == MARKER)
        .ok_or_else(|| anyhow::anyhow!("snapshot descriptor envelope missing"))?;
    let split = position + MARKER.len() - 1;
    Ok((bytes[..split].to_vec(), bytes[split..].to_vec()))
}

/// Append root summaries to an unserviceable descriptor, one bounded source
/// step at a time, with every append and cursor checkpoint in one transaction.
pub(crate) async fn advance_snapshot_summaries_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &mut SnapshotWork<M>,
    now_ms: i64,
) -> Result<()> {
    let work_limits = state.work_limits();
    use protocol::SyncError;
    ensure!(
        state.root_promoted && !state.descriptor_complete,
        "snapshot summaries unavailable"
    );
    let expected = serde_json::to_vec(state)?;
    let mut next = state.clone();
    let append = !next.descriptor_started
        || next
            .summary
            .as_ref()
            .is_some_and(|s| matches!(s.phase, SummaryPhase::Done))
        || (next.summary.is_none()
            && next
                .summary_refs
                .as_ref()
                .is_some_and(|r| r.current.is_none() && r.pending.is_none()));
    let required_mutations = if append { 5 } else { 3 };
    if work_limits.max_mutations < required_mutations {
        return Err(SyncError::LimitTooSmall {
            required: required_mutations,
        }
        .into());
    }
    let checkpoint_bound = u64::try_from(expected.len())?
        .checked_add(8192)
        .ok_or(SyncError::LogicalLimit)?;
    let required_bytes = checkpoint_bound
        .checked_add(8192)
        .ok_or(SyncError::LogicalLimit)?;
    if work_limits.max_bytes < required_bytes {
        return Err(SyncError::LimitTooSmall {
            required: required_bytes,
        }
        .into());
    }
    let namespace = next.source.namespace;
    let request = next.cursor.operation_id;
    let lease = crate::causal_leases::building_on(tx, namespace, request, now_ms).await?;
    ensure!(
        lease.build_state.as_deref() == Some(expected.as_slice())
            && lease.generation == next.source.generation.as_bytes(),
        "stale snapshot checkpoint"
    );
    let mut descriptor_bytes = None;
    if !next.descriptor_started {
        ensure!(lease.descriptor.is_none(), "descriptor already staged");
        descriptor_bytes = Some(descriptor_envelope(&next, &lease)?.0);
        let refs = next
            .reference_reversal
            .as_ref()
            .ok_or(SyncError::StaleWork)?;
        ensure!(refs.done, "partition references incomplete");
        next.summary_refs = Some(SummaryReferenceScan {
            current: refs.reverse_head,
            read: SegmentNodeRead::Header,
            pending: None,
        });
        next.descriptor_started = true;
    } else if let Some(summary) = &mut next.summary {
        if matches!(summary.phase, SummaryPhase::Done) {
            let result = summary.result.as_ref().ok_or(SyncError::StaleWork)?;
            let mut bytes = if next.summary_count == 0 {
                Vec::new()
            } else {
                vec![b',']
            };
            bytes.extend(serde_json::to_vec(result)?);
            descriptor_bytes = Some(bytes);
            next.summary = None;
            next.summary_count = next
                .summary_count
                .checked_add(1)
                .ok_or(SyncError::LogicalLimit)?;
        } else {
            let mut limits = work_limits;
            limits.max_mutations -= 3;
            limits.max_bytes -= checkpoint_bound;
            summary_pass_step_on(tx, namespace, &next.scope, lease.state_seq, summary, limits)
                .await?;
        }
    } else {
        let refs = next.summary_refs.as_mut().ok_or(SyncError::StaleWork)?;
        if let Some(entry) = refs.pending.take() {
            let mut collections: Vec<_> = next.scope.collections.iter().collect();
            collections.sort_by_key(|v| M::collection_tag(v));
            let collection = collections
                .get(usize::try_from(next.summary_count)?)
                .ok_or(SyncError::StaleWork)?;
            ensure!(
                entry.collection == M::collection_tag(collection)
                    && entry.manifest.kind == protocol::ManifestKind::ContextPartition,
                "invalid summary partition order"
            );
            let root = next
                .reversal
                .as_ref()
                .and_then(|r| {
                    r.collection_roots
                        .get(M::collection_tag(collection))
                        .copied()
                })
                .ok_or(SyncError::StaleWork)?;
            next.summary = Some(SummaryPass::new(
                request,
                (*collection).clone(),
                entry.manifest.item_count,
                root,
            )?);
        } else if let Some(digest) = refs.current {
            if let Some(node) = read_private_node_step_on(
                tx,
                namespace,
                digest,
                &mut refs.read,
                work_limits.max_rows_scanned,
                work_limits.max_bytes - checkpoint_bound,
            )
            .await?
            {
                let entry: PartitionReferenceNode = serde_json::from_slice(&node.payload)?;
                ensure!(
                    entry.kind == "partition_reference" && entry.version == 1,
                    "invalid summary reference"
                );
                refs.pending = Some(entry);
                refs.current = node.previous;
                refs.read = SegmentNodeRead::Header;
            }
        } else {
            ensure!(
                usize::try_from(next.summary_count)? == next.scope.collections.len(),
                "incomplete snapshot summary coverage"
            );
            descriptor_bytes = Some(descriptor_envelope(&next, &lease)?.1);
            next.descriptor_complete = true;
        }
    }
    next.cursor.position = next
        .cursor
        .position
        .checked_add(1)
        .ok_or(SyncError::LogicalLimit)?;
    let encoded = serde_json::to_vec(&next)?;
    ensure!(
        u64::try_from(encoded.len())? <= checkpoint_bound,
        "snapshot summary checkpoint {} exceeds reserved {} bytes",
        encoded.len(),
        checkpoint_bound
    );
    if let Some(bytes) = descriptor_bytes {
        let required = u64::try_from(encoded.len())?
            .checked_add(u64::try_from(bytes.len())?)
            .ok_or(SyncError::LogicalLimit)?;
        if work_limits.max_bytes < required {
            return Err(SyncError::LimitTooSmall { required }.into());
        }
        next.transient_lease_bytes = next
            .transient_lease_bytes
            .checked_add(u64::try_from(bytes.len())?)
            .ok_or(SyncError::LogicalLimit)?;
        crate::causal_leases::append_descriptor_on(tx, namespace, request, bytes, now_ms).await?;
    }
    crate::causal_leases::checkpoint_on(tx, namespace, request, Some(&expected), encoded, now_ms)
        .await?;
    *state = next;
    Ok(())
}

/// Atomically make a fully staged descriptor serviceable and replace the work
/// root with the verified protocol ContextRoot. The caller commits before use.
pub(crate) async fn publish_snapshot_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &mut SnapshotWork<M>,
    now_ms: i64,
) -> Result<protocol::SnapshotDescriptor<M>> {
    let work_limits = state.work_limits();
    use protocol::SyncError;
    const MUTATIONS: u64 = 7;
    ensure!(
        state.root_promoted
            && state.descriptor_complete
            && usize::try_from(state.summary_count)? == state.scope.collections.len(),
        "snapshot descriptor incomplete"
    );
    if work_limits.max_mutations < MUTATIONS {
        return Err(SyncError::LimitTooSmall {
            required: MUTATIONS,
        }
        .into());
    }
    let expected = serde_json::to_vec(state)?;
    let mut next = state.clone();
    let namespace = next.source.namespace;
    let request = next.cursor.operation_id;
    let lease = crate::causal_leases::building_on(tx, namespace, request, now_ms).await?;
    ensure!(
        lease.build_state.as_deref() == Some(expected.as_slice())
            && lease.generation == next.source.generation.as_bytes(),
        "stale snapshot checkpoint"
    );
    let descriptor_bytes = lease.descriptor.as_deref().ok_or(SyncError::StaleWork)?;
    let required = u64::try_from(descriptor_bytes.len())?;
    if work_limits.max_bytes < required {
        return Err(SyncError::LimitTooSmall { required }.into());
    }
    let descriptor: protocol::SnapshotDescriptor<M> = serde_json::from_slice(descriptor_bytes)?;
    ensure!(
        descriptor.source == next.source
            && descriptor.scope == next.scope
            && descriptor.snapshot_id == request
            && descriptor.roots.len() == next.scope.collections.len()
            && descriptor.head.sequence == u64::try_from(lease.head)?
            && descriptor.head.anchor.as_slice() == lease.head_anchor
            && descriptor.expires_unix_ms == u64::try_from(lease.expires_at_ms)?,
        "staged descriptor mismatch"
    );
    let emission = next.root_emission.as_ref().ok_or(SyncError::StaleWork)?;
    ensure!(
        matches!(emission.phase, PartitionEmissionPhase::Verified)
            && lease.retained_root.as_deref() == emission.work_root.as_ref().map(|v| v.as_slice()),
        "snapshot root changed"
    );
    let root = emission.manifest.manifest_digest;
    let staging = next.staging.as_ref().ok_or(SyncError::StaleWork)?;
    ensure!(
        staging.done
            && staging.total == next.segment_count
            && lease.output_generation.as_deref() == Some(staging.generation.as_bytes().as_slice()),
        "staged source index incomplete"
    );
    next.cursor.position = next
        .cursor
        .position
        .checked_add(1)
        .ok_or(SyncError::LogicalLimit)?;
    let encoded = serde_json::to_vec(&next)?;
    let total_bytes = required
        .checked_add(u64::try_from(encoded.len())?)
        .ok_or(SyncError::LogicalLimit)?;
    if work_limits.max_bytes < total_bytes {
        return Err(SyncError::LimitTooSmall {
            required: total_bytes,
        }
        .into());
    }
    crate::causal_leases::pin_root_on(tx, namespace, request, root, now_ms).await?;
    crate::causal_leases::checkpoint_on(tx, namespace, request, Some(&expected), encoded, now_ms)
        .await?;
    crate::causal_leases::publish_staged_on(tx, namespace, request, now_ms).await?;
    *state = next;
    Ok(descriptor)
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

/// Plan only from the already admitted or just-checkpointed in-memory state.
fn plan_snapshot_step<M: SyncModel>(
    state: &SnapshotWork<M>,
    full_limits: WorkLimits,
) -> Result<SnapshotStepPlan> {
    let fixed = if usize::try_from(state.collection_index)? < state.scope.collections.len() {
        state.collection_end
            || state.pending_context.is_some()
            || (state.awaiting_union && state.union.is_none())
            || (!state.awaiting_union
                && state
                    .boundary
                    .as_ref()
                    .is_none_or(|boundary| boundary.after.is_none()))
    } else if !state.reversal.as_ref().is_some_and(|r| r.done)
        || usize::try_from(state.partition_index)? < state.scope.collections.len()
        || !state.reference_reversal.as_ref().is_some_and(|r| r.done)
        || !state
            .root_pass
            .as_ref()
            .is_some_and(|p| matches!(p.phase, PartitionPassPhase::Done))
        || !state
            .root_emission
            .as_ref()
            .is_some_and(|e| matches!(e.phase, PartitionEmissionPhase::Verified))
        || !state.root_promoted
        || !state.staging.as_ref().is_some_and(|s| s.done)
    {
        true
    } else if !state.descriptor_complete {
        state
            .summary
            .as_ref()
            .is_none_or(|summary| match summary.phase {
                SummaryPhase::Segments | SummaryPhase::Done => true,
                SummaryPhase::Count => {
                    summary.count.count == 0
                        && summary.count.after.is_none()
                        && summary.count.pending.is_none()
                }
                SummaryPhase::Candidates => summary.count.count == 0,
            })
    } else {
        false
    };
    let admitted = if fixed {
        let encoded = serde_json::to_vec(state)?;
        let bytes = u64::try_from(encoded.len())?
            .checked_mul(2)
            .and_then(|n| n.checked_add(state.transient_lease_bytes))
            .and_then(|n| n.checked_add(16_384))
            .ok_or(protocol::SyncError::LogicalLimit)?;
        if full_limits.max_rows_scanned >= 5
            && full_limits.max_mutations >= 16
            && full_limits.max_bytes >= bytes
        {
            WorkLimits {
                max_rows_scanned: 5,
                max_mutations: 16,
                max_bytes: bytes,
                ..full_limits
            }
        } else {
            full_limits
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
            return Ok(protocol::WorkOutcome::Ready(descriptor));
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

async fn advance_snapshot_step_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &mut SnapshotWork<M>,
    now_ms: i64,
) -> Result<Option<protocol::SnapshotDescriptor<M>>> {
    if usize::try_from(state.collection_index)? < state.scope.collections.len() {
        if state.collection_end || state.pending_context.is_some() {
            advance_snapshot_segment_on(tx, state, now_ms).await?;
        } else if state.awaiting_union {
            advance_snapshot_context_on(tx, state, now_ms).await?;
        } else {
            advance_snapshot_boundary_on(tx, state, now_ms).await?;
        }
    } else if !state.reversal.as_ref().is_some_and(|r| r.done) {
        advance_snapshot_reversal_on(tx, state, now_ms).await?;
    } else if usize::try_from(state.partition_index)? < state.scope.collections.len() {
        if !state
            .partition
            .as_ref()
            .is_some_and(|p| matches!(p.phase, PartitionPassPhase::Done))
        {
            advance_snapshot_partition_on(tx, state, now_ms).await?;
        } else if !state
            .partition_emission
            .as_ref()
            .is_some_and(|e| matches!(e.phase, PartitionEmissionPhase::Verified))
        {
            advance_snapshot_partition_emission_on(tx, state, now_ms).await?;
        } else {
            advance_snapshot_partition_promotion_on(tx, state, now_ms).await?;
        }
    } else if !state.reference_reversal.as_ref().is_some_and(|r| r.done) {
        advance_snapshot_reference_reversal_on(tx, state, now_ms).await?;
    } else if !state
        .root_pass
        .as_ref()
        .is_some_and(|p| matches!(p.phase, PartitionPassPhase::Done))
    {
        advance_snapshot_root_pass_on(tx, state, now_ms).await?;
    } else if !state
        .root_emission
        .as_ref()
        .is_some_and(|e| matches!(e.phase, PartitionEmissionPhase::Verified))
    {
        advance_snapshot_root_emission_on(tx, state, now_ms).await?;
    } else if !state.root_promoted {
        advance_snapshot_root_promotion_on(tx, state, now_ms).await?;
    } else if !state.staging.as_ref().is_some_and(|s| s.done) {
        advance_snapshot_staging_on(tx, state, now_ms).await?;
    } else if !state.descriptor_complete {
        advance_snapshot_summaries_on(tx, state, now_ms).await?;
    } else {
        let descriptor = publish_snapshot_on(tx, state, now_ms).await?;
        return Ok(Some(descriptor));
    }
    Ok(None)
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
                    outcome: protocol::WorkOutcome::Ready(descriptor),
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
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
struct SourceStaging<M: SyncModel> {
    generation: Uuid,
    collection_index: u32,
    ordinal: u64,
    total: u64,
    next_lower: Vec<u8>,
    previous_context: Option<protocol::Digest>,
    scan: SnapshotSegmentScan<M>,
    done: bool,
}

/// Stage one authenticated normalized segment or advance its ordered iterator.
/// The private generation is never selected as the active baseline here.
pub(crate) async fn advance_snapshot_staging_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    state: &mut SnapshotWork<M>,
    now_ms: i64,
) -> Result<()> {
    let work_limits = state.work_limits();
    use protocol::SyncError;
    ensure!(state.root_promoted, "source root incomplete");
    let expected = serde_json::to_vec(state)?;
    let mut next = state.clone();
    let mut collections: Vec<_> = next.scope.collections.iter().cloned().collect();
    collections.sort_by_key(M::collection_tag);
    let required_mutations = if next.staging.is_none() {
        5
    } else if next
        .staging
        .as_ref()
        .is_some_and(|s| s.scan.done && s.scan.best.is_some())
    {
        4
    } else {
        3
    };
    if work_limits.max_mutations < required_mutations {
        return Err(SyncError::LimitTooSmall {
            required: required_mutations,
        }
        .into());
    }
    let checkpoint_bound = u64::try_from(expected.len())?
        .checked_add(8192)
        .ok_or(SyncError::LogicalLimit)?;
    let required_bytes = checkpoint_bound
        .checked_add(8192)
        .ok_or(SyncError::LogicalLimit)?;
    if work_limits.max_bytes < required_bytes {
        return Err(SyncError::LimitTooSmall {
            required: required_bytes,
        }
        .into());
    }
    let namespace = next.source.namespace;
    let request = next.cursor.operation_id;
    let lease = crate::causal_leases::building_on(tx, namespace, request, now_ms).await?;
    ensure!(
        lease.build_state.as_deref() == Some(expected.as_slice())
            && lease.generation == next.source.generation.as_bytes(),
        "stale snapshot checkpoint"
    );
    let emission = next.root_emission.as_ref().ok_or(SyncError::StaleWork)?;
    ensure!(
        matches!(emission.phase, PartitionEmissionPhase::Verified)
            && lease.retained_root.as_deref() == emission.work_root.as_ref().map(|v| v.as_slice()),
        "source root changed"
    );
    if let Some(staging) = &mut next.staging {
        ensure!(
            !staging.done
                && lease.output_generation.as_deref()
                    == Some(staging.generation.as_bytes().as_slice()),
            "invalid source staging generation"
        );
        if !staging.scan.done {
            let mut limits = work_limits;
            limits.max_bytes -= checkpoint_bound;
            limits.max_mutations -= 3;
            snapshot_segment_scan_step_on(tx, namespace, &mut staging.scan, limits).await?;
        } else {
            let segment = staging
                .scan
                .best
                .take()
                .ok_or_else(|| anyhow::anyhow!("missing staged source segment"))?;
            let collection = collections
                .get(usize::try_from(staging.collection_index)?)
                .ok_or(SyncError::StaleWork)?;
            ensure!(
                segment.range.collection == *collection
                    && snapshot_bound(segment.range.lower, true) == staging.next_lower
                    && staging.previous_context != Some(segment.context.digest),
                "invalid staged source partition"
            );
            let reference = crate::causal_manifests::reference_on(
                tx,
                namespace,
                segment.context.manifest.manifest_digest,
            )
            .await?;
            ensure!(
                reference == segment.context.manifest
                    && reference.kind == protocol::ManifestKind::Context,
                "invalid staged context reference"
            );
            crate::causal_manifests::ensure_context_digest_on(
                tx,
                namespace,
                reference.manifest_digest,
                segment.context.digest,
            )
            .await?;
            let lower = snapshot_bound(segment.range.lower, true);
            let upper = snapshot_bound(segment.range.upper, false);
            ensure!(lower < upper, "invalid staged source bounds");
            let tag = M::collection_tag(collection).as_bytes().to_vec();
            let ordinal = i64::try_from(staging.ordinal)?;
            let q = Query::select()
                .columns(
                    [
                        "lower_bound",
                        "upper_bound",
                        "context_manifest",
                        "context_digest",
                    ]
                    .map(Alias::new),
                )
                .from(Alias::new("sync_context_ranges"))
                .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
                .and_where(Expr::col(Alias::new("collection")).eq(tag.clone()))
                .and_where(
                    Expr::col(Alias::new("basis_generation"))
                        .eq(staging.generation.as_bytes().to_vec()),
                )
                .and_where(Expr::col(Alias::new("state_seq")).eq(lease.state_seq))
                .and_where(Expr::col(Alias::new("ordinal")).eq(ordinal))
                .to_owned();
            if let Some(row) = tx.query_one(&q).await? {
                ensure!(
                    row.try_get_by_index::<Vec<u8>>(0)? == lower
                        && row.try_get_by_index::<Vec<u8>>(1)? == upper
                        && row.try_get_by_index::<Vec<u8>>(2)? == reference.manifest_digest
                        && row.try_get_by_index::<Option<Vec<u8>>>(3)?.as_deref()
                            == Some(segment.context.digest.as_slice()),
                    "conflicting staged source segment"
                );
            } else {
                let q = Query::insert()
                    .into_table(Alias::new("sync_context_ranges"))
                    .columns(
                        [
                            "namespace",
                            "collection",
                            "basis_generation",
                            "state_seq",
                            "ordinal",
                            "lower_bound",
                            "upper_bound",
                            "context_manifest",
                            "context_digest",
                        ]
                        .map(Alias::new),
                    )
                    .values_panic([
                        namespace.as_bytes().to_vec().into(),
                        tag.into(),
                        staging.generation.as_bytes().to_vec().into(),
                        lease.state_seq.into(),
                        ordinal.into(),
                        lower.into(),
                        upper.clone().into(),
                        reference.manifest_digest.to_vec().into(),
                        segment.context.digest.to_vec().into(),
                    ])
                    .to_owned();
                tx.execute(&q).await?;
            }
            staging.total = staging
                .total
                .checked_add(1)
                .ok_or(SyncError::LogicalLimit)?;
            if segment.range.upper.is_none() {
                staging.collection_index = staging
                    .collection_index
                    .checked_add(1)
                    .ok_or(SyncError::LogicalLimit)?;
                if usize::try_from(staging.collection_index)? == collections.len() {
                    ensure!(
                        staging.total == next.segment_count && staging.scan.current.is_none(),
                        "incomplete staged source coverage"
                    );
                    staging.done = true;
                } else {
                    let following = collections
                        .get(usize::try_from(staging.collection_index)?)
                        .ok_or(SyncError::StaleWork)?;
                    staging.ordinal = 0;
                    staging.next_lower = vec![0];
                    staging.previous_context = None;
                    staging.scan = SnapshotSegmentScan {
                        current: staging.scan.current,
                        read: SegmentNodeRead::Header,
                        collection: M::collection_tag(following).to_owned(),
                        lower: vec![0],
                        best: None,
                        done: false,
                        ordered: true,
                    };
                }
            } else {
                staging.ordinal = staging
                    .ordinal
                    .checked_add(1)
                    .ok_or(SyncError::LogicalLimit)?;
                staging.next_lower = upper;
                staging.previous_context = Some(segment.context.digest);
                staging.scan = SnapshotSegmentScan {
                    current: staging.scan.current,
                    read: SegmentNodeRead::Header,
                    collection: M::collection_tag(collection).to_owned(),
                    lower: staging.next_lower.clone(),
                    best: None,
                    done: false,
                    ordered: true,
                };
            }
        }
    } else {
        ensure!(
            lease.output_generation.is_none(),
            "source generation already claimed"
        );
        let generation = Uuid::new_v4();
        crate::causal_leases::claim_output_generation_on(
            tx, namespace, request, generation, now_ms,
        )
        .await?;
        let ordered = next
            .reversal
            .as_ref()
            .filter(|r| r.done)
            .and_then(|r| r.reverse_head)
            .ok_or(SyncError::StaleWork)?;
        let collection = collections.first().ok_or(SyncError::StaleWork)?;
        next.staging = Some(SourceStaging {
            generation,
            collection_index: 0,
            ordinal: 0,
            total: 0,
            next_lower: vec![0],
            previous_context: None,
            scan: SnapshotSegmentScan::ordered(ordered, M::collection_tag(collection).to_owned()),
            done: false,
        });
    }
    next.cursor.position = next
        .cursor
        .position
        .checked_add(1)
        .ok_or(SyncError::LogicalLimit)?;
    let encoded = serde_json::to_vec(&next)?;
    ensure!(
        u64::try_from(encoded.len())? <= checkpoint_bound,
        "source staging checkpoint exceeded bound"
    );
    crate::causal_leases::checkpoint_on(tx, namespace, request, Some(&expected), encoded, now_ms)
        .await?;
    *state = next;
    Ok(())
}

/// Sealed proof that every segment of the verified source root has been staged
/// into one claimed inactive generation under the same lease checkpoints.
pub(crate) struct StagedSourceProof {
    source: CompletedSourceProof,
    generation: Uuid,
    segments: u64,
}
impl StagedSourceProof {
    pub(crate) fn source(&self) -> &CompletedSourceProof {
        &self.source
    }
    pub(crate) fn generation(&self) -> Uuid {
        self.generation
    }
    pub(crate) fn segments(&self) -> u64 {
        self.segments
    }
}

pub(crate) async fn staged_source_proof_for_lease_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    lease_id: Uuid,
    now_ms: i64,
) -> Result<StagedSourceProof> {
    use protocol::SyncError;
    let lease = crate::causal_leases::load_on(tx, namespace, lease_id)
        .await?
        .ok_or(SyncError::SnapshotExpired)?;
    let state: SnapshotWork<M> =
        serde_json::from_slice(lease.build_state.as_deref().ok_or(SyncError::StaleWork)?)?;
    ensure!(
        state.source.namespace == namespace && state.cursor.operation_id == lease_id,
        "source staging identity mismatch"
    );
    let source = completed_source_proof_on(tx, &state, now_ms).await?;
    let staging = state.staging.as_ref().ok_or(SyncError::StaleWork)?;
    ensure!(
        staging.done
            && staging.total == state.segment_count
            && lease.output_generation.as_deref() == Some(staging.generation.as_bytes().as_slice()),
        "source staging incomplete"
    );
    Ok(StagedSourceProof {
        source,
        generation: staging.generation,
        segments: staging.total,
    })
}

/// Authenticate a ready normalized generation against the completed source
/// cursor and descriptor before any indexed source-view read.
pub(crate) fn ready_generation<M: SyncModel>(
    lease: &crate::causal_leases::Lease,
    descriptor: &protocol::SnapshotDescriptor<M>,
) -> Result<Uuid> {
    use protocol::SyncError;
    let state: SnapshotWork<M> =
        serde_json::from_slice(lease.build_state.as_deref().ok_or(SyncError::StaleWork)?)?;
    let staging = state.staging.as_ref().ok_or(SyncError::StaleWork)?;
    let root = state.root_emission.as_ref().ok_or(SyncError::StaleWork)?;
    ensure!(
        state.root_promoted
            && state.descriptor_complete
            && staging.done
            && staging.total == state.segment_count
            && lease.output_generation.as_deref() == Some(staging.generation.as_bytes().as_slice())
            && lease.retained_root.as_deref() == Some(root.manifest.manifest_digest.as_slice())
            && matches!(root.phase, PartitionEmissionPhase::Verified)
            && state.source == descriptor.source
            && state.scope == descriptor.scope
            && state.cursor.operation_id == descriptor.snapshot_id
            && descriptor.head.sequence == u64::try_from(lease.head)?
            && descriptor.head.anchor.as_slice() == lease.head_anchor
            && descriptor.expires_unix_ms == u64::try_from(lease.expires_at_ms)?
            && descriptor.roots.len() == state.scope.collections.len(),
        "ready source generation mismatch"
    );
    Ok(staging.generation)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum RangeWorkPhase {
    CountSegments,
    CountCandidates,
    HashSegments,
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
    context_cursor: protocol::ContextCursor,
    segment_count: u64,
    candidates: CandidateCount,
    hasher: Option<Vec<u8>>,
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
    let generation = ready_generation(&parent, &descriptor)?;
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
    if parent.status != 1
        || parent.expires_at_ms <= now_ms
        || parent.output_generation.as_deref() != Some(generation.as_bytes().as_slice())
    {
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
            RangeWorkPhase::CountSegments | RangeWorkPhase::HashSegments => {
                let page = crate::causal_snapshot::context_page_on(
                    tx,
                    namespace,
                    snapshot_id,
                    now_ms,
                    range.clone(),
                    work.context_cursor.clone(),
                    protocol::PageLimits {
                        max_scanned: 1,
                        max_bytes: std::cmp::min(data_limits.max_bytes, protocol::MAX_PAGE_BYTES),
                    },
                )
                .await?;
                ensure!(page.segments.len() == 1, "empty summary context page");
                let segment = &page.segments[0];
                match work.phase {
                    RangeWorkPhase::CountSegments => {
                        work.segment_count = work
                            .segment_count
                            .checked_add(1)
                            .ok_or(SyncError::LogicalLimit)?
                    }
                    RangeWorkPhase::HashSegments => {
                        let mut hasher = protocol::RangeHasher::<M>::from_checkpoint(
                            &descriptor.scope,
                            &range,
                            work.hasher.as_deref().ok_or(SyncError::StaleWork)?,
                        )?;
                        hasher.push_segment(&segment.range, segment.context.digest)?;
                        work.hasher = Some(hasher.checkpoint()?);
                    }
                    _ => unreachable!(),
                }
                work.context_cursor = page.next;
                if page.done {
                    match work.phase {
                        RangeWorkPhase::CountSegments => {
                            work.phase = RangeWorkPhase::CountCandidates
                        }
                        RangeWorkPhase::HashSegments => {
                            let hasher = protocol::RangeHasher::<M>::from_checkpoint(
                                &descriptor.scope,
                                &range,
                                work.hasher.as_deref().ok_or(SyncError::StaleWork)?,
                            )?;
                            work.candidate_hash = Some(CandidateHash::new(&hasher)?);
                            work.hasher = None;
                            work.phase = RangeWorkPhase::HashCandidates;
                        }
                        _ => unreachable!(),
                    }
                }
            }
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
                        work.segment_count,
                    )?;
                    work.hasher = Some(hash.checkpoint()?);
                    work.context_cursor = protocol::ContextCursor {
                        segment_index: 0,
                        next_lower: range.lower,
                    };
                    work.phase = RangeWorkPhase::HashSegments;
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
                    let (aggregate, context_commitment) =
                        hash.clone().finish(&descriptor.scope, &range)?;
                    ready = Some(protocol::RangeSummary {
                        snapshot_id,
                        range: range.clone(),
                        candidate_count: work.candidates.count,
                        candidate_bytes: work.candidates.bytes,
                        context_segments: work.segment_count,
                        context_commitment,
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
        phase: RangeWorkPhase::CountSegments,
        context_cursor: protocol::ContextCursor {
            segment_index: 0,
            next_lower: range.lower,
        },
        segment_count: 0,
        candidates: CandidateCount::default(),
        hasher: None,
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

/// Build a full-collection normalized generation while retaining the input
/// basis. The result is private compaction evidence, not a served snapshot.
pub(crate) async fn prepare_baseline_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    source: protocol::StoreIdentity,
    request: Uuid,
    now_ms: i64,
    limits: WorkLimits,
    resume: Option<protocol::WorkCursor>,
) -> Result<protocol::WorkOutcome<StagedSourceProof>> {
    let scope = protocol::Scope::<M> {
        collections: M::collections().into_iter().collect(),
    };
    if let Some(lease) = crate::causal_leases::load_on(tx, source.namespace, request).await?
        && let Some(saved) = lease.build_state.as_deref()
    {
        let state: SnapshotWork<M> = serde_json::from_slice(saved)?;
        if state.staging.as_ref().is_some_and(|staging| staging.done) {
            let admitted = admit_snapshot_on(
                tx,
                source.clone(),
                request,
                scope,
                crate::causal_source::SnapshotTiming {
                    now_ms,
                    ttl_ms: protocol::DEFAULT_SNAPSHOT_TTL_MS as i64,
                },
                limits,
                resume,
            )
            .await?;
            ensure!(
                matches!(admitted, SnapshotAdmission::Building(_)),
                protocol::SyncError::StaleWork
            );
            return Ok(protocol::WorkOutcome::Ready(
                staged_source_proof_for_lease_on::<M>(tx, source.namespace, request, now_ms)
                    .await?,
            ));
        }
    }
    match open_on(
        tx,
        source,
        request,
        scope,
        crate::causal_source::SnapshotTiming {
            now_ms,
            ttl_ms: protocol::DEFAULT_SNAPSHOT_TTL_MS as i64,
        },
        limits,
        resume,
    )
    .await?
    {
        protocol::WorkOutcome::Incomplete { cursor } => {
            Ok(protocol::WorkOutcome::Incomplete { cursor })
        }
        protocol::WorkOutcome::Ready(_) => Err(protocol::SyncError::StaleWork.into()),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum SplitPhase {
    CountCandidates,
    ContextFallback,
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

/// Choose a stable interior boundary using a captured view. Candidate and
/// context passes are checkpointed without copying candidate payloads.
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
    let generation = ready_generation(&parent, &descriptor)?;
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
    if parent.status != 1
        || parent.expires_at_ms <= now_ms
        || parent.output_generation.as_deref() != Some(generation.as_bytes().as_slice())
    {
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
                    work.phase = SplitPhase::ContextFallback;
                }
            }
            SplitPhase::ContextFallback => {
                let page = crate::causal_snapshot::context_page_on(
                    tx,
                    namespace,
                    snapshot_id,
                    now_ms,
                    range.clone(),
                    protocol::ContextCursor {
                        segment_index: 0,
                        next_lower: range.lower,
                    },
                    protocol::PageLimits {
                        max_scanned: 1,
                        max_bytes: std::cmp::min(data_limits.max_bytes, protocol::MAX_PAGE_BYTES),
                    },
                )
                .await?;
                ensure!(page.segments.len() == 1, "empty split context page");
                work.fallback = page.segments[0]
                    .range
                    .upper
                    .filter(|end| Some(*end) != range.upper)
                    .or(work.fallback);
                if work.summary_candidates.unwrap_or(work.distinct) == 0 {
                    result = Some(split_ranges(&range, work.fallback));
                } else {
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
            SplitPhase::ContextFallback
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
