//! Atomic source snapshot construction and reads from published source views.
//! Resumable construction is provided by `causal_source`.
use crate::{causal_candidates, causal_leases, causal_manifests};
use anyhow::{Result, ensure};
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, ExprTrait, FromQueryResult,
    sea_query::{Alias, Expr, Query},
};
use sync::protocol::{
    self, ContextPartitionRef, ContextRef, ContextRoot, ContextSegment, KeyRange, ManifestKind,
    RangeSummary, ReceiveProgress, Scope, SnapshotDescriptor, StoreIdentity, SyncModel, WorkLimits,
};
use uuid::Uuid;
#[derive(FromQueryResult)]
struct Assertion {
    lower_bound: Vec<u8>,
    upper_bound: Vec<u8>,
    state_seq: i64,
    context_manifest: Vec<u8>,
}

/// Construct and publish a ready durable view in a single caller transaction.
/// Large states outside the admitted work are rejected rather than
/// represented as complete.
pub async fn open_bounded_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    source: StoreIdentity,
    scope: Scope<M>,
    now_ms: i64,
    ttl_ms: i64,
    limits: WorkLimits,
) -> Result<SnapshotDescriptor<M>> {
    ensure!(
        now_ms >= 0
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
            && limits.max_depth <= protocol::MAX_DEPTH,
        "invalid snapshot admission"
    );
    protocol::validate_model::<M>()?;
    protocol::scope_id(&scope)?;
    ensure!(
        !source.namespace.is_nil()
            && source.model.model == M::model_id()
            && source.model.schema == M::schema_id(),
        "invalid snapshot identity"
    );
    let id = Uuid::new_v4();
    let expires = now_ms
        .checked_add(ttl_ms)
        .ok_or_else(|| anyhow::anyhow!("snapshot expiry overflow"))?;
    let lease = causal_leases::create_build_on(
        tx,
        crate::causal_leases::BuildLeaseRequest {
            namespace: source.namespace,
            id,
            scope: serde_json::to_vec(&scope)?,
            now_ms,
            expires_at_ms: expires,
            max_active: 16,
            max_ttl_ms: protocol::MAX_SNAPSHOT_TTL_MS as i64,
        },
    )
    .await?;
    ensure!(
        source.generation.as_bytes() == lease.generation.as_slice(),
        "snapshot generation changed"
    );
    // Verify replica and floor while holding the same namespace write lock.
    let q = Query::select()
        .columns(["replica", "floor_state_seq", "model", "schema"].map(Alias::new))
        .from(Alias::new("sync_meta"))
        .and_where(Expr::col(Alias::new("namespace")).eq(source.namespace.as_bytes().to_vec()))
        .to_owned();
    let meta = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing namespace"))?;
    ensure!(
        meta.try_get_by_index::<Vec<u8>>(0)? == source.replica.as_bytes(),
        "snapshot replica changed"
    );
    ensure!(
        meta.try_get_by_index::<String>(2)? == M::model_id()
            && meta.try_get_by_index::<String>(3)? == M::schema_id(),
        "snapshot model mismatch"
    );
    let floor: i64 = meta.try_get_by_index(1)?;
    ensure!(
        floor >= 0 && floor <= lease.state_seq && lease.basis_state_seq == Some(floor),
        "invalid snapshot floor"
    );
    let basis = lease
        .basis_generation
        .clone()
        .ok_or_else(|| anyhow::anyhow!("missing snapshot basis generation"))?;
    ensure!(basis.len() == 16, "invalid snapshot basis generation");
    let mut collections: Vec<_> = scope.collections.iter().cloned().collect();
    collections.sort_by(|a, b| {
        M::collection_tag(a)
            .as_bytes()
            .cmp(M::collection_tag(b).as_bytes())
    });
    ensure!(
        (collections.len() as u64) * 12 + 10 <= limits.max_mutations,
        "snapshot mutation limit"
    );
    let mut roots = Vec::new();
    let mut partitions = Vec::new();
    let mut used_rows = 0_u64;
    let mut used_bytes = 0_u64;
    let mut used_mutations = (collections.len() as u64) * 12 + 10;
    for collection in collections {
        let tag = M::collection_tag(&collection).as_bytes().to_vec();
        let q = Query::select()
            .columns(
                [
                    "lower_bound",
                    "upper_bound",
                    "state_seq",
                    "context_manifest",
                ]
                .map(Alias::new),
            )
            .from(Alias::new("sync_context_ranges"))
            .and_where(Expr::col(Alias::new("namespace")).eq(source.namespace.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("collection")).eq(tag.clone()))
            .cond_where(crate::causal_context::active_basis_condition(
                floor,
                basis.clone(),
            ))
            .and_where(Expr::col(Alias::new("state_seq")).lte(lease.state_seq))
            .limit(limits.max_rows_scanned - used_rows + 1)
            .to_owned();
        let rows = Assertion::find_by_statement(tx.get_database_backend().build(&q))
            .all(tx)
            .await?;
        used_rows = used_rows
            .checked_add(rows.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("snapshot work overflow"))?;
        ensure!(used_rows <= limits.max_rows_scanned, "snapshot row limit");
        let mut input = Vec::new();
        let mut baseline = Vec::new();
        for row in rows {
            ensure!(
                crate::causal_compaction::valid_bound(&row.lower_bound)
                    && crate::causal_compaction::valid_bound(&row.upper_bound)
                    && row.lower_bound < row.upper_bound,
                "invalid context bounds"
            );
            if floor > 0 && row.state_seq == floor {
                baseline.push((row.lower_bound.clone(), row.upper_bound.clone()));
            }
            let digest: [u8; 32] = row
                .context_manifest
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid context reference"))?;
            let remaining = limits
                .max_bytes
                .checked_sub(used_bytes)
                .ok_or_else(|| anyhow::anyhow!("snapshot byte limit"))?;
            ensure!(remaining > 0, "snapshot byte limit");
            let (manifest, body) = causal_manifests::read_on(
                tx,
                source.namespace,
                digest,
                remaining,
                std::cmp::min(
                    u32::try_from(limits.max_rows_scanned.saturating_sub(used_rows))
                        .unwrap_or(u32::MAX),
                    protocol::MAX_FRAGMENTS,
                ),
            )
            .await?;
            used_rows = used_rows
                .checked_add(u64::from(manifest.fragment_count))
                .ok_or_else(|| anyhow::anyhow!("snapshot work overflow"))?;
            ensure!(used_rows <= limits.max_rows_scanned, "snapshot row limit");
            ensure!(
                manifest.kind == ManifestKind::Context,
                "invalid context kind"
            );
            used_bytes += body.len() as u64;
            input.push((
                row.lower_bound,
                row.upper_bound,
                protocol::causal_context(&protocol::decode_context(&body)?)?,
            ));
        }
        if floor > 0 {
            baseline.sort();
            let mut end = vec![0_u8];
            for (lo, hi) in baseline {
                ensure!(lo == end, "baseline gap or overlap");
                end = hi;
            }
            ensure!(end == [2], "missing baseline coverage");
        }
        let remaining_work = limits
            .max_rows_scanned
            .checked_sub(used_rows)
            .ok_or_else(|| anyhow::anyhow!("snapshot row limit"))?;
        let remaining_bytes = limits
            .max_bytes
            .checked_sub(used_bytes)
            .ok_or_else(|| anyhow::anyhow!("snapshot byte limit"))?;
        let (pieces, comparisons, normalized_bytes) =
            crate::causal_compaction::normalize(&input, remaining_work, remaining_bytes)?;
        used_rows += comparisons;
        used_bytes += normalized_bytes;
        let range = KeyRange {
            collection: collection.clone(),
            lower: None,
            upper: None,
        };
        let mut segments = Vec::new();
        let mut children = Vec::new();
        for (lower, upper, context) in pieces {
            let exact = protocol::exact_context(&context);
            let context_bytes = protocol::canonical_context(&exact)?;
            let interval_count = context
                .entries()
                .map(|(_, prefix, intervals)| u64::from(prefix > 0) + intervals.len() as u64)
                .sum::<u64>();
            used_mutations = used_mutations
                .checked_add(interval_count + 8)
                .ok_or_else(|| anyhow::anyhow!("snapshot mutation overflow"))?;
            ensure!(
                used_mutations <= limits.max_mutations,
                "snapshot mutation limit"
            );
            let (manifest, fragments) = causal_manifests::fragment_body(
                ManifestKind::Context,
                &context_bytes,
                exact.writers.len() as u64,
                1024 * 1024,
            )?;
            let inserted = causal_manifests::publish_on(
                tx,
                source.namespace,
                &manifest,
                &fragments,
                limits.max_bytes,
                |bytes| {
                    protocol::decode_context(bytes)?;
                    Ok(vec![])
                },
            )
            .await?;
            if inserted {
                for (writer, prefix, intervals) in context.entries() {
                    for (first, last) in (prefix > 0)
                        .then_some((1, prefix))
                        .into_iter()
                        .chain(intervals.iter().copied())
                    {
                        let q = Query::insert()
                            .into_table(Alias::new("sync_context_intervals"))
                            .columns(
                                ["namespace", "context_manifest", "writer", "first", "last"]
                                    .map(Alias::new),
                            )
                            .values_panic([
                                source.namespace.as_bytes().to_vec().into(),
                                manifest.manifest_digest.to_vec().into(),
                                writer.as_bytes().to_vec().into(),
                                i64::try_from(first)?.into(),
                                i64::try_from(last)?.into(),
                            ])
                            .to_owned();
                        tx.execute(&q).await?;
                    }
                }
            }
            children.push(manifest.manifest_digest);
            segments.push(ContextSegment {
                range: KeyRange {
                    collection: collection.clone(),
                    lower: crate::causal_compaction::decode_bound(&lower, true)?,
                    upper: crate::causal_compaction::decode_bound(&upper, false)?,
                },
                context: ContextRef {
                    digest: protocol::context_digest(&exact)?,
                    manifest,
                },
            });
        }
        let part_bytes = protocol::canonical_context_partition(&range, &segments)?;
        used_bytes = used_bytes
            .checked_add(part_bytes.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("snapshot byte overflow"))?;
        ensure!(used_bytes <= limits.max_bytes, "snapshot byte limit");
        let (part, fragments) = causal_manifests::fragment_body(
            ManifestKind::ContextPartition,
            &part_bytes,
            segments.len() as u64,
            1024 * 1024,
        )?;
        causal_manifests::publish_on(
            tx,
            source.namespace,
            &part,
            &fragments,
            limits.max_bytes,
            |bytes| {
                protocol::decode_context_partition::<M>(&range, bytes)?;
                Ok(children)
            },
        )
        .await?;
        // First pass obtains exact hash counts; second feeds candidates in the
        // same immutable transaction and checks every physical/typed identity.
        let mut count = 0_u64;
        let mut byte_count = 0_u64;
        let mut cursor = None;
        loop {
            let page = causal_candidates::page_on(
                tx,
                crate::causal_candidates::CandidatePageRequest {
                    namespace: source.namespace,
                    collection: &tag,
                    state_seq: lease.state_seq,
                    lower: None,
                    upper: None,
                    after: cursor,
                    max_items: 128,
                    max_bytes: protocol::MAX_PAGE_BYTES as usize,
                },
            )
            .await?;
            for row in &page.rows {
                let candidate = protocol::decode_candidate::<M>(&row.body)?;
                ensure!(
                    candidate.namespace == source.namespace
                        && candidate.key.collection == collection
                        && candidate.key.id.as_bytes() == row.object.as_slice()
                        && candidate.dot.writer_incarnation.as_bytes() == row.writer.as_slice()
                        && i64::try_from(candidate.dot.counter)? == row.counter
                        && candidate.revision.actor.as_bytes() == row.author.as_slice()
                        && i64::try_from(candidate.revision.counter)? == row.revision
                        && protocol::candidate_digest(&candidate)?.as_slice()
                            == row.digest.as_slice(),
                    "candidate identity mismatch"
                );
                count += 1;
                byte_count = byte_count
                    .checked_add(row.body.len() as u64)
                    .ok_or_else(|| anyhow::anyhow!("candidate bytes overflow"))?;
            }
            used_rows = used_rows
                .checked_add(page.rows.len() as u64)
                .ok_or_else(|| anyhow::anyhow!("snapshot work overflow"))?;
            ensure!(
                used_rows <= limits.max_rows_scanned
                    && used_bytes
                        .checked_add(byte_count)
                        .is_some_and(|n| n <= limits.max_bytes),
                "snapshot work limit"
            );
            if page.done {
                break;
            }
            cursor = page.next;
        }
        let mut hasher = protocol::RangeHasher::<M>::new(
            &scope,
            range.clone(),
            count,
            byte_count,
            segments.len() as u64,
        )?;
        for segment in &segments {
            hasher.push_segment(&segment.range, segment.context.digest)?;
        }
        let mut cursor = None;
        loop {
            let page = causal_candidates::page_on(
                tx,
                crate::causal_candidates::CandidatePageRequest {
                    namespace: source.namespace,
                    collection: &tag,
                    state_seq: lease.state_seq,
                    lower: None,
                    upper: None,
                    after: cursor,
                    max_items: 128,
                    max_bytes: protocol::MAX_PAGE_BYTES as usize,
                },
            )
            .await?;
            used_rows = used_rows
                .checked_add(page.rows.len() as u64)
                .ok_or_else(|| anyhow::anyhow!("snapshot work overflow"))?;
            ensure!(used_rows <= limits.max_rows_scanned, "snapshot row limit");
            for row in &page.rows {
                hasher.push_candidate(&protocol::decode_candidate::<M>(&row.body)?)?;
            }
            if page.done {
                break;
            }
            cursor = page.next;
        }
        used_bytes = used_bytes
            .checked_add(byte_count)
            .ok_or_else(|| anyhow::anyhow!("snapshot byte overflow"))?;
        let (aggregate, context_commitment) = hasher.finish_with_context()?;
        roots.push(RangeSummary {
            snapshot_id: id,
            range,
            candidate_count: count,
            candidate_bytes: byte_count,
            context_segments: segments.len() as u64,
            context_commitment,
            aggregate,
        });
        partitions.push(ContextPartitionRef {
            collection,
            partition: part,
        });
    }
    let root = ContextRoot {
        scope: scope.clone(),
        partitions,
    };
    let root_bytes = protocol::canonical_context_root(&root)?;
    used_bytes = used_bytes
        .checked_add(root_bytes.len() as u64)
        .ok_or_else(|| anyhow::anyhow!("snapshot byte overflow"))?;
    ensure!(used_bytes <= limits.max_bytes, "snapshot byte limit");
    let (manifest, fragments) = causal_manifests::fragment_body(
        ManifestKind::ContextRoot,
        &root_bytes,
        root.partitions.len() as u64,
        1024 * 1024,
    )?;
    let children = root
        .partitions
        .iter()
        .map(|p| p.partition.manifest_digest)
        .collect::<Vec<_>>();
    causal_manifests::publish_on(
        tx,
        source.namespace,
        &manifest,
        &fragments,
        limits.max_bytes,
        |bytes| {
            protocol::decode_context_root::<M>(&scope, bytes)?;
            Ok(children)
        },
    )
    .await?;
    let descriptor = SnapshotDescriptor {
        source: source.clone(),
        scope,
        snapshot_id: id,
        head: ReceiveProgress {
            sequence: u64::try_from(lease.head)?,
            anchor: lease
                .head_anchor
                .clone()
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid anchor"))?,
        },
        roots,
        expires_unix_ms: u64::try_from(expires)?,
    };
    causal_leases::pin_root_on(tx, source.namespace, id, manifest.manifest_digest, now_ms).await?;
    causal_leases::publish_on(
        tx,
        source.namespace,
        id,
        serde_json::to_vec(&descriptor)?,
        now_ms,
    )
    .await?;
    Ok(descriptor)
}

/// Verify a durable ready lease and its stored descriptor inside a local read
/// transaction. Expiry/revocation is checked on every access.
pub async fn descriptor_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    id: Uuid,
    now_ms: i64,
) -> Result<(causal_leases::Lease, SnapshotDescriptor<M>)> {
    let lease = causal_leases::ready_on(tx, namespace, id, now_ms).await?;
    let descriptor: SnapshotDescriptor<M> = serde_json::from_slice(
        lease
            .descriptor
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("missing descriptor"))?,
    )?;
    ensure!(
        descriptor.source.namespace == namespace
            && descriptor.source.model.model == M::model_id()
            && descriptor.source.model.schema == M::schema_id()
            && descriptor.snapshot_id == id
            && descriptor.source.generation.as_bytes() == lease.generation.as_slice()
            && descriptor.head.sequence == u64::try_from(lease.head)?
            && descriptor.head.anchor.as_slice() == lease.head_anchor.as_slice()
            && descriptor.expires_unix_ms == u64::try_from(lease.expires_at_ms)?
            && serde_json::to_vec(&descriptor.scope)? == lease.scope,
        "snapshot descriptor mismatch"
    );
    let root_digest: [u8; 32] = lease
        .retained_root
        .clone()
        .ok_or_else(|| anyhow::anyhow!("missing snapshot root"))?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid snapshot root"))?;
    let (manifest, body) = causal_manifests::read_on(
        tx,
        namespace,
        root_digest,
        protocol::MAX_LOGICAL_BYTES,
        protocol::MAX_FRAGMENTS,
    )
    .await?;
    ensure!(
        manifest.kind == ManifestKind::ContextRoot,
        "invalid snapshot root kind"
    );
    protocol::decode_context_root::<M>(&descriptor.scope, &body)?;
    Ok((lease, descriptor))
}

/// Read candidates from a valid pinned view. This helper has no live-row
/// fallback and returns a bounded page at the captured state position.
pub async fn leaf_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    id: Uuid,
    now_ms: i64,
    range: KeyRange<M>,
    after: Option<protocol::CandidateCursor<M>>,
    limits: protocol::PageLimits,
) -> Result<protocol::LeafPage<M>> {
    ensure!(
        limits.max_scanned > 0
            && limits.max_scanned <= protocol::MAX_PAGE_ITEMS
            && limits.max_bytes > 0
            && limits.max_bytes <= protocol::MAX_PAGE_BYTES
            && range.lower.zip(range.upper).is_none_or(|(a, b)| a < b),
        "invalid leaf limits or range"
    );
    let (lease, descriptor) = descriptor_on::<M>(tx, namespace, id, now_ms).await?;
    ensure!(
        descriptor.scope.collections.contains(&range.collection),
        "range outside snapshot scope"
    );
    if let Some(cursor) = &after {
        ensure!(
            cursor.key.collection == range.collection
                && range.lower.is_none_or(|lo| cursor.key.id >= lo)
                && range.upper.is_none_or(|hi| cursor.key.id < hi),
            "cursor outside range"
        );
    }
    let mut page = protocol::LeafPage {
        snapshot_id: id,
        range: range.clone(),
        after: after.clone(),
        next: after.clone(),
        done: false,
        candidates: vec![],
    };
    let base = serde_json::to_vec(&page)?.len();
    ensure!(
        base as u64 <= limits.max_bytes,
        "leaf header exceeds byte limit"
    );
    let cursor = after
        .map(|c| -> Result<causal_candidates::Cursor> {
            Ok(causal_candidates::Cursor {
                object: c.key.id,
                writer: c.dot.writer_incarnation,
                counter: i64::try_from(c.dot.counter)?,
            })
        })
        .transpose()?;
    // Bound the retained bodies by the requested page size as well as the
    // number of scanned candidates. The database page includes a conservative
    // per-row allowance, so a response may stop before its exact JSON limit.
    let raw = causal_candidates::page_on(
        tx,
        crate::causal_candidates::CandidatePageRequest {
            namespace,
            collection: M::collection_tag(&range.collection).as_bytes(),
            state_seq: lease.state_seq,
            lower: range.lower,
            upper: range.upper,
            after: cursor,
            max_items: limits.max_scanned as usize,
            max_bytes: limits.max_bytes as usize,
        },
    )
    .await?;
    let raw_len = raw.rows.len();
    let mut value_bytes = 0_usize;
    page.done = raw.done && raw.rows.is_empty();
    for (index, row) in raw.rows.into_iter().enumerate() {
        let candidate = protocol::decode_candidate::<M>(&row.body)?;
        ensure!(
            candidate.namespace == namespace
                && candidate.key.collection == range.collection
                && candidate.key.id.as_bytes() == row.object.as_slice()
                && candidate.dot.writer_incarnation.as_bytes() == row.writer.as_slice()
                && i64::try_from(candidate.dot.counter)? == row.counter
                && candidate.revision.actor.as_bytes() == row.author.as_slice()
                && i64::try_from(candidate.revision.counter)? == row.revision
                && protocol::candidate_digest(&candidate)?.as_slice() == row.digest.as_slice(),
            "candidate identity mismatch"
        );
        let next = protocol::CandidateCursor {
            key: candidate.key.clone(),
            dot: candidate.dot,
        };
        let value_size = serde_json::to_vec(&candidate)?.len();
        let done = raw.done && index + 1 == raw_len;
        let header = protocol::LeafPage::<M> {
            snapshot_id: id,
            range: range.clone(),
            after: page.after.clone(),
            next: Some(next.clone()),
            done,
            candidates: vec![],
        };
        let total = serde_json::to_vec(&header)?
            .len()
            .checked_add(value_bytes)
            .and_then(|v| v.checked_add(value_size))
            .and_then(|v| v.checked_add(page.candidates.len()))
            .ok_or_else(|| anyhow::anyhow!("leaf byte overflow"))?;
        if total as u64 > limits.max_bytes {
            ensure!(
                !page.candidates.is_empty(),
                "candidate does not fit leaf byte limit"
            );
            page.done = false;
            break;
        }
        value_bytes += value_size;
        page.next = Some(next);
        page.done = done;
        page.candidates.push(candidate);
    }
    ensure!(
        page.done || !page.candidates.is_empty(),
        "leaf did not advance"
    );
    ensure!(
        serde_json::to_vec(&page)?.len() as u64 <= limits.max_bytes,
        "leaf exceeds byte limit"
    );
    Ok(page)
}

/// Return the exact retained context partition clipped to the requested range.
pub async fn context_page_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    id: Uuid,
    now_ms: i64,
    range: KeyRange<M>,
    after: protocol::ContextCursor,
    limits: protocol::PageLimits,
) -> Result<protocol::ContextPage<M>> {
    ensure!(
        limits.max_scanned > 0
            && limits.max_scanned <= protocol::MAX_PAGE_ITEMS
            && limits.max_bytes > 0
            && limits.max_bytes <= protocol::MAX_PAGE_BYTES
            && range.lower.zip(range.upper).is_none_or(|(a, b)| a < b),
        "invalid context page limits"
    );
    let preliminary = causal_leases::load_on(tx, namespace, id)
        .await?
        .ok_or(protocol::SyncError::SnapshotExpired)?;
    if preliminary.output_generation.is_some() {
        if preliminary.status != 1 || preliminary.expires_at_ms <= now_ms {
            return Err(protocol::SyncError::SnapshotExpired.into());
        }
        let descriptor: SnapshotDescriptor<M> = serde_json::from_slice(
            preliminary
                .descriptor
                .as_deref()
                .ok_or(protocol::SyncError::SnapshotExpired)?,
        )?;
        ensure!(
            descriptor.source.namespace == namespace
                && descriptor.snapshot_id == id
                && descriptor.source.model.model == M::model_id()
                && descriptor.source.model.schema == M::schema_id()
                && descriptor.source.generation.as_bytes() == preliminary.generation.as_slice()
                && serde_json::to_vec(&descriptor.scope)? == preliminary.scope
                && descriptor.scope.collections.contains(&range.collection),
            "snapshot descriptor mismatch"
        );
        let generation = crate::causal_source::ready_generation(&preliminary, &descriptor)?;
        return context_page_indexed_on(
            tx,
            IndexedContextView {
                namespace,
                id,
                lease: &preliminary,
                generation,
            },
            range,
            after,
            limits,
        )
        .await;
    }
    let (lease, descriptor) = descriptor_on::<M>(tx, namespace, id, now_ms).await?;
    ensure!(
        descriptor.scope.collections.contains(&range.collection),
        "range outside snapshot scope"
    );
    let digest: [u8; 32] = lease
        .retained_root
        .ok_or_else(|| anyhow::anyhow!("missing snapshot root"))?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid snapshot root"))?;
    let (_, body) = causal_manifests::read_on(
        tx,
        namespace,
        digest,
        protocol::MAX_LOGICAL_BYTES,
        protocol::MAX_FRAGMENTS,
    )
    .await?;
    let root = protocol::decode_context_root::<M>(&descriptor.scope, &body)?;
    let partition = root
        .partitions
        .iter()
        .find(|p| p.collection == range.collection)
        .ok_or_else(|| anyhow::anyhow!("missing scope partition"))?;
    let (stored, body) = causal_manifests::read_on(
        tx,
        namespace,
        partition.partition.manifest_digest,
        protocol::MAX_LOGICAL_BYTES,
        protocol::MAX_FRAGMENTS,
    )
    .await?;
    ensure!(stored == partition.partition, "context partition mismatch");
    let root_range = KeyRange {
        collection: range.collection.clone(),
        lower: None,
        upper: None,
    };
    let source_segments = protocol::decode_context_partition::<M>(&root_range, &body)?;
    let mut clipped = Vec::new();
    for segment in source_segments {
        if segment
            .range
            .upper
            .zip(range.lower)
            .is_some_and(|(hi, lo)| hi <= lo)
            || range
                .upper
                .zip(segment.range.lower)
                .is_some_and(|(hi, lo)| hi <= lo)
        {
            continue;
        }
        let lower = match (segment.range.lower, range.lower) {
            (Some(a), Some(b)) => Some(std::cmp::max(a, b)),
            (a, None) | (None, a) => a,
        };
        let upper = match (segment.range.upper, range.upper) {
            (Some(a), Some(b)) => Some(std::cmp::min(a, b)),
            (a, None) | (None, a) => a,
        };
        let stored =
            causal_manifests::reference_on(tx, namespace, segment.context.manifest.manifest_digest)
                .await?;
        ensure!(
            stored == segment.context.manifest,
            "snapshot context manifest mismatch"
        );
        clipped.push(ContextSegment {
            range: KeyRange {
                collection: range.collection.clone(),
                lower,
                upper,
            },
            context: segment.context,
        });
    }
    ensure!(!clipped.is_empty(), "missing context coverage");
    let index = usize::try_from(after.segment_index)?;
    ensure!(
        index <= clipped.len()
            && if index == 0 {
                after.next_lower == range.lower
            } else {
                after.next_lower == clipped[index - 1].range.upper
            },
        "stale context cursor"
    );
    let mut segments = Vec::new();
    let mut segment_bytes = 0_usize;
    let mut next = after.clone();
    let mut done = index == clipped.len();
    for segment in clipped.iter().skip(index).take(limits.max_scanned as usize) {
        let candidate_next = protocol::ContextCursor {
            segment_index: next.segment_index + 1,
            next_lower: segment.range.upper,
        };
        let item_bytes = serde_json::to_vec(segment)?.len();
        let trial = protocol::ContextPage::<M> {
            snapshot_id: id,
            range: range.clone(),
            after: after.clone(),
            next: candidate_next.clone(),
            done: candidate_next.segment_index as usize == clipped.len(),
            segments: vec![],
        };
        let total = serde_json::to_vec(&trial)?
            .len()
            .checked_add(segment_bytes)
            .and_then(|n| n.checked_add(item_bytes))
            .and_then(|n| n.checked_add(segments.len()))
            .ok_or_else(|| anyhow::anyhow!("context page byte overflow"))?;
        if total as u64 > limits.max_bytes {
            ensure!(!segments.is_empty(), "context page byte limit too small");
            break;
        }
        segment_bytes += item_bytes;
        next = candidate_next;
        done = next.segment_index as usize == clipped.len();
        segments.push(segment.clone());
    }
    let page = protocol::ContextPage {
        snapshot_id: id,
        range,
        after,
        next,
        done,
        segments,
    };
    ensure!(
        serde_json::to_vec(&page)?.len() as u64 <= limits.max_bytes,
        "context page byte limit too small"
    );
    Ok(page)
}

/// Read a fragment belonging to the pinned context root. A verified root has
/// one partition per collection, and partitions refer directly to contexts.
/// Membership therefore requires at most MAX_COLLECTIONS indexed parents.
pub async fn fragment_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    id: Uuid,
    now_ms: i64,
    requested: protocol::ManifestRef,
    ordinal: u32,
    max_bytes: u64,
) -> Result<protocol::Fragment> {
    ensure!(
        max_bytes > 0
            && max_bytes <= protocol::MAX_PAGE_BYTES
            && ordinal < requested.fragment_count,
        "invalid fragment request"
    );
    let (lease, _) = descriptor_on::<M>(tx, namespace, id, now_ms).await?;
    let root = lease
        .retained_root
        .ok_or_else(|| anyhow::anyhow!("missing snapshot root"))?;
    let requested_digest = requested.manifest_digest.to_vec();
    let found = if root == requested_digest {
        requested.kind == ManifestKind::ContextRoot
    } else {
        let query = Query::select()
            .column(Alias::new("child_manifest"))
            .from(Alias::new("sync_manifest_edges"))
            .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("parent_manifest")).eq(root))
            .limit(protocol::MAX_COLLECTIONS as u64 + 1)
            .to_owned();
        let rows = tx.query_all(&query).await?;
        ensure!(
            rows.len() <= protocol::MAX_COLLECTIONS,
            "invalid root partition count"
        );
        let parents = rows
            .into_iter()
            .map(|row| row.try_get_by_index::<Vec<u8>>(0))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if requested.kind == ManifestKind::ContextPartition {
            parents.contains(&requested_digest)
        } else if requested.kind == ManifestKind::Context && !parents.is_empty() {
            let query = Query::select()
                .expr(Expr::val(1))
                .from(Alias::new("sync_manifest_edges"))
                .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
                .and_where(Expr::col(Alias::new("parent_manifest")).is_in(parents))
                .and_where(Expr::col(Alias::new("child_manifest")).eq(requested_digest))
                .limit(1)
                .to_owned();
            tx.query_one(&query).await?.is_some()
        } else {
            false
        }
    };
    ensure!(found, "manifest not reachable from snapshot");
    let actual = causal_manifests::reference_on(tx, namespace, requested.manifest_digest).await?;
    ensure!(actual == requested, "manifest reference mismatch");
    let q = Query::select()
        .columns(["offset", "digest"].map(Alias::new))
        .expr_as(
            sea_orm::sea_query::Func::cust(Alias::new("length"))
                .arg(Expr::col(Alias::new("bytes"))),
            Alias::new("size"),
        )
        .from(Alias::new("sync_chunks"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("manifest")).eq(requested.manifest_digest.to_vec()))
        .and_where(Expr::col(Alias::new("ordinal")).eq(i64::from(ordinal)))
        .to_owned();
    let row = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing manifest fragment"))?;
    let offset = u64::try_from(row.try_get_by_index::<i64>(0)?)?;
    let digest: Vec<u8> = row.try_get_by_index(1)?;
    let size = u64::try_from(row.try_get_by_index::<i64>(2)?)?;
    ensure!(
        size > 0
            && size <= max_bytes
            && offset
                .checked_add(size)
                .is_some_and(|end| end <= requested.encoded_bytes),
        "fragment does not fit or has invalid extent"
    );
    let q = Query::select()
        .column(Alias::new("bytes"))
        .from(Alias::new("sync_chunks"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("manifest")).eq(requested.manifest_digest.to_vec()))
        .and_where(Expr::col(Alias::new("ordinal")).eq(i64::from(ordinal)))
        .to_owned();
    let bytes: Vec<u8> = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("fragment disappeared"))?
        .try_get_by_index(0)?;
    ensure!(bytes.len() as u64 == size, "fragment size changed");
    let fragment = protocol::Fragment {
        manifest: requested,
        ordinal,
        offset,
        bytes,
        digest: digest
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid fragment digest"))?,
    };
    ensure!(
        protocol::fragment_digest(&fragment)? == fragment.digest,
        "fragment digest mismatch"
    );
    Ok(fragment)
}

/// Page a ready lease's verified normalized generation using its key-bound
/// index. Cursor continuity is checked against the requested range, matching
/// the core source; the receiver separately validates complete coverage.
struct IndexedContextView<'a> {
    namespace: Uuid,
    id: Uuid,
    lease: &'a causal_leases::Lease,
    generation: Uuid,
}

async fn context_page_indexed_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    view: IndexedContextView<'_>,
    range: KeyRange<M>,
    after: protocol::ContextCursor,
    limits: protocol::PageLimits,
) -> Result<protocol::ContextPage<M>> {
    let IndexedContextView {
        namespace,
        id,
        lease,
        generation,
    } = view;
    use protocol::SyncError;
    use sea_orm::sea_query::Order;
    if (after.segment_index == 0 && after.next_lower != range.lower)
        || (after.segment_index > 0 && after.next_lower.is_none())
        || after
            .next_lower
            .zip(range.lower)
            .is_some_and(|(a, b)| a < b)
        || after
            .next_lower
            .zip(range.upper)
            .is_some_and(|(a, b)| a >= b)
    {
        return Err(SyncError::StaleWork.into());
    }
    ensure!(!generation.is_nil(), "invalid snapshot generation");
    let mut next = after.clone();
    let mut segments = Vec::new();
    let mut segment_bytes = 0usize;
    let mut done = false;
    for _ in 0..limits.max_scanned {
        let key = crate::causal_source::snapshot_bound(next.next_lower, true);
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
            .and_where(
                Expr::col(Alias::new("collection"))
                    .eq(M::collection_tag(&range.collection).as_bytes().to_vec()),
            )
            .and_where(Expr::col(Alias::new("basis_generation")).eq(generation.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("state_seq")).eq(lease.state_seq))
            .and_where(Expr::col(Alias::new("lower_bound")).lte(key.clone()))
            .order_by(Alias::new("lower_bound"), Order::Desc)
            .limit(1)
            .to_owned();
        let row = tx
            .query_one(&q)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing snapshot context coverage"))?;
        let lower: Vec<u8> = row.try_get_by_index(0)?;
        let upper: Vec<u8> = row.try_get_by_index(1)?;
        ensure!(
            crate::causal_compaction::valid_bound(&lower)
                && crate::causal_compaction::valid_bound(&upper)
                && lower <= key
                && key < upper,
            "invalid snapshot context coverage"
        );
        let digest: protocol::Digest = row
            .try_get_by_index::<Option<Vec<u8>>>(3)?
            .ok_or_else(|| anyhow::anyhow!("missing staged context digest"))?
            .try_into()
            .map_err(|_| SyncError::StaleWork)?;
        let manifest_digest: protocol::Digest = row
            .try_get_by_index::<Vec<u8>>(2)?
            .try_into()
            .map_err(|_| SyncError::StaleWork)?;
        let manifest = causal_manifests::reference_on(tx, namespace, manifest_digest).await?;
        ensure!(
            manifest.kind == ManifestKind::Context,
            "invalid staged context kind"
        );
        let raw_upper = crate::causal_compaction::decode_bound(&upper, false)?;
        let clipped_upper = match (raw_upper, range.upper) {
            (Some(a), Some(b)) => Some(std::cmp::min(a, b)),
            (a, None) | (None, a) => a,
        };
        let segment = ContextSegment {
            range: KeyRange {
                collection: range.collection.clone(),
                lower: next.next_lower,
                upper: clipped_upper,
            },
            context: ContextRef { digest, manifest },
        };
        let candidate_next = protocol::ContextCursor {
            segment_index: next
                .segment_index
                .checked_add(1)
                .ok_or(SyncError::LogicalLimit)?,
            next_lower: clipped_upper,
        };
        let finishes = clipped_upper == range.upper;
        let item_bytes = serde_json::to_vec(&segment)?.len();
        let trial = protocol::ContextPage::<M> {
            snapshot_id: id,
            range: range.clone(),
            after: after.clone(),
            next: candidate_next.clone(),
            done: finishes,
            segments: vec![],
        };
        let total = serde_json::to_vec(&trial)?
            .len()
            .checked_add(segment_bytes)
            .and_then(|n| n.checked_add(item_bytes))
            .and_then(|n| n.checked_add(segments.len()))
            .ok_or(SyncError::LogicalLimit)?;
        if u64::try_from(total)? > limits.max_bytes {
            if segments.is_empty() {
                return Err(SyncError::LimitTooSmall {
                    required: u64::try_from(total)?,
                }
                .into());
            }
            break;
        }
        segment_bytes += item_bytes;
        segments.push(segment);
        next = candidate_next;
        done = finishes;
        if done {
            break;
        }
    }
    Ok(protocol::ContextPage {
        snapshot_id: id,
        range,
        after,
        next,
        done,
        segments,
    })
}
