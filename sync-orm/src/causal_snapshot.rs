//! Atomic source construction and reads from published source views.
use crate::{causal_candidates, causal_leases, causal_manifests};
use anyhow::{Result, ensure};
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, ExprTrait,
    sea_query::{Alias, Expr, Query},
};
use sync::protocol::{
    self, KeyRange, ManifestKind, Scope, SnapshotDescriptor, StoreIdentity, SyncModel, WorkLimits,
};
use uuid::Uuid;

/// Construct a bounded ready view in one caller transaction.
pub async fn open_bounded_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    source: StoreIdentity,
    scope: Scope<M>,
    now_ms: i64,
    ttl_ms: i64,
    limits: WorkLimits,
) -> Result<SnapshotDescriptor<M>> {
    causal_leases::lock_on(tx, source.namespace).await?;
    let request = Uuid::new_v4();
    let mut resume = None;
    // Each step is bounded; cap the number of steps for this atomic convenience API.
    for _ in 0..limits.max_requests {
        match crate::causal_source::open_on(
            tx,
            source.clone(),
            request,
            scope.clone(),
            crate::causal_source::SnapshotTiming { now_ms, ttl_ms },
            limits,
            resume,
        )
        .await?
        {
            protocol::WorkOutcome::Ready(descriptor) => return Ok(descriptor),
            protocol::WorkOutcome::Incomplete { cursor } => resume = Some(cursor),
        }
    }
    Err(protocol::SyncError::WorkLimit.into())
}

pub async fn descriptor_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    id: Uuid,
    now_ms: i64,
) -> Result<(causal_leases::Lease, SnapshotDescriptor<M>)> {
    let lease = causal_leases::load_on(tx, namespace, id)
        .await?
        .ok_or(protocol::SyncError::SnapshotExpired)?;
    if lease.status != 1
        || lease.expires_at_ms <= now_ms
        || lease.descriptor.is_none()
        || lease.retained_root.is_none()
    {
        return Err(protocol::SyncError::SnapshotExpired.into());
    }
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
    let root = protocol::decode_context_root::<M>(&descriptor.scope, &body)?;
    ensure!(
        root.contexts == descriptor.contexts,
        "snapshot context mismatch"
    );
    let expected: std::collections::BTreeSet<_> = root
        .contexts
        .iter()
        .map(|c| c.context.manifest.manifest_digest.to_vec())
        .collect();
    let edges = Query::select()
        .column(Alias::new("child_manifest"))
        .from(Alias::new("sync_manifest_edges"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("parent_manifest")).eq(root_digest.to_vec()))
        .limit(protocol::MAX_COLLECTIONS as u64 + 1)
        .to_owned();
    let actual = tx
        .query_all(&edges)
        .await?
        .into_iter()
        .map(|row| row.try_get_by_index::<Vec<u8>>(0))
        .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
    ensure!(actual == expected, "snapshot context dependencies mismatch");
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
            "invalid root context count"
        );
        let parents = rows
            .into_iter()
            .map(|row| row.try_get_by_index::<Vec<u8>>(0))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        requested.kind == ManifestKind::Context && parents.contains(&requested_digest)
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
