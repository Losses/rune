//! Validation of complete retained source-range evidence before mutation.
use crate::causal_manifests::read_on;
use anyhow::{Result, ensure};
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, ExprTrait,
    sea_query::{Alias, Expr, Query},
};
use std::collections::{BTreeMap, BTreeSet};
use sync::protocol::{self, Candidate, ContextSegment, Event, ManifestKind, RangeEvent, SyncModel};
use uuid::Uuid;

async fn read_dependency_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    digest: protocol::Digest,
    max_bytes: u64,
    max_fragments: u32,
) -> Result<(protocol::ManifestRef, Vec<u8>)> {
    let query = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("digest")).eq(digest.to_vec()))
        .limit(1)
        .to_owned();
    if tx.query_one(&query).await?.is_none() {
        return Err(protocol::SyncError::MissingDependency(digest).into());
    }
    let reference = crate::causal_manifests::reference_on(tx, namespace, digest).await?;
    ensure!(
        reference.encoded_bytes <= max_bytes && reference.fragment_count <= max_fragments,
        protocol::SyncError::WorkLimit
    );
    read_on(tx, namespace, digest, max_bytes, max_fragments).await
}

/// Evidence whose bodies, scope, bounds, candidate validators and summary were
/// checked together. It is only valid for the database transaction that read it.
pub struct ValidatedRange<'a, M: SyncModel> {
    transaction: &'a DatabaseTransaction,
    event: RangeEvent<M>,
    segments: Vec<ContextSegment<M>>,
    contexts: BTreeMap<protocol::Digest, sync::causal::Context>,
    candidates: Vec<Candidate<M>>,
}
impl<M: SyncModel> ValidatedRange<'_, M> {
    pub fn event(&self) -> &RangeEvent<M> {
        &self.event
    }
    pub fn segments(&self) -> &[ContextSegment<M>] {
        &self.segments
    }
    pub fn candidates(&self) -> &[Candidate<M>] {
        &self.candidates
    }
    pub fn context(&self, digest: &protocol::Digest) -> Option<&sync::causal::Context> {
        self.contexts.get(digest)
    }
}
/// Validate all transitive source evidence before any receiver mutation.
/// The namespace lock prevents concurrent GC while this transaction remains
/// open. The caller must commit or roll back joined projection in this same
/// transaction as its eventual range application.
pub async fn validate_on<'a, M: SyncModel>(
    tx: &'a DatabaseTransaction,
    namespace: Uuid,
    event: &RangeEvent<M>,
    max_bytes: u64,
    max_items: u64,
    max_fragments: u32,
) -> Result<ValidatedRange<'a, M>> {
    ensure!(
        !namespace.is_nil()
            && namespace == event.source.source.namespace
            && max_bytes > 0
            && max_bytes <= protocol::MAX_LOGICAL_BYTES
            && max_items > 0
            && max_items <= protocol::MAX_LOGICAL_ITEMS
            && max_fragments > 0
            && max_fragments <= protocol::MAX_FRAGMENTS,
        "invalid range admission"
    );
    crate::causal_leases::lock_on(tx, namespace).await?;
    protocol::canonical_event(&Event::Range(Box::new(event.clone())))?;
    let mut used_bytes = 0_u64;
    let mut used_fragments = 0_u64;
    let (part, body) = read_dependency_on(
        tx,
        namespace,
        event.context_partition.manifest_digest,
        max_bytes,
        max_fragments,
    )
    .await?;
    ensure!(
        part == event.context_partition && part.kind == ManifestKind::ContextPartition,
        "invalid partition manifest"
    );
    used_bytes += body.len() as u64;
    used_fragments += u64::from(part.fragment_count);
    let segments = protocol::decode_context_partition::<M>(&event.range, &body)?;
    ensure!(
        segments.len() as u64 == part.item_count,
        "invalid partition count"
    );
    ensure!(
        segments.len() as u64 <= max_items,
        protocol::SyncError::WorkLimit
    );
    let mut used_items = segments.len() as u64;
    // The retained graph must agree with the decoded body: otherwise a
    // concurrent or later collection could reclaim evidence still referenced by
    // this partition. Limit the query before reading the edge set.
    let expected: BTreeSet<_> = segments
        .iter()
        .map(|segment| segment.context.manifest.manifest_digest)
        .collect();
    let query = Query::select()
        .column(Alias::new("child_manifest"))
        .from(Alias::new("sync_manifest_edges"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("parent_manifest")).eq(part.manifest_digest.to_vec()))
        .limit(expected.len() as u64 + 1)
        .to_owned();
    let mut actual = BTreeSet::new();
    for row in tx.query_all(&query).await? {
        let bytes: Vec<u8> = row.try_get_by_index(0)?;
        let digest: protocol::Digest = bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid dependency digest"))?;
        actual.insert(digest);
    }
    ensure!(actual == expected, "partition dependency graph mismatch");
    let mut contexts = BTreeMap::new();
    let mut context_refs = BTreeMap::new();
    for segment in &segments {
        let key = segment.context.digest;
        if let Some(previous) = context_refs.get(&key) {
            ensure!(
                previous == &segment.context.manifest,
                "conflicting context reference"
            );
            continue;
        }
        context_refs.insert(key, segment.context.manifest.clone());
        let remaining = max_bytes
            .checked_sub(used_bytes)
            .ok_or_else(|| anyhow::Error::new(protocol::SyncError::WorkLimit))?;
        let fragments = u32::try_from(
            u64::from(max_fragments)
                .checked_sub(used_fragments)
                .ok_or_else(|| anyhow::Error::new(protocol::SyncError::WorkLimit))?,
        )?;
        ensure!(
            remaining > 0 && fragments > 0,
            protocol::SyncError::WorkLimit
        );
        let (stored, body) = read_dependency_on(
            tx,
            namespace,
            segment.context.manifest.manifest_digest,
            remaining,
            fragments,
        )
        .await?;
        ensure!(
            stored == segment.context.manifest && stored.kind == ManifestKind::Context,
            "invalid context manifest"
        );
        used_bytes = used_bytes
            .checked_add(body.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("range byte overflow"))?;
        used_fragments += u64::from(stored.fragment_count);
        let exact = protocol::decode_context(&body)?;
        let context_items = exact
            .writers
            .iter()
            .try_fold(0_u64, |count, writer| {
                count.checked_add(1 + writer.exceptions.len() as u64)
            })
            .ok_or_else(|| anyhow::anyhow!("range item overflow"))?;
        used_items = used_items
            .checked_add(context_items)
            .ok_or_else(|| anyhow::anyhow!("range item overflow"))?;
        ensure!(used_items <= max_items, protocol::SyncError::WorkLimit);
        ensure!(
            protocol::context_digest(&exact)? == key
                && stored.item_count == exact.writers.len() as u64,
            "context commitment mismatch"
        );
        contexts.insert(key, protocol::causal_context(&exact)?);
    }
    let remaining = max_bytes
        .checked_sub(used_bytes)
        .ok_or_else(|| anyhow::Error::new(protocol::SyncError::WorkLimit))?;
    let fragments = u32::try_from(
        u64::from(max_fragments)
            .checked_sub(used_fragments)
            .ok_or_else(|| anyhow::Error::new(protocol::SyncError::WorkLimit))?,
    )?;
    ensure!(
        remaining > 0 && fragments > 0,
        protocol::SyncError::WorkLimit
    );
    let (live, body) = read_dependency_on(
        tx,
        namespace,
        event.live_manifest.manifest_digest,
        remaining,
        fragments,
    )
    .await?;
    ensure!(
        live == event.live_manifest && live.kind == ManifestKind::Leaf,
        "invalid live manifest"
    );
    let candidates = protocol::decode_candidates::<M>(&body)?;
    ensure!(
        live.item_count == candidates.len() as u64,
        "invalid live candidate count"
    );
    ensure!(
        used_items
            .checked_add(candidates.len() as u64)
            .is_some_and(|n| n <= max_items),
        protocol::SyncError::WorkLimit
    );
    let mut candidate_bytes = 0_u64;
    let mut source_dots = BTreeSet::new();
    for candidate in &candidates {
        ensure!(
            candidate.namespace == namespace && candidate.key.collection == event.range.collection,
            "candidate outside range namespace or collection"
        );
        let index = segments
            .partition_point(|segment| segment.range.lower.is_none_or(|lo| lo <= candidate.key.id));
        let segment = index
            .checked_sub(1)
            .and_then(|index| segments.get(index))
            .ok_or_else(|| anyhow::anyhow!("candidate outside covered range"))?;
        ensure!(
            segment.range.upper.is_none_or(|hi| candidate.key.id < hi),
            "candidate outside covered range"
        );
        let context = contexts
            .get(&segment.context.digest)
            .ok_or_else(|| anyhow::anyhow!("missing source context"))?;
        let dot = sync::causal::Dot::new(candidate.dot.writer_incarnation, candidate.dot.counter)?;
        ensure!(
            context.contains(dot),
            "source context omits live candidate dot"
        );
        ensure!(
            source_dots.insert(candidate.dot),
            "source dot appears more than once in collection"
        );
        // A retained dot binds its complete candidate to one key within this
        // collection. Check even suppressed candidates before any mutation.
        let query = Query::select()
            .columns(["object", "body"].map(Alias::new))
            .from(Alias::new("sync_candidates"))
            .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
            .and_where(
                Expr::col(Alias::new("collection")).eq(M::collection_tag(
                    &candidate.key.collection,
                )
                .as_bytes()
                .to_vec()),
            )
            .and_where(
                Expr::col(Alias::new("writer")).eq(candidate
                    .dot
                    .writer_incarnation
                    .as_bytes()
                    .to_vec()),
            )
            .and_where(Expr::col(Alias::new("counter")).eq(i64::try_from(candidate.dot.counter)?))
            .limit(1)
            .to_owned();
        if let Some(row) = tx.query_one(&query).await? {
            ensure!(
                row.try_get_by_index::<Vec<u8>>(0)? == candidate.key.id.as_bytes()
                    && row.try_get_by_index::<Vec<u8>>(1)?
                        == protocol::canonical_candidate(candidate)?,
                "conflicting retained source dot"
            );
        }
        candidate_bytes = candidate_bytes
            .checked_add(protocol::canonical_candidate(candidate)?.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("candidate byte overflow"))?;
    }
    let mut hasher = protocol::RangeHasher::<M>::new(
        &event.scope,
        event.range.clone(),
        candidates.len() as u64,
        candidate_bytes,
        segments.len() as u64,
    )?;
    for segment in &segments {
        hasher.push_segment(&segment.range, segment.context.digest)?;
    }
    for candidate in &candidates {
        hasher.push_candidate(candidate)?;
    }
    ensure!(
        hasher.finish()? == event.range_summary,
        "source range commitment mismatch"
    );
    Ok(ValidatedRange {
        transaction: tx,
        event: event.clone(),
        segments,
        contexts,
        candidates,
    })
}

/// A bounded, transaction-local candidate join. This is a preflight for an
/// eventual atomic range commit; it does not itself publish causal state.
pub struct CandidateJoin<M: SyncModel> {
    pub changes: Vec<protocol::ProjectionChange<M>>,
    pub close: Vec<protocol::Dot>,
    pub open: Vec<Candidate<M>>,
}

pub async fn candidate_join_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    evidence: &ValidatedRange<'_, M>,
    floor_state: i64,
    state: i64,
    max_candidates: usize,
    max_bytes: usize,
) -> Result<CandidateJoin<M>> {
    use crate::causal_context::{ContextScan, ScanResult, scan_on};
    ensure!(
        max_candidates > 0
            && max_candidates <= 1024
            && max_bytes > 0
            && max_bytes <= 4 * 1024 * 1024,
        "invalid candidate join budget"
    );
    ensure!(
        evidence.candidates().len() <= max_candidates,
        protocol::SyncError::WorkLimit
    );
    let source_bytes = evidence
        .candidates()
        .iter()
        .try_fold(0_usize, |used, candidate| {
            used.checked_add(protocol::canonical_candidate(candidate)?.len())
                .ok_or_else(|| anyhow::anyhow!("source candidate byte overflow"))
        })?;
    ensure!(source_bytes <= max_bytes, protocol::SyncError::WorkLimit);
    ensure!(
        std::ptr::eq(tx, evidence.transaction),
        "range evidence belongs to another transaction"
    );
    let event = evidence.event();
    let namespace = event.source.source.namespace;
    crate::causal_leases::lock_on(tx, namespace).await?;
    let meta_query = Query::select()
        .columns(["floor_state_seq", "state_seq", "baseline_generation"].map(Alias::new))
        .from(Alias::new("sync_meta"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .to_owned();
    let meta = tx
        .query_one(&meta_query)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing causal namespace"))?;
    ensure!(
        meta.try_get_by_index::<i64>(0)? == floor_state
            && meta.try_get_by_index::<i64>(1)? == state,
        "stale candidate join basis"
    );
    let tag = M::collection_tag(&event.range.collection).as_bytes();
    let page = crate::causal_candidates::page_on(
        tx,
        crate::causal_candidates::CandidatePageRequest {
            namespace,
            collection: tag,
            state_seq: state,
            lower: event.range.lower,
            upper: event.range.upper,
            after: None,
            max_items: max_candidates,
            max_bytes,
        },
    )
    .await?;
    ensure!(page.done, protocol::SyncError::WorkLimit);
    let mut local = BTreeMap::<Uuid, Vec<Candidate<M>>>::new();
    for row in page.rows {
        let candidate = protocol::decode_candidate::<M>(&row.body)?;
        ensure!(
            candidate.namespace == namespace
                && candidate.key.collection == event.range.collection
                && candidate.key.id.as_bytes() == row.object.as_slice()
                && candidate.dot.writer_incarnation.as_bytes() == row.writer.as_slice()
                && i64::try_from(candidate.dot.counter)? == row.counter
                && candidate.revision.actor.as_bytes() == row.author.as_slice()
                && i64::try_from(candidate.revision.counter)? == row.revision
                && protocol::candidate_digest(&candidate)?.as_slice() == row.digest.as_slice(),
            "stored candidate identity mismatch"
        );
        local.entry(candidate.key.id).or_default().push(candidate);
    }
    let mut incoming = BTreeMap::<Uuid, Vec<Candidate<M>>>::new();
    for candidate in evidence.candidates() {
        incoming
            .entry(candidate.key.id)
            .or_default()
            .push(candidate.clone());
    }
    let mut ids: BTreeSet<Uuid> = local.keys().copied().collect();
    ids.extend(incoming.keys().copied());
    ensure!(ids.len() <= max_candidates, protocol::SyncError::WorkLimit);
    let mut changes = Vec::new();
    let mut close = Vec::new();
    let mut open = Vec::new();
    for id in ids {
        let before_candidates = local.remove(&id).unwrap_or_default();
        let source = incoming.remove(&id).unwrap_or_default();
        let source_dots: BTreeSet<_> = source.iter().map(|c| c.dot).collect();
        let local_dots: BTreeSet<_> = before_candidates.iter().map(|c| c.dot).collect();
        let segment_index = evidence
            .segments()
            .partition_point(|s| s.range.lower.is_none_or(|lo| lo <= id));
        let segment = segment_index
            .checked_sub(1)
            .and_then(|i| evidence.segments().get(i))
            .ok_or_else(|| anyhow::anyhow!("candidate outside source context"))?;
        ensure!(
            segment.range.upper.is_none_or(|hi| id < hi),
            "candidate outside source context"
        );
        let context = evidence
            .context(&segment.context.digest)
            .ok_or_else(|| anyhow::anyhow!("missing source context"))?;
        let before = crate::causal_store::winner(&before_candidates)?;
        let mut after_candidates = Vec::new();
        for candidate in before_candidates {
            let dot =
                sync::causal::Dot::new(candidate.dot.writer_incarnation, candidate.dot.counter)?;
            if !source_dots.contains(&candidate.dot) && context.contains(dot) {
                close.push(candidate.dot);
            } else {
                after_candidates.push(candidate);
            }
        }
        for candidate in source {
            if local_dots.contains(&candidate.dot) {
                continue;
            }
            let mut bound = vec![1_u8];
            bound.extend_from_slice(id.as_bytes());
            let probe = ContextScan::new(
                namespace,
                tag.to_vec(),
                bound,
                candidate.dot.writer_incarnation,
                i64::try_from(candidate.dot.counter)?,
                floor_state,
                state,
            )?;
            match scan_on(tx, probe, 1024).await? {
                ScanResult::Absent => {
                    after_candidates.push(candidate.clone());
                    open.push(candidate);
                }
                ScanResult::Observed => {}
                ScanResult::Incomplete(_) => return Err(protocol::SyncError::WorkLimit.into()),
            }
        }
        let after = crate::causal_store::winner(&after_candidates)?;
        if after.is_none() {
            let key = protocol::ObjectKey {
                collection: event.range.collection.clone(),
                id,
            };
            M::validate(&key, &None)?;
        }
        if before != after {
            changes.push(protocol::ProjectionChange {
                key: protocol::ObjectKey {
                    collection: event.range.collection.clone(),
                    id,
                },
                before,
                after,
            });
        }
    }
    Ok(CandidateJoin {
        changes,
        close,
        open,
    })
}

/// Compare the current exact observation function with a source range and
/// promoted candidate dots. A successful result is a preflight only; callers
/// must publish the assertions and event in the same locked transaction.
pub async fn context_join_changed_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    evidence: &ValidatedRange<'_, M>,
    floor_state: i64,
    state: i64,
    max_rows: u64,
    max_bytes: u64,
) -> Result<bool> {
    ensure!(
        std::ptr::eq(tx, evidence.transaction),
        "range evidence belongs to another transaction"
    );
    ensure!(
        max_rows > 0
            && max_rows <= 1024
            && max_bytes > 0
            && max_bytes <= protocol::MAX_LOGICAL_BYTES,
        "invalid context join budget"
    );
    ensure!(
        evidence.segments().len() as u64 <= max_rows,
        protocol::SyncError::WorkLimit
    );
    let event = evidence.event();
    let ns = event.source.source.namespace;
    crate::causal_leases::lock_on(tx, ns).await?;
    let meta = Query::select()
        .columns(["floor_state_seq", "state_seq", "baseline_generation"].map(Alias::new))
        .from(Alias::new("sync_meta"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .to_owned();
    let meta = tx
        .query_one(&meta)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing causal namespace"))?;
    ensure!(
        meta.try_get_by_index::<i64>(0)? == floor_state
            && meta.try_get_by_index::<i64>(1)? == state,
        "stale context join basis"
    );
    let query = Query::select()
        .columns(
            [
                "state_seq",
                "lower_bound",
                "upper_bound",
                "context_manifest",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_context_ranges"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(
            Expr::col(Alias::new("collection")).eq(M::collection_tag(&event.range.collection)
                .as_bytes()
                .to_vec()),
        )
        .and_where(Expr::col(Alias::new("state_seq")).gte(floor_state))
        .and_where(Expr::col(Alias::new("state_seq")).lte(state))
        .cond_where(crate::causal_context::active_basis_condition(
            floor_state,
            meta.try_get_by_index::<Vec<u8>>(2)?,
        ))
        .limit(max_rows + 1)
        .to_owned();
    let rows = tx.query_all(&query).await?;
    ensure!(
        rows.len() as u64 <= max_rows,
        protocol::SyncError::WorkLimit
    );
    let mut old = Vec::new();
    let mut baseline = Vec::new();
    let mut consumed = 0_u64;
    let mut fragments = 0_u64;
    let mut contexts =
        BTreeMap::<protocol::Digest, (u64, u32, std::sync::Arc<sync::causal::Context>)>::new();
    for row in rows {
        let position: i64 = row.try_get_by_index(0)?;
        let lower: Vec<u8> = row.try_get_by_index(1)?;
        let upper: Vec<u8> = row.try_get_by_index(2)?;
        let digest: Vec<u8> = row.try_get_by_index(3)?;
        ensure!(
            crate::causal_compaction::valid_bound(&lower)
                && crate::causal_compaction::valid_bound(&upper)
                && lower < upper,
            "invalid local context bounds"
        );
        if floor_state > 0 && position == floor_state {
            baseline.push((lower.clone(), upper.clone()));
        }
        let digest: protocol::Digest = digest
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid local context digest"))?;
        let remaining = max_bytes
            .checked_sub(consumed)
            .ok_or_else(|| anyhow::Error::new(protocol::SyncError::WorkLimit))?;
        ensure!(remaining > 0, protocol::SyncError::WorkLimit);
        let (body_bytes, fragment_count, context) = match contexts.get(&digest) {
            Some(context) => context.clone(),
            None => {
                let (manifest, body) =
                    read_on(tx, ns, digest, remaining, u32::try_from(max_rows)?).await?;
                ensure!(
                    manifest.kind == ManifestKind::Context,
                    "invalid local context manifest"
                );
                let decoded = protocol::decode_context(&body)?;
                let context = (
                    body.len() as u64,
                    manifest.fragment_count,
                    std::sync::Arc::new(protocol::causal_context(&decoded)?),
                );
                contexts.insert(digest, context.clone());
                context
            }
        };
        fragments = fragments
            .checked_add(u64::from(fragment_count))
            .ok_or_else(|| anyhow::anyhow!("context fragment overflow"))?;
        ensure!(fragments <= max_rows, "invalid local context manifest");
        consumed = consumed
            .checked_add(body_bytes)
            .ok_or_else(|| anyhow::anyhow!("context byte overflow"))?;
        ensure!(consumed <= max_bytes, protocol::SyncError::WorkLimit);
        old.push((lower, upper, context));
    }
    if floor_state > 0 {
        baseline.sort();
        let mut end = vec![0_u8];
        for (lo, hi) in baseline {
            ensure!(lo == end, "local baseline gap or overlap");
            end = hi;
        }
        ensure!(end == [2], "incomplete local baseline");
    }
    let mut incoming = Vec::new();
    for segment in evidence.segments() {
        let bound = |id: Option<Uuid>, lower: bool| match id {
            None => vec![if lower { 0 } else { 2 }],
            Some(id) => {
                let mut v = vec![1];
                v.extend_from_slice(id.as_bytes());
                v
            }
        };
        incoming.push((
            bound(segment.range.lower, true),
            bound(segment.range.upper, false),
            evidence
                .context(&segment.context.digest)
                .ok_or_else(|| anyhow::anyhow!("missing source context"))?
                .clone(),
        ));
    }
    let mut promoted = sync::causal::Context::default();
    for candidate in evidence.candidates() {
        promoted.insert(sync::causal::Dot::new(
            candidate.dot.writer_incarnation,
            candidate.dot.counter,
        )?)?;
    }
    incoming.push((vec![0], vec![2], promoted));
    let mut comparisons = 0u64;
    // Joining only adds observations. Compare incoming observations against
    // the current function and stop at the first uncovered interval; unrelated
    // existing regions do not need to be normalized twice for every receive.
    for (lower, upper, desired) in incoming {
        // A writer prefix carries observations without any sparse intervals.
        if desired.entries().next().is_none() {
            continue;
        }
        let mut boundaries = std::collections::BTreeSet::from([lower.clone(), upper.clone()]);
        let mut covering = sync::causal::Context::default();
        let mut partial = Vec::new();
        for (lo, hi, context) in &old {
            comparisons = comparisons
                .checked_add(1)
                .ok_or(protocol::SyncError::WorkLimit)?;
            ensure!(comparisons <= max_rows, protocol::SyncError::WorkLimit);
            if *lo <= lower && upper <= *hi {
                covering = covering.union(context)?;
            } else if *lo < upper && lower < *hi {
                boundaries.insert(std::cmp::max(lo.clone(), lower.clone()));
                boundaries.insert(std::cmp::min(hi.clone(), upper.clone()));
                partial.push((lo, hi, context));
            }
        }
        if covering.union(&desired)? == covering {
            continue;
        }
        let boundaries: Vec<_> = boundaries.into_iter().collect();
        for pair in boundaries.windows(2) {
            let mut actual = covering.clone();
            for (lo, hi, context) in &partial {
                comparisons = comparisons
                    .checked_add(1)
                    .ok_or(protocol::SyncError::WorkLimit)?;
                ensure!(comparisons <= max_rows, protocol::SyncError::WorkLimit);
                if **lo <= pair[0] && pair[1] <= **hi {
                    actual = actual.union(context)?;
                }
            }
            let joined = actual.union(&desired)?;
            consumed = consumed
                .checked_add(
                    protocol::canonical_context(&protocol::exact_context(&joined))?.len() as u64,
                )
                .ok_or(protocol::SyncError::WorkLimit)?;
            ensure!(consumed <= max_bytes, protocol::SyncError::WorkLimit);
            if joined != actual {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Materialize the exact interval index of a validated context manifest.
/// Repeated indexing is idempotent and remains in the caller transaction.
pub(crate) async fn index_context_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    manifest: protocol::Digest,
    context: &sync::causal::Context,
    remaining: &mut u64,
) -> Result<()> {
    // Five parameters per row keep each statement below SQLite's common
    // 999-parameter limit, independently of the admitted context size.
    const BATCH_SIZE: usize = 100;
    let mut batch = Vec::with_capacity(BATCH_SIZE);
    for (writer, prefix, intervals) in context.entries() {
        for (first, last) in (prefix > 0)
            .then_some((1, prefix))
            .into_iter()
            .chain(intervals.iter().copied())
        {
            *remaining = remaining
                .checked_sub(1)
                .ok_or_else(|| anyhow::Error::new(protocol::SyncError::WorkLimit))?;
            batch.push((writer, i64::try_from(first)?, i64::try_from(last)?));
            if batch.len() == BATCH_SIZE {
                index_context_batch_on(tx, namespace, manifest, &batch).await?;
                batch.clear();
            }
        }
    }
    if !batch.is_empty() {
        index_context_batch_on(tx, namespace, manifest, &batch).await?;
    }

    Ok(())
}

async fn index_context_batch_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    manifest: protocol::Digest,
    batch: &[(Uuid, i64, i64)],
) -> Result<()> {
    use sea_orm::sea_query::{Condition, OnConflict};
    let mut insert = Query::insert();
    insert
        .into_table(Alias::new("sync_context_intervals"))
        .columns(["namespace", "context_manifest", "writer", "first", "last"].map(Alias::new))
        .on_conflict(
            OnConflict::columns(
                ["namespace", "context_manifest", "writer", "first"].map(Alias::new),
            )
            .do_nothing()
            .to_owned(),
        );
    let mut keys = Condition::any();
    let mut expected = BTreeMap::new();
    for &(writer, first, last) in batch {
        insert.values_panic([
            namespace.as_bytes().to_vec().into(),
            manifest.to_vec().into(),
            writer.as_bytes().to_vec().into(),
            first.into(),
            last.into(),
        ]);
        keys = keys.add(
            Condition::all()
                .add(Expr::col(Alias::new("writer")).eq(writer.as_bytes().to_vec()))
                .add(Expr::col(Alias::new("first")).eq(first)),
        );
        ensure!(
            expected.insert((writer, first), last).is_none(),
            "duplicate context interval"
        );
    }
    tx.execute(&insert.to_owned()).await?;
    let query = Query::select()
        .columns(["writer", "first", "last"].map(Alias::new))
        .from(Alias::new("sync_context_intervals"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("context_manifest")).eq(manifest.to_vec()))
        .cond_where(keys)
        .to_owned();
    for row in tx.query_all(&query).await? {
        let writer = Uuid::from_slice(&row.try_get_by_index::<Vec<u8>>(0)?)?;
        let first = row.try_get_by_index::<i64>(1)?;
        let last = row.try_get_by_index::<i64>(2)?;
        ensure!(
            expected.remove(&(writer, first)) == Some(last),
            "conflicting indexed interval"
        );
    }
    ensure!(expected.is_empty(), "missing indexed interval");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{Database, TransactionTrait};

    #[tokio::test]
    async fn interval_batches_preserve_gaps_and_reject_conflicts_and_budget_overflow() -> Result<()>
    {
        let db = Database::connect("sqlite::memory:").await?;
        crate::causal_schema::install_fresh(&db).await?;
        let namespace = Uuid::new_v4();
        let writer = Uuid::new_v4();
        let manifest = [7_u8; 32];
        let mut context = sync::causal::Context::default();
        for n in 0..205 {
            context.insert_interval(writer, 2 * n + 1, 2 * n + 1)?;
        }
        let tx = db.begin().await?;
        let mut remaining = 205;
        index_context_on(&tx, namespace, manifest, &context, &mut remaining).await?;
        assert_eq!(remaining, 0);
        let mut remaining = 205;
        index_context_on(&tx, namespace, manifest, &context, &mut remaining).await?;
        assert_eq!(remaining, 0);
        let count = Query::select()
            .expr(sea_orm::sea_query::Func::count(Expr::val(1)))
            .from(Alias::new("sync_context_intervals"))
            .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
            .to_owned();
        assert_eq!(
            tx.query_one(&count)
                .await?
                .unwrap()
                .try_get_by_index::<i64>(0)?,
            205
        );
        tx.commit().await?;

        let tx = db.begin().await?;
        let mut insufficient = 204;
        assert!(
            index_context_on(&tx, namespace, manifest, &context, &mut insufficient)
                .await
                .is_err()
        );
        tx.rollback().await?;

        let tx = db.begin().await?;
        let corrupt = Query::update()
            .table(Alias::new("sync_context_intervals"))
            .value(Alias::new("last"), 2_i64)
            .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("first")).eq(1_i64))
            .to_owned();
        tx.execute(&corrupt).await?;
        let mut remaining = 205;
        assert!(
            index_context_on(&tx, namespace, manifest, &context, &mut remaining)
                .await
                .is_err()
        );
        tx.rollback().await?;
        let tx = db.begin().await?;
        let mut remaining = 205;
        index_context_on(&tx, namespace, manifest, &context, &mut remaining).await?;
        tx.rollback().await?;
        Ok(())
    }
}
