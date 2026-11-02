//! Atomic publication of complete, validated causal manifests.
use crate::causal_leases::lock_on;
use anyhow::{Result, bail, ensure};
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, ExprTrait,
    sea_query::{Alias, Condition, Expr, Func, Order, Query},
};
use std::collections::BTreeSet;
use sync::protocol::{
    Digest, Fragment, MAX_FRAGMENTS, MAX_LOGICAL_BYTES, MAX_LOGICAL_ITEMS, ManifestKind,
    ManifestRef, content_digest, fragment_digest, manifest_digest,
};
use uuid::Uuid;

fn kind(k: ManifestKind) -> i64 {
    match k {
        ManifestKind::KeyEvent => 0,
        ManifestKind::RangeEvent => 1,
        ManifestKind::Context => 2,
        ManifestKind::ContextPartition => 3,
        ManifestKind::Leaf => 4,
        ManifestKind::ContextRoot => 5,
    }
}

/// Validate a complete logical body before acquiring the writer lock, then
/// publish its manifest, fragments, and dependency edges in the caller's SQL
/// transaction. `validate` must decode the body canonically and return its
/// complete set of referenced child manifests. A child must already exist;
/// this child-before-parent rule keeps the graph acyclic. The caller must roll
/// back its transaction if this returns an error.
pub async fn publish_on<F>(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    manifest: &ManifestRef,
    fragments: &[Fragment],
    max_bytes: u64,
    validate: F,
) -> Result<bool>
where
    F: FnOnce(&[u8]) -> Result<Vec<Digest>>,
{
    ensure!(!namespace.is_nil(), "invalid namespace");
    ensure!(
        max_bytes > 0
            && max_bytes <= MAX_LOGICAL_BYTES
            && manifest.encoded_bytes > 0
            && manifest.encoded_bytes <= max_bytes
            && manifest.item_count <= MAX_LOGICAL_ITEMS
            && manifest.fragment_count > 0
            && manifest.fragment_count <= MAX_FRAGMENTS
            && manifest.fragment_count as usize == fragments.len(),
        "manifest exceeds admission limit"
    );
    ensure!(
        manifest_digest(manifest)? == manifest.manifest_digest,
        "manifest digest mismatch"
    );
    let capacity = usize::try_from(manifest.encoded_bytes)?;
    let mut body = Vec::with_capacity(capacity);
    for (ordinal, fragment) in fragments.iter().enumerate() {
        ensure!(
            fragment.manifest == *manifest
                && fragment.ordinal as usize == ordinal
                && fragment.offset == body.len() as u64
                && !fragment.bytes.is_empty()
                && fragment.bytes.len() as u64 <= sync::protocol::MAX_PAGE_BYTES
                && fragment_digest(fragment)? == fragment.digest,
            "invalid manifest fragment"
        );
        ensure!(
            body.len()
                .checked_add(fragment.bytes.len())
                .is_some_and(|n| n <= capacity),
            "fragment exceeds manifest"
        );
        body.extend_from_slice(&fragment.bytes);
    }
    ensure!(
        body.len() == capacity && content_digest(&body)? == manifest.content_digest,
        "manifest content mismatch"
    );
    let children: BTreeSet<Digest> = validate(&body)?.into_iter().collect();
    ensure!(
        !children.contains(&manifest.manifest_digest),
        "self-referencing manifest"
    );
    lock_on(tx, namespace).await?;
    let ns = namespace.as_bytes().to_vec();
    let digest = manifest.manifest_digest.to_vec();
    let find = Query::select()
        .columns(
            [
                "kind",
                "content_digest",
                "encoded_bytes",
                "item_count",
                "fragment_count",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
        .and_where(Expr::col(Alias::new("digest")).eq(digest.clone()))
        .to_owned();
    if let Some(row) = tx.query_one(&find).await? {
        ensure!(
            row.try_get_by_index::<i64>(0)? == kind(manifest.kind)
                && row.try_get_by_index::<Vec<u8>>(1)? == manifest.content_digest
                && row.try_get_by_index::<i64>(2)? == i64::try_from(manifest.encoded_bytes)?
                && row.try_get_by_index::<i64>(3)? == i64::try_from(manifest.item_count)?
                && row.try_get_by_index::<i64>(4)? == i64::from(manifest.fragment_count),
            "conflicting stored manifest"
        );
        let q = Query::select()
            .columns(["ordinal", "offset", "digest", "bytes"].map(Alias::new))
            .from(Alias::new("sync_chunks"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
            .and_where(Expr::col(Alias::new("manifest")).eq(digest.clone()))
            .order_by(Alias::new("ordinal"), Order::Asc)
            .limit(fragments.len() as u64 + 1)
            .to_owned();
        let stored = tx.query_all(&q).await?;
        ensure!(
            stored.len() == fragments.len(),
            "incomplete stored manifest"
        );
        for (row, f) in stored.iter().zip(fragments) {
            ensure!(
                row.try_get_by_index::<i64>(0)? == i64::from(f.ordinal)
                    && row.try_get_by_index::<i64>(1)? == i64::try_from(f.offset)?
                    && row.try_get_by_index::<Vec<u8>>(2)? == f.digest
                    && row.try_get_by_index::<Vec<u8>>(3)? == f.bytes,
                "conflicting stored fragment"
            );
        }
        let q = Query::select()
            .column(Alias::new("child_manifest"))
            .from(Alias::new("sync_manifest_edges"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
            .and_where(Expr::col(Alias::new("parent_manifest")).eq(digest.clone()))
            .limit(children.len() as u64 + 1)
            .to_owned();
        let stored = tx.query_all(&q).await?;
        let mut actual = BTreeSet::new();
        for row in stored {
            let bytes: Vec<u8> = row.try_get_by_index(0)?;
            let value: Digest = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid stored dependency digest"))?;
            actual.insert(value);
        }
        ensure!(
            actual == children,
            "conflicting stored manifest dependencies"
        );
        return Ok(false);
    }
    for child in &children {
        let q = Query::select()
            .column(Alias::new("kind"))
            .from(Alias::new("sync_manifests"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
            .and_where(Expr::col(Alias::new("digest")).eq(child.to_vec()))
            .limit(1)
            .to_owned();
        let row = tx
            .query_one(&q)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing manifest dependency"))?;
        ensure!(
            (0..=5).contains(&row.try_get_by_index::<i64>(0)?),
            "manifest dependency is private or being reclaimed"
        );
    }
    let q = Query::insert()
        .into_table(Alias::new("sync_manifests"))
        .columns(
            [
                "namespace",
                "digest",
                "kind",
                "content_digest",
                "encoded_bytes",
                "item_count",
                "fragment_count",
                "metadata",
            ]
            .map(Alias::new),
        )
        .values_panic([
            ns.clone().into(),
            digest.clone().into(),
            kind(manifest.kind).into(),
            manifest.content_digest.to_vec().into(),
            i64::try_from(manifest.encoded_bytes)?.into(),
            i64::try_from(manifest.item_count)?.into(),
            i64::from(manifest.fragment_count).into(),
            Vec::<u8>::new().into(),
        ])
        .to_owned();
    tx.execute(&q).await?;
    for fragment in fragments {
        let q = Query::insert()
            .into_table(Alias::new("sync_chunks"))
            .columns(
                [
                    "namespace",
                    "manifest",
                    "ordinal",
                    "offset",
                    "digest",
                    "bytes",
                ]
                .map(Alias::new),
            )
            .values_panic([
                ns.clone().into(),
                digest.clone().into(),
                i64::from(fragment.ordinal).into(),
                i64::try_from(fragment.offset)?.into(),
                fragment.digest.to_vec().into(),
                fragment.bytes.clone().into(),
            ])
            .to_owned();
        tx.execute(&q).await?;
    }
    for child in children {
        let q = Query::insert()
            .into_table(Alias::new("sync_manifest_edges"))
            .columns(["namespace", "parent_manifest", "child_manifest"].map(Alias::new))
            .values_panic([
                ns.clone().into(),
                digest.clone().into(),
                child.to_vec().into(),
            ])
            .to_owned();
        tx.execute(&q).await?;
    }
    Ok(true)
}

/// Encode an already validated logical body into bounded ordered fragments.
pub fn fragment_body(
    kind: ManifestKind,
    body: &[u8],
    item_count: u64,
    fragment_bytes: usize,
) -> Result<(ManifestRef, Vec<Fragment>)> {
    ensure!(
        !body.is_empty()
            && body.len() as u64 <= MAX_LOGICAL_BYTES
            && item_count <= MAX_LOGICAL_ITEMS
            && fragment_bytes > 0
            && fragment_bytes as u64 <= sync::protocol::MAX_PAGE_BYTES,
        "invalid manifest body limits"
    );
    let count = body.len().div_ceil(fragment_bytes);
    ensure!(count <= MAX_FRAGMENTS as usize, "too many fragments");
    let mut manifest = ManifestRef {
        kind,
        manifest_digest: [0; 32],
        content_digest: content_digest(body)?,
        encoded_bytes: body.len() as u64,
        item_count,
        fragment_count: u32::try_from(count)?,
    };
    manifest.manifest_digest = manifest_digest(&manifest)?;
    let canonical_fragment_bytes = body.len().div_ceil(count);
    let mut fragments = Vec::with_capacity(count);
    for (ordinal, chunk) in body.chunks(canonical_fragment_bytes).enumerate() {
        let mut fragment = Fragment {
            manifest: manifest.clone(),
            ordinal: u32::try_from(ordinal)?,
            offset: u64::try_from(
                ordinal
                    .checked_mul(canonical_fragment_bytes)
                    .ok_or_else(|| anyhow::anyhow!("fragment offset overflow"))?,
            )?,
            bytes: chunk.to_vec(),
            digest: [0; 32],
        };
        fragment.digest = fragment_digest(&fragment)?;
        fragments.push(fragment);
    }
    Ok((manifest, fragments))
}

/// Read and validate immutable manifest metadata without loading its body.
pub async fn reference_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    digest: Digest,
) -> Result<ManifestRef> {
    ensure!(!namespace.is_nil(), "invalid namespace");
    let ns = namespace.as_bytes().to_vec();
    let q = Query::select()
        .columns(
            [
                "kind",
                "content_digest",
                "encoded_bytes",
                "item_count",
                "fragment_count",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
        .and_where(Expr::col(Alias::new("digest")).eq(digest.to_vec()))
        .to_owned();
    let row = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing retained manifest"))?;
    let kind = match row.try_get_by_index::<i64>(0)? {
        0 => ManifestKind::KeyEvent,
        1 => ManifestKind::RangeEvent,
        2 => ManifestKind::Context,
        3 => ManifestKind::ContextPartition,
        4 => ManifestKind::Leaf,
        5 => ManifestKind::ContextRoot,
        _ => bail!("invalid manifest kind"),
    };
    let content: Vec<u8> = row.try_get_by_index(1)?;
    let content_digest_value: Digest = content
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid content digest"))?;
    let manifest = ManifestRef {
        kind,
        manifest_digest: digest,
        content_digest: content_digest_value,
        encoded_bytes: u64::try_from(row.try_get_by_index::<i64>(2)?)?,
        item_count: u64::try_from(row.try_get_by_index::<i64>(3)?)?,
        fragment_count: u32::try_from(row.try_get_by_index::<i64>(4)?)?,
    };
    ensure!(
        manifest.encoded_bytes > 0
            && manifest.encoded_bytes <= MAX_LOGICAL_BYTES
            && manifest.item_count <= MAX_LOGICAL_ITEMS
            && manifest.fragment_count > 0
            && manifest.fragment_count <= MAX_FRAGMENTS
            && manifest_digest(&manifest)? == digest,
        "invalid retained manifest metadata"
    );
    Ok(manifest)
}

/// Load and verify one retained manifest using bounded fragment reads. The
/// caller must hold the appropriate namespace lock or durable retention pin.
pub async fn read_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    digest: Digest,
    max_bytes: u64,
    max_fragments: u32,
) -> Result<(ManifestRef, Vec<u8>)> {
    ensure!(
        !namespace.is_nil()
            && max_bytes > 0
            && max_bytes <= MAX_LOGICAL_BYTES
            && max_fragments > 0
            && max_fragments <= MAX_FRAGMENTS,
        "invalid manifest read limit"
    );
    let ns = namespace.as_bytes().to_vec();
    let manifest = reference_on(tx, namespace, digest).await?;
    ensure!(
        manifest.encoded_bytes <= max_bytes && manifest.fragment_count <= max_fragments,
        "manifest read limit exceeded"
    );
    let mut body = Vec::with_capacity(usize::try_from(manifest.encoded_bytes)?);
    if manifest.fragment_count == 1 {
        // A single fragment must span the already validated manifest length.
        // Filter by that exact length before asking the driver for its body.
        ensure!(
            manifest.encoded_bytes <= sync::protocol::MAX_PAGE_BYTES,
            "invalid fragment extent"
        );
        let query = Query::select()
            .columns(["offset", "digest", "bytes"].map(Alias::new))
            .from(Alias::new("sync_chunks"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
            .and_where(Expr::col(Alias::new("manifest")).eq(digest.to_vec()))
            .and_where(Expr::col(Alias::new("ordinal")).eq(0_i64))
            .and_where(
                Expr::expr(Func::cust(Alias::new("length")).arg(Expr::col(Alias::new("digest"))))
                    .eq(32_i64),
            )
            .and_where(
                Expr::expr(Func::cust(Alias::new("length")).arg(Expr::col(Alias::new("bytes"))))
                    .eq(i64::try_from(manifest.encoded_bytes)?),
            )
            .to_owned();
        let row = tx
            .query_one(&query)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing or changed manifest fragment"))?;
        let offset = u64::try_from(row.try_get_by_index::<i64>(0)?)?;
        let stored: Vec<u8> = row.try_get_by_index(1)?;
        let bytes: Vec<u8> = row.try_get_by_index(2)?;
        ensure!(
            offset == 0 && bytes.len() as u64 == manifest.encoded_bytes,
            "invalid fragment extent"
        );
        let fragment = Fragment {
            manifest: manifest.clone(),
            ordinal: 0,
            offset,
            bytes,
            digest: stored
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid fragment digest"))?,
        };
        ensure!(
            fragment_digest(&fragment)? == fragment.digest,
            "fragment digest mismatch"
        );
        body.extend_from_slice(&fragment.bytes);
    } else {
        // Two bound parameters per fragment leave room below conservative
        // database parameter limits. Body batches allocate at most one page.
        const HEADER_BATCH: u32 = 96;
        let mut ordinal = 0_u32;
        while ordinal < manifest.fragment_count {
            let count = std::cmp::min(HEADER_BATCH, manifest.fragment_count - ordinal);
            let query = Query::select()
                .columns(["ordinal", "offset"].map(Alias::new))
                .expr_as(
                    Func::cust(Alias::new("length")).arg(Expr::col(Alias::new("digest"))),
                    Alias::new("digest_size"),
                )
                .expr_as(
                    Func::cust(Alias::new("length")).arg(Expr::col(Alias::new("bytes"))),
                    Alias::new("size"),
                )
                .from(Alias::new("sync_chunks"))
                .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
                .and_where(Expr::col(Alias::new("manifest")).eq(digest.to_vec()))
                .and_where(Expr::col(Alias::new("ordinal")).gte(i64::from(ordinal)))
                .and_where(Expr::col(Alias::new("ordinal")).lt(i64::from(ordinal + count)))
                .order_by(Alias::new("ordinal"), Order::Asc)
                .limit(u64::from(count))
                .to_owned();
            let rows = tx.query_all(&query).await?;
            ensure!(
                rows.len() == usize::try_from(count)?,
                "missing manifest fragment"
            );
            let mut headers = Vec::with_capacity(rows.len());
            let mut offset = body.len() as u64;
            let mut batch_bytes = 0_u64;
            for row in rows {
                let row_ordinal = u32::try_from(row.try_get_by_index::<i64>(0)?)?;
                let row_offset = u64::try_from(row.try_get_by_index::<i64>(1)?)?;
                let digest_size: i64 = row.try_get_by_index(2)?;
                let size = u64::try_from(row.try_get_by_index::<i64>(3)?)?;
                ensure!(
                    row_ordinal == ordinal + u32::try_from(headers.len())?
                        && row_offset == offset
                        && digest_size == 32
                        && size > 0
                        && size <= sync::protocol::MAX_PAGE_BYTES
                        && offset
                            .checked_add(size)
                            .is_some_and(|end| end <= manifest.encoded_bytes),
                    "invalid fragment extent"
                );
                if !headers.is_empty() && batch_bytes + size > sync::protocol::MAX_PAGE_BYTES {
                    break;
                }
                offset += size;
                batch_bytes += size;
                headers.push((row_ordinal, row_offset, size));
            }
            let mut selected = Condition::any();
            for (index, _, size) in &headers {
                selected = selected.add(
                    Condition::all()
                        .add(Expr::col(Alias::new("ordinal")).eq(i64::from(*index)))
                        .add(
                            Expr::expr(
                                Func::cust(Alias::new("length"))
                                    .arg(Expr::col(Alias::new("bytes"))),
                            )
                            .eq(i64::try_from(*size)?),
                        ),
                );
            }
            let query = Query::select()
                .columns(["ordinal", "digest", "bytes"].map(Alias::new))
                .from(Alias::new("sync_chunks"))
                .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
                .and_where(Expr::col(Alias::new("manifest")).eq(digest.to_vec()))
                .and_where(
                    Expr::expr(
                        Func::cust(Alias::new("length")).arg(Expr::col(Alias::new("digest"))),
                    )
                    .eq(32_i64),
                )
                .cond_where(selected)
                .order_by(Alias::new("ordinal"), Order::Asc)
                .to_owned();
            let rows = tx.query_all(&query).await?;
            ensure!(
                rows.len() == headers.len(),
                "missing or changed manifest fragment"
            );
            for (row, (index, offset, size)) in rows.into_iter().zip(headers) {
                let actual = u32::try_from(row.try_get_by_index::<i64>(0)?)?;
                let stored: Vec<u8> = row.try_get_by_index(1)?;
                let bytes: Vec<u8> = row.try_get_by_index(2)?;
                ensure!(
                    actual == index && bytes.len() as u64 == size,
                    "manifest fragment changed"
                );
                let fragment = Fragment {
                    manifest: manifest.clone(),
                    ordinal: index,
                    offset,
                    bytes,
                    digest: stored
                        .try_into()
                        .map_err(|_| anyhow::anyhow!("invalid fragment digest"))?,
                };
                ensure!(
                    fragment_digest(&fragment)? == fragment.digest,
                    "fragment digest mismatch"
                );
                body.extend_from_slice(&fragment.bytes);
                ordinal += 1;
            }
        }
    }
    ensure!(
        body.len() as u64 == manifest.encoded_bytes
            && content_digest(&body)? == manifest.content_digest,
        "manifest content mismatch"
    );
    Ok((manifest, body))
}

/// An ORM-private, content-addressed node rooting resumable metadata work.
/// It is never returned as a protocol manifest or accepted as a source root.
/// The caller must pin the returned digest in its build lease in the same
/// transaction and may only pass children already validated and retained by
/// that lease or by the pinned source basis.
pub(crate) async fn publish_work_node_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    previous: Option<Digest>,
    child: Option<Digest>,
    payload: &[u8],
    max_bytes: u64,
    max_mutations: u64,
) -> Result<Digest> {
    ensure!(
        !namespace.is_nil() && payload.len() <= 4096,
        "invalid work node"
    );
    let mut body = Vec::with_capacity(payload.len() + 70);
    body.push(1);
    for value in [previous, child] {
        match value {
            Some(value) => {
                body.push(1);
                body.extend_from_slice(&value);
            }
            None => body.push(0),
        }
    }
    body.extend_from_slice(&u32::try_from(payload.len())?.to_be_bytes());
    body.extend_from_slice(payload);
    let required_bytes = u64::try_from(body.len())?;
    if required_bytes > max_bytes {
        return Err(sync::protocol::SyncError::LimitTooSmall {
            required: required_bytes,
        }
        .into());
    }
    let children: BTreeSet<_> = [previous, child].into_iter().flatten().collect();
    // The namespace serialization UPDATE is a mutation as well as the
    // manifest and edge inserts.
    let required_mutations = 2 + children.len() as u64;
    if required_mutations > max_mutations {
        return Err(sync::protocol::SyncError::LimitTooSmall {
            required: required_mutations,
        }
        .into());
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"sync.orm.private.work-node.v1\0");
    hasher.update(&body);
    let digest = *hasher.finalize().as_bytes();
    ensure!(!children.contains(&digest), "self-referencing work node");
    lock_on(tx, namespace).await?;
    let ns = namespace.as_bytes().to_vec();
    for child in &children {
        let q = Query::select()
            .column(Alias::new("kind"))
            .from(Alias::new("sync_manifests"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
            .and_where(Expr::col(Alias::new("digest")).eq(child.to_vec()))
            .to_owned();
        let row = tx
            .query_one(&q)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing work node child"))?;
        ensure!(
            (0..=7).contains(&row.try_get_by_index::<i64>(0)?),
            "work node child has an invalid or reclaiming kind"
        );
    }
    let existing = Query::select()
        .columns(["kind", "metadata"].map(Alias::new))
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
        .and_where(Expr::col(Alias::new("digest")).eq(digest.to_vec()))
        .to_owned();
    if let Some(row) = tx.query_one(&existing).await? {
        ensure!(
            row.try_get_by_index::<i64>(0)? == 6 && row.try_get_by_index::<Vec<u8>>(1)? == body,
            "conflicting work node"
        );
        let q = Query::select()
            .column(Alias::new("child_manifest"))
            .from(Alias::new("sync_manifest_edges"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns))
            .and_where(Expr::col(Alias::new("parent_manifest")).eq(digest.to_vec()))
            .limit(3)
            .to_owned();
        let mut stored = BTreeSet::new();
        for row in tx.query_all(&q).await? {
            let value: Vec<u8> = row.try_get_by_index(0)?;
            stored.insert(
                <[u8; 32]>::try_from(value)
                    .map_err(|_| anyhow::anyhow!("invalid work node child"))?,
            );
        }
        ensure!(stored == children, "conflicting work node children");
        return Ok(digest);
    }
    let q = Query::insert()
        .into_table(Alias::new("sync_manifests"))
        .columns(
            [
                "namespace",
                "digest",
                "kind",
                "content_digest",
                "encoded_bytes",
                "item_count",
                "fragment_count",
                "metadata",
            ]
            .map(Alias::new),
        )
        .values_panic([
            ns.clone().into(),
            digest.to_vec().into(),
            6i64.into(),
            blake3::hash(&body).as_bytes().to_vec().into(),
            i64::try_from(required_bytes)?.into(),
            i64::try_from(children.len())?.into(),
            0i64.into(),
            body.into(),
        ])
        .to_owned();
    tx.execute(&q).await?;
    for child in children {
        let q = Query::insert()
            .into_table(Alias::new("sync_manifest_edges"))
            .columns(["namespace", "parent_manifest", "child_manifest"].map(Alias::new))
            .values_panic([
                ns.clone().into(),
                digest.to_vec().into(),
                child.to_vec().into(),
            ])
            .to_owned();
        tx.execute(&q).await?;
    }
    Ok(digest)
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkNode {
    pub previous: Option<Digest>,
    pub child: Option<Digest>,
    pub payload: Vec<u8>,
}

/// Decode only a private work node, with explicit metadata and edge bounds.
pub(crate) async fn read_work_node_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    digest: Digest,
    max_bytes: u64,
    max_rows: u64,
) -> Result<WorkNode> {
    ensure!(
        !namespace.is_nil() && max_rows > 0,
        "invalid work node read"
    );
    let q = Query::select()
        .columns(
            [
                "kind",
                "encoded_bytes",
                "content_digest",
                "item_count",
                "fragment_count",
            ]
            .map(Alias::new),
        )
        .expr_as(
            sea_orm::sea_query::Func::cust(Alias::new("length"))
                .arg(Expr::col(Alias::new("metadata"))),
            Alias::new("metadata_len"),
        )
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("digest")).eq(digest.to_vec()))
        .to_owned();
    let row = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing work node"))?;
    ensure!(
        row.try_get_by_index::<i64>(0)? == 6,
        "not a private work node"
    );
    let size = u64::try_from(row.try_get_by_index::<i64>(1)?)?;
    if size > max_bytes {
        return Err(sync::protocol::SyncError::LimitTooSmall { required: size }.into());
    }
    let content_digest: Vec<u8> = row.try_get_by_index(2)?;
    let item_count: i64 = row.try_get_by_index(3)?;
    let fragment_count: i64 = row.try_get_by_index(4)?;
    let metadata_len = u64::try_from(row.try_get_by_index::<i64>(5)?)?;
    ensure!(
        metadata_len == size && size <= 4167 && (0..=2).contains(&item_count),
        "invalid work node size"
    );
    let required_rows = 2 + item_count as u64;
    if required_rows > max_rows {
        return Err(sync::protocol::SyncError::LimitTooSmall {
            required: required_rows,
        }
        .into());
    }
    let body_query = Query::select()
        .column(Alias::new("metadata"))
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("digest")).eq(digest.to_vec()))
        .to_owned();
    let body: Vec<u8> = tx
        .query_one(&body_query)
        .await?
        .ok_or_else(|| anyhow::anyhow!("work node disappeared"))?
        .try_get_by_index(0)?;
    ensure!(body.len() as u64 == size, "work node changed");
    let mut hash = blake3::Hasher::new();
    hash.update(b"sync.orm.private.work-node.v1\0");
    hash.update(&body);
    ensure!(
        hash.finalize().as_bytes() == &digest
            && content_digest.as_slice() == blake3::hash(&body).as_bytes()
            && fragment_count == 0,
        "invalid work node digest"
    );
    let mut cursor = 0usize;
    let version = *body
        .get(cursor)
        .ok_or_else(|| anyhow::anyhow!("truncated work node"))?;
    cursor += 1;
    ensure!(version == 1, "unknown work node version");
    let mut parse_child = || -> Result<Option<Digest>> {
        let present = *body
            .get(cursor)
            .ok_or_else(|| anyhow::anyhow!("truncated work node"))?;
        cursor += 1;
        match present {
            0 => Ok(None),
            1 => {
                let end = cursor
                    .checked_add(32)
                    .ok_or_else(|| anyhow::anyhow!("work node overflow"))?;
                let bytes = body
                    .get(cursor..end)
                    .ok_or_else(|| anyhow::anyhow!("truncated work node"))?;
                cursor = end;
                Ok(Some(bytes.try_into()?))
            }
            _ => anyhow::bail!("invalid work node child flag"),
        }
    };
    let previous = parse_child()?;
    let child = parse_child()?;
    let end = cursor
        .checked_add(4)
        .ok_or_else(|| anyhow::anyhow!("work node overflow"))?;
    let length = u32::from_be_bytes(
        body.get(cursor..end)
            .ok_or_else(|| anyhow::anyhow!("truncated work node"))?
            .try_into()?,
    ) as usize;
    cursor = end;
    ensure!(
        length <= 4096 && body.len().checked_sub(cursor) == Some(length),
        "invalid work node payload"
    );
    let payload = body[cursor..].to_vec();
    let expected: BTreeSet<_> = [previous, child].into_iter().flatten().collect();
    ensure!(
        item_count == i64::try_from(expected.len())?,
        "invalid work node item count"
    );
    let q = Query::select()
        .column(Alias::new("child_manifest"))
        .from(Alias::new("sync_manifest_edges"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("parent_manifest")).eq(digest.to_vec()))
        .limit(3)
        .to_owned();
    let mut actual = BTreeSet::new();
    for row in tx.query_all(&q).await? {
        let bytes: Vec<u8> = row.try_get_by_index(0)?;
        actual.insert(
            <[u8; 32]>::try_from(bytes).map_err(|_| anyhow::anyhow!("invalid work node child"))?,
        );
    }
    ensure!(actual == expected, "work node dependency mismatch");
    Ok(WorkNode {
        previous,
        child,
        payload,
    })
}

#[cfg(test)]
mod work_node_tests {
    use super::*;
    use sea_orm::{Database, TransactionTrait};
    use serde::{Deserialize, Serialize};
    use sync::protocol::{self, ObjectKey, SyncModel};
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Model;
    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    enum Collection {
        Notes,
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
            vec![Collection::Notes]
        }
        fn collection_tag(_: &Collection) -> &'static str {
            "notes"
        }
        fn collection_from_tag(tag: &str) -> Option<Collection> {
            (tag == "notes").then_some(Collection::Notes)
        }
        fn validate(_: &ObjectKey<Self>, _: &Option<Note>) -> protocol::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn fragment_body_uses_canonical_balanced_bounds() -> Result<()> {
        let (manifest, fragments) = fragment_body(ManifestKind::Context, b"0123456789", 0, 6)?;
        assert_eq!(fragments.len(), 2);
        for fragment in &fragments {
            let (start, end) = protocol::fragment_bounds(&manifest, fragment.ordinal)?;
            assert_eq!(
                (
                    fragment.offset,
                    fragment.offset + fragment.bytes.len() as u64
                ),
                (start, end)
            );
        }
        assert_eq!((fragments[0].bytes.len(), fragments[1].bytes.len()), (5, 5));
        Ok(())
    }
    #[tokio::test]
    async fn reserved_context_is_private_idempotent_and_collected_with_its_chunks() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        crate::causal_schema::install_fresh(&db).await?;
        let ns = Uuid::new_v4();
        crate::causal_store::CausalReplica::<Model>::create_empty(db.clone(), ns, Uuid::new_v4())
            .await?;
        let lease = Uuid::new_v4();
        let body = protocol::canonical_context(&protocol::ExactContext { writers: vec![] })?;
        let (manifest, fragments) = fragment_body(ManifestKind::Context, &body, 0, 1024)?;
        let tx = db.begin().await?;
        crate::causal_leases::create_build_on(
            &tx,
            crate::causal_leases::BuildLeaseRequest {
                namespace: ns,
                id: lease,
                scope: b"scope".to_vec(),
                now_ms: 1,
                expires_at_ms: 100,
                max_active: 4,
                max_ttl_ms: 1000,
            },
        )
        .await?;
        assert_eq!(
            reserve_context_on(&tx, ns, &manifest).await?,
            Reservation::Reserved
        );
        assert_eq!(
            reserve_context_on(&tx, ns, &manifest).await?,
            Reservation::Reserved
        );
        let mark = |kind: i64| {
            Query::update()
                .table(Alias::new("sync_manifests"))
                .value(Alias::new("kind"), kind)
                .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                .and_where(Expr::col(Alias::new("digest")).eq(manifest.manifest_digest.to_vec()))
                .to_owned()
        };
        tx.execute(&mark(8)).await?;
        assert!(reserve_context_on(&tx, ns, &manifest).await.is_err());
        assert!(
            crate::causal_leases::pin_root_on(&tx, ns, lease, manifest.manifest_digest, 2)
                .await
                .is_err()
        );
        assert!(
            publish_work_node_on(
                &tx,
                ns,
                None,
                Some(manifest.manifest_digest),
                b"reclaiming",
                1024,
                8
            )
            .await
            .is_err()
        );
        assert!(
            read_on(&tx, ns, manifest.manifest_digest, 1024, 16)
                .await
                .is_err()
        );
        tx.execute(&mark(RESERVED_CONTEXT_KIND)).await?;
        reserve_context_fragment_on(&tx, ns, &fragments[0]).await?;
        reserve_context_fragment_on(&tx, ns, &fragments[0]).await?;
        assert!(
            read_reserved_fragment_header_on(&tx, ns, &manifest, 0, 63)
                .await
                .is_err()
        );
        let header = read_reserved_fragment_header_on(&tx, ns, &manifest, 0, 64).await?;
        let header: ReservedFragmentHeader = serde_json::from_slice(&serde_json::to_vec(&header)?)?;
        assert!(
            read_reserved_fragment_body_on(&tx, ns, &manifest, &header, 64)
                .await
                .is_err()
        );
        let fetched = read_reserved_fragment_body_on(&tx, ns, &manifest, &header, 1024).await?;
        assert_eq!(fetched, fragments[0]);
        assert!(
            read_on(&tx, ns, manifest.manifest_digest, 1024, 16)
                .await
                .is_err()
        );
        assert!(
            publish_on(&tx, ns, &manifest, &fragments, 1024, |_| Ok(vec![]))
                .await
                .is_err()
        );
        let limits = protocol::WorkLimits {
            max_requests: 1,
            max_bytes: 1024,
            max_depth: 1,
            max_rows_scanned: 1,
            max_mutations: 8,
        };
        let root = advance_work_node_on(
            &tx,
            crate::causal_manifests::WorkNodeUpdate {
                namespace: ns,
                lease_id: lease,
                previous: None,
                child: Some(manifest.manifest_digest),
                payload: b"cursor",
                expected_state: None,
                next_state: b"state".to_vec(),
                now_ms: 2,
                limits,
            },
        )
        .await?;
        assert!(
            !crate::causal_gc::collect_one_on(&tx, ns, manifest.manifest_digest, 16, 1024).await?
        );
        crate::causal_leases::revoke_on(&tx, ns, lease).await?;
        assert!(crate::causal_gc::collect_one_on(&tx, ns, root, 16, 1024).await?);
        assert!(
            crate::causal_gc::collect_one_on(&tx, ns, manifest.manifest_digest, 16, 1024).await?
        );
        let q = Query::select()
            .column(Alias::new("ordinal"))
            .from(Alias::new("sync_chunks"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("manifest")).eq(manifest.manifest_digest.to_vec()))
            .to_owned();
        assert!(tx.query_one(&q).await?.is_none());
        publish_on(&tx, ns, &manifest, &fragments, 1024, |_| Ok(vec![])).await?;
        let range = protocol::KeyRange::<Model> {
            collection: Collection::Notes,
            lower: None,
            upper: None,
        };
        let context = protocol::ContextRef {
            digest: protocol::context_digest(&protocol::ExactContext { writers: vec![] })?,
            manifest: manifest.clone(),
        };
        let body = protocol::canonical_context_partition(
            &range,
            &[protocol::ContextSegment {
                range: range.clone(),
                context,
            }],
        )?;
        let (partition, _) = fragment_body(ManifestKind::ContextPartition, &body, 1, 1024)?;
        assert_eq!(
            reserve_manifest_on(&tx, ns, &partition).await?,
            Reservation::Reserved
        );
        reserve_manifest_edge_on(&tx, ns, &partition, manifest.manifest_digest).await?;
        reserve_manifest_edge_on(&tx, ns, &partition, manifest.manifest_digest).await?;
        assert!(
            !crate::causal_gc::collect_one_on(&tx, ns, manifest.manifest_digest, 16, 1024).await?
        );
        assert!(
            crate::causal_gc::collect_one_on(&tx, ns, partition.manifest_digest, 16, 1024).await?
        );
        assert!(
            crate::causal_gc::collect_one_on(&tx, ns, manifest.manifest_digest, 16, 1024).await?
        );
        tx.commit().await?;
        Ok(())
    }
    #[tokio::test]
    async fn private_work_node_is_rooted_rejected_by_protocol_and_collectible() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        crate::causal_schema::install_fresh(&db).await?;
        let ns = Uuid::new_v4();
        let node = Uuid::new_v4();
        crate::causal_store::CausalReplica::<Model>::create_empty(db.clone(), ns, node).await?;
        let lease = Uuid::new_v4();
        let tx = db.begin().await?;
        crate::causal_leases::create_build_on(
            &tx,
            crate::causal_leases::BuildLeaseRequest {
                namespace: ns,
                id: lease,
                scope: b"scope".to_vec(),
                now_ms: 1,
                expires_at_ms: 100,
                max_active: 4,
                max_ttl_ms: 1000,
            },
        )
        .await?;
        let limits = protocol::WorkLimits {
            max_requests: 1,
            max_bytes: 1024,
            max_depth: 1,
            max_rows_scanned: 1,
            max_mutations: 8,
        };
        assert!(
            publish_work_node_on(&tx, ns, None, Some([99; 32]), b"missing", 1024, 8)
                .await
                .is_err()
        );
        let digest = advance_work_node_on(
            &tx,
            crate::causal_manifests::WorkNodeUpdate {
                namespace: ns,
                lease_id: lease,
                previous: None,
                child: None,
                payload: b"cursor",
                expected_state: None,
                next_state: b"state".to_vec(),
                now_ms: 2,
                limits,
            },
        )
        .await?;
        let decoded = read_work_node_on(&tx, ns, digest, 1024, 2).await?;
        let header = work_node_header_on(&tx, ns, digest, 1024).await?;
        let mut edges = work_node_body_on(&tx, ns, &header, 1024).await?;
        edges = serde_json::from_slice(&serde_json::to_vec(&edges)?)?;
        assert_eq!(
            work_node_edge_step_on(&tx, ns, &mut edges, 32).await?,
            Some(decoded.clone())
        );
        assert_eq!(
            decoded,
            WorkNode {
                previous: None,
                child: None,
                payload: b"cursor".to_vec()
            }
        );
        assert!(read_on(&tx, ns, digest, 1024, 16).await.is_err());
        assert!(!crate::causal_gc::collect_one_on(&tx, ns, digest, 16, 1024).await?);
        tx.commit().await?;
        let tx = db.begin().await?;
        assert!(
            advance_work_node_on(
                &tx,
                crate::causal_manifests::WorkNodeUpdate {
                    namespace: ns,
                    lease_id: lease,
                    previous: Some(digest),
                    child: None,
                    payload: b"next",
                    expected_state: Some(b"wrong"),
                    next_state: b"new".to_vec(),
                    now_ms: 3,
                    limits
                }
            )
            .await
            .is_err()
        );
        tx.rollback().await?;
        let tx = db.begin().await?;
        let current = crate::causal_leases::building_on(&tx, ns, lease, 3).await?;
        assert_eq!(current.retained_root, Some(digest.to_vec()));
        assert_eq!(current.build_state, Some(b"state".to_vec()));
        crate::causal_leases::revoke_on(&tx, ns, lease).await?;
        assert!(crate::causal_gc::collect_one_on(&tx, ns, digest, 16, 1024).await?);
        tx.commit().await?;
        Ok(())
    }
}

/// Publish a private work node, advance its lease root, and checkpoint progress
/// together. The caller commits or rolls back the surrounding transaction.
pub(crate) struct WorkNodeUpdate<'a> {
    pub namespace: Uuid,
    pub lease_id: Uuid,
    pub previous: Option<Digest>,
    pub child: Option<Digest>,
    pub payload: &'a [u8],
    pub expected_state: Option<&'a [u8]>,
    pub next_state: Vec<u8>,
    pub now_ms: i64,
    pub limits: sync::protocol::WorkLimits,
}

pub(crate) async fn advance_work_node_on(
    tx: &DatabaseTransaction,
    update: WorkNodeUpdate<'_>,
) -> Result<Digest> {
    let WorkNodeUpdate {
        namespace,
        lease_id,
        previous,
        child,
        payload,
        expected_state,
        next_state,
        now_ms,
        limits,
    } = update;
    // Four namespace-lock UPDATEs, two lease UPDATEs, the node INSERT,
    // and one edge INSERT per distinct child.
    let required_mutations = 7 + [previous, child]
        .into_iter()
        .flatten()
        .collect::<BTreeSet<_>>()
        .len() as u64;
    if limits.max_mutations < required_mutations {
        return Err(sync::protocol::SyncError::LimitTooSmall {
            required: required_mutations,
        }
        .into());
    }
    let required_bytes = u64::try_from(payload.len())?
        .checked_add(u64::try_from(next_state.len())?)
        .and_then(|n| n.checked_add(70))
        .ok_or_else(|| anyhow::anyhow!("work byte overflow"))?;
    if limits.max_bytes < required_bytes {
        return Err(sync::protocol::SyncError::LimitTooSmall {
            required: required_bytes,
        }
        .into());
    }
    let lease = crate::causal_leases::building_on(tx, namespace, lease_id, now_ms).await?;
    ensure!(
        lease.retained_root.as_deref() == previous.as_ref().map(|v| v.as_slice()),
        "stale work root"
    );
    if let Some(child) = child {
        let q = Query::select()
            .column(Alias::new("kind"))
            .from(Alias::new("sync_manifests"))
            .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("digest")).eq(child.to_vec()))
            .to_owned();
        let row = tx
            .query_one(&q)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing completed work child"))?;
        ensure!(
            (0..=5).contains(&row.try_get_by_index::<i64>(0)?)
                || row.try_get_by_index::<i64>(0)? == RESERVED_CONTEXT_KIND,
            "invalid completed work child"
        );
    }
    let digest = publish_work_node_on(
        tx,
        namespace,
        previous,
        child,
        payload,
        limits.max_bytes,
        limits.max_mutations - 2,
    )
    .await?;
    crate::causal_leases::pin_root_on(tx, namespace, lease_id, digest, now_ms).await?;
    crate::causal_leases::checkpoint_on(
        tx,
        namespace,
        lease_id,
        expected_state,
        next_state,
        now_ms,
    )
    .await?;
    Ok(digest)
}

/// Private reservation for a not-yet-complete Context manifest. It is keyed by
/// the eventual protocol digest but has a dedicated non-protocol kind. A
/// caller must root it from an active build in the same transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reservation {
    Reserved,
    Published,
}

const RESERVED_CONTEXT_KIND: i64 = 7;

pub(crate) async fn reserve_context_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    manifest: &ManifestRef,
) -> Result<Reservation> {
    ensure!(
        manifest.kind == ManifestKind::Context,
        "invalid context kind"
    );
    reserve_manifest_on(tx, namespace, manifest).await
}

pub(crate) async fn reserve_manifest_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    manifest: &ManifestRef,
) -> Result<Reservation> {
    ensure!(
        !namespace.is_nil()
            && matches!(
                manifest.kind,
                ManifestKind::Context | ManifestKind::ContextPartition | ManifestKind::ContextRoot
            )
            && manifest.encoded_bytes > 0
            && manifest.encoded_bytes <= MAX_LOGICAL_BYTES
            && manifest.item_count <= MAX_LOGICAL_ITEMS
            && manifest.fragment_count > 0
            && manifest.fragment_count <= MAX_FRAGMENTS
            && manifest_digest(manifest)? == manifest.manifest_digest,
        "invalid context reservation"
    );
    lock_on(tx, namespace).await?;
    let ns = namespace.as_bytes().to_vec();
    let expected = serde_json::to_vec(manifest)?;
    let q = Query::select()
        .columns(
            [
                "kind",
                "content_digest",
                "encoded_bytes",
                "item_count",
                "fragment_count",
                "metadata",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
        .and_where(Expr::col(Alias::new("digest")).eq(manifest.manifest_digest.to_vec()))
        .to_owned();
    if let Some(row) = tx.query_one(&q).await? {
        let stored_kind: i64 = row.try_get_by_index(0)?;
        ensure!(
            row.try_get_by_index::<Vec<u8>>(1)? == manifest.content_digest
                && row.try_get_by_index::<i64>(2)? == i64::try_from(manifest.encoded_bytes)?
                && row.try_get_by_index::<i64>(3)? == i64::try_from(manifest.item_count)?
                && row.try_get_by_index::<i64>(4)? == i64::from(manifest.fragment_count),
            "conflicting reserved context"
        );
        if stored_kind == RESERVED_CONTEXT_KIND {
            ensure!(
                row.try_get_by_index::<Vec<u8>>(5)? == expected,
                "conflicting reservation metadata"
            );
            return Ok(Reservation::Reserved);
        }
        ensure!(
            stored_kind == kind(manifest.kind),
            "conflicting manifest kind"
        );
        return Ok(Reservation::Published);
    }
    let q = Query::insert()
        .into_table(Alias::new("sync_manifests"))
        .columns(
            [
                "namespace",
                "digest",
                "kind",
                "content_digest",
                "encoded_bytes",
                "item_count",
                "fragment_count",
                "metadata",
            ]
            .map(Alias::new),
        )
        .values_panic([
            ns.into(),
            manifest.manifest_digest.to_vec().into(),
            RESERVED_CONTEXT_KIND.into(),
            manifest.content_digest.to_vec().into(),
            i64::try_from(manifest.encoded_bytes)?.into(),
            i64::try_from(manifest.item_count)?.into(),
            i64::from(manifest.fragment_count).into(),
            expected.into(),
        ])
        .to_owned();
    tx.execute(&q).await?;
    Ok(Reservation::Reserved)
}

/// Insert one validated fragment into a private reservation. Replays must
/// match the complete existing row; conflicting incoming bytes are rejected.
pub(crate) async fn reserve_context_fragment_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    fragment: &Fragment,
) -> Result<()> {
    ensure!(
        fragment.manifest.kind == ManifestKind::Context,
        "invalid context kind"
    );
    reserve_manifest_fragment_on(tx, namespace, fragment).await
}

pub(crate) async fn reserve_manifest_fragment_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    fragment: &Fragment,
) -> Result<()> {
    let manifest = &fragment.manifest;
    ensure!(
        fragment.ordinal < manifest.fragment_count
            && !fragment.bytes.is_empty()
            && fragment.bytes.len() as u64 <= sync::protocol::MAX_PAGE_BYTES
            && fragment
                .offset
                .checked_add(fragment.bytes.len() as u64)
                .is_some_and(|n| n <= manifest.encoded_bytes)
            && fragment_digest(fragment)? == fragment.digest,
        "invalid reserved fragment"
    );
    lock_on(tx, namespace).await?;
    let ns = namespace.as_bytes().to_vec();
    let exists = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
        .and_where(Expr::col(Alias::new("digest")).eq(manifest.manifest_digest.to_vec()))
        .to_owned();
    ensure!(
        tx.query_one(&exists).await?.is_some(),
        "context reservation is missing"
    );
    let reservation = reserve_manifest_on(tx, namespace, manifest).await?;
    let q = Query::select()
        .columns(["offset", "digest", "bytes"].map(Alias::new))
        .from(Alias::new("sync_chunks"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
        .and_where(Expr::col(Alias::new("manifest")).eq(manifest.manifest_digest.to_vec()))
        .and_where(Expr::col(Alias::new("ordinal")).eq(i64::from(fragment.ordinal)))
        .and_where(
            sea_orm::sea_query::Func::cust(Alias::new("length"))
                .arg(Expr::col(Alias::new("bytes")))
                .lte(i64::try_from(sync::protocol::MAX_PAGE_BYTES)?),
        )
        .to_owned();
    if let Some(row) = tx.query_one(&q).await? {
        ensure!(
            row.try_get_by_index::<i64>(0)? == i64::try_from(fragment.offset)?
                && row.try_get_by_index::<Vec<u8>>(1)? == fragment.digest
                && row.try_get_by_index::<Vec<u8>>(2)? == fragment.bytes,
            "conflicting reserved fragment"
        );
        return Ok(());
    }
    ensure!(
        reservation == Reservation::Reserved,
        "published context fragment is missing"
    );
    let q = Query::insert()
        .into_table(Alias::new("sync_chunks"))
        .columns(
            [
                "namespace",
                "manifest",
                "ordinal",
                "offset",
                "digest",
                "bytes",
            ]
            .map(Alias::new),
        )
        .values_panic([
            ns.into(),
            manifest.manifest_digest.to_vec().into(),
            i64::from(fragment.ordinal).into(),
            i64::try_from(fragment.offset)?.into(),
            fragment.digest.to_vec().into(),
            fragment.bytes.clone().into(),
        ])
        .to_owned();
    tx.execute(&q).await?;
    Ok(())
}

/// Promote only a reservation verified by the context builder against the
/// active lease and its exact retained root. The sealed proof is constructed
/// after bounded fragment rereads and canonical/hash validation.
pub(crate) async fn promote_context_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    proof: &crate::causal_context_builder::ContextCompletionProof,
    now_ms: i64,
) -> Result<sync::protocol::ContextRef> {
    ensure!(
        proof.namespace() == namespace,
        "context proof namespace mismatch"
    );
    let manifest = proof.manifest();
    let lease = crate::causal_leases::building_on(tx, namespace, proof.lease(), now_ms).await?;
    ensure!(
        lease.retained_root.as_deref() == Some(proof.retained_root().as_slice()),
        "context proof root changed"
    );
    let ns = namespace.as_bytes().to_vec();
    let q = Query::select()
        .columns(
            [
                "kind",
                "content_digest",
                "encoded_bytes",
                "item_count",
                "fragment_count",
                "metadata",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
        .and_where(Expr::col(Alias::new("digest")).eq(manifest.manifest_digest.to_vec()))
        .to_owned();
    let row = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("context reservation disappeared"))?;
    let stored_kind: i64 = row.try_get_by_index(0)?;
    ensure!(
        manifest.kind == ManifestKind::Context
            && manifest_digest(manifest)? == manifest.manifest_digest
            && row.try_get_by_index::<Vec<u8>>(1)? == manifest.content_digest
            && row.try_get_by_index::<i64>(2)? == i64::try_from(manifest.encoded_bytes)?
            && row.try_get_by_index::<i64>(3)? == i64::try_from(manifest.item_count)?
            && row.try_get_by_index::<i64>(4)? == i64::from(manifest.fragment_count),
        "context proof metadata mismatch"
    );
    if stored_kind == RESERVED_CONTEXT_KIND {
        ensure!(
            row.try_get_by_index::<Vec<u8>>(5)? == serde_json::to_vec(manifest)?,
            "context reservation metadata mismatch"
        );
        let q = Query::update()
            .table(Alias::new("sync_manifests"))
            .value(Alias::new("kind"), kind(ManifestKind::Context))
            .value(Alias::new("metadata"), Vec::<u8>::new())
            .and_where(Expr::col(Alias::new("namespace")).eq(ns))
            .and_where(Expr::col(Alias::new("digest")).eq(manifest.manifest_digest.to_vec()))
            .and_where(Expr::col(Alias::new("kind")).eq(RESERVED_CONTEXT_KIND))
            .to_owned();
        ensure!(
            tx.execute(&q).await?.rows_affected() == 1,
            "context promotion race"
        );
    } else {
        ensure!(
            stored_kind == kind(ManifestKind::Context),
            "context proof kind mismatch"
        );
    }
    Ok(sync::protocol::ContextRef {
        digest: proof.context_digest(),
        manifest: manifest.clone(),
    })
}

pub(crate) async fn promote_partition_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    proof: &crate::causal_source::PartitionCompletionProof,
    now_ms: i64,
) -> Result<ManifestRef> {
    ensure!(
        proof.namespace() == namespace,
        "context proof namespace mismatch"
    );
    let manifest = proof.manifest();
    let lease = crate::causal_leases::building_on(tx, namespace, proof.lease(), now_ms).await?;
    ensure!(
        lease.retained_root.as_deref() == Some(proof.root().as_slice()),
        "context proof root changed"
    );
    let ns = namespace.as_bytes().to_vec();
    let q = Query::select()
        .columns(
            [
                "kind",
                "content_digest",
                "encoded_bytes",
                "item_count",
                "fragment_count",
                "metadata",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
        .and_where(Expr::col(Alias::new("digest")).eq(manifest.manifest_digest.to_vec()))
        .to_owned();
    let row = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("context reservation disappeared"))?;
    let stored_kind: i64 = row.try_get_by_index(0)?;
    ensure!(
        manifest.kind == ManifestKind::ContextPartition
            && manifest_digest(manifest)? == manifest.manifest_digest
            && row.try_get_by_index::<Vec<u8>>(1)? == manifest.content_digest
            && row.try_get_by_index::<i64>(2)? == i64::try_from(manifest.encoded_bytes)?
            && row.try_get_by_index::<i64>(3)? == i64::try_from(manifest.item_count)?
            && row.try_get_by_index::<i64>(4)? == i64::from(manifest.fragment_count),
        "context proof metadata mismatch"
    );
    if stored_kind == RESERVED_CONTEXT_KIND {
        ensure!(
            row.try_get_by_index::<Vec<u8>>(5)? == serde_json::to_vec(manifest)?,
            "context reservation metadata mismatch"
        );
        let q = Query::update()
            .table(Alias::new("sync_manifests"))
            .value(Alias::new("kind"), kind(ManifestKind::ContextPartition))
            .value(Alias::new("metadata"), Vec::<u8>::new())
            .and_where(Expr::col(Alias::new("namespace")).eq(ns))
            .and_where(Expr::col(Alias::new("digest")).eq(manifest.manifest_digest.to_vec()))
            .and_where(Expr::col(Alias::new("kind")).eq(RESERVED_CONTEXT_KIND))
            .to_owned();
        ensure!(
            tx.execute(&q).await?.rows_affected() == 1,
            "context promotion race"
        );
    } else {
        ensure!(
            stored_kind == kind(ManifestKind::ContextPartition),
            "context proof kind mismatch"
        );
    }
    Ok(manifest.clone())
}

pub(crate) async fn promote_root_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    proof: &crate::causal_source::RootCompletionProof,
    now_ms: i64,
) -> Result<ManifestRef> {
    ensure!(
        proof.namespace() == namespace,
        "context proof namespace mismatch"
    );
    let manifest = proof.manifest();
    let lease = crate::causal_leases::building_on(tx, namespace, proof.lease(), now_ms).await?;
    ensure!(
        lease.retained_root.as_deref() == Some(proof.root().as_slice()),
        "context proof root changed"
    );
    let ns = namespace.as_bytes().to_vec();
    let q = Query::select()
        .columns(
            [
                "kind",
                "content_digest",
                "encoded_bytes",
                "item_count",
                "fragment_count",
                "metadata",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
        .and_where(Expr::col(Alias::new("digest")).eq(manifest.manifest_digest.to_vec()))
        .to_owned();
    let row = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("context reservation disappeared"))?;
    let stored_kind: i64 = row.try_get_by_index(0)?;
    ensure!(
        manifest.kind == ManifestKind::ContextRoot
            && manifest_digest(manifest)? == manifest.manifest_digest
            && row.try_get_by_index::<Vec<u8>>(1)? == manifest.content_digest
            && row.try_get_by_index::<i64>(2)? == i64::try_from(manifest.encoded_bytes)?
            && row.try_get_by_index::<i64>(3)? == i64::try_from(manifest.item_count)?
            && row.try_get_by_index::<i64>(4)? == i64::from(manifest.fragment_count),
        "context proof metadata mismatch"
    );
    if stored_kind == RESERVED_CONTEXT_KIND {
        ensure!(
            row.try_get_by_index::<Vec<u8>>(5)? == serde_json::to_vec(manifest)?,
            "context reservation metadata mismatch"
        );
        let q = Query::update()
            .table(Alias::new("sync_manifests"))
            .value(Alias::new("kind"), kind(ManifestKind::ContextRoot))
            .value(Alias::new("metadata"), Vec::<u8>::new())
            .and_where(Expr::col(Alias::new("namespace")).eq(ns))
            .and_where(Expr::col(Alias::new("digest")).eq(manifest.manifest_digest.to_vec()))
            .and_where(Expr::col(Alias::new("kind")).eq(RESERVED_CONTEXT_KIND))
            .to_owned();
        ensure!(
            tx.execute(&q).await?.rows_affected() == 1,
            "context promotion race"
        );
    } else {
        ensure!(
            stored_kind == kind(ManifestKind::ContextRoot),
            "context proof kind mismatch"
        );
    }
    Ok(manifest.clone())
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReservedFragmentHeader {
    ordinal: u32,
    offset: u64,
    digest: Digest,
    length: u64,
}

/// Inspect one fragment header before reading its body, permitting callers to
/// enforce both one-row work budgets and byte limits across restart.
pub(crate) async fn read_reserved_fragment_header_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    manifest: &ManifestRef,
    ordinal: u32,
    max_bytes: u64,
) -> Result<ReservedFragmentHeader> {
    const HEADER_BYTES: u64 = 64;
    if max_bytes < HEADER_BYTES {
        return Err(sync::protocol::SyncError::LimitTooSmall {
            required: HEADER_BYTES,
        }
        .into());
    }
    ensure!(
        ordinal < manifest.fragment_count
            && matches!(
                manifest.kind,
                ManifestKind::Context | ManifestKind::ContextPartition | ManifestKind::ContextRoot
            )
            && manifest_digest(manifest)? == manifest.manifest_digest,
        "invalid fragment request"
    );
    let ns = namespace.as_bytes().to_vec();
    let q = Query::select()
        .columns(
            [
                "kind",
                "content_digest",
                "encoded_bytes",
                "item_count",
                "fragment_count",
                "metadata",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
        .and_where(Expr::col(Alias::new("digest")).eq(manifest.manifest_digest.to_vec()))
        .to_owned();
    let row = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("context reservation missing"))?;
    let stored_kind: i64 = row.try_get_by_index(0)?;
    ensure!(
        (stored_kind == RESERVED_CONTEXT_KIND || stored_kind == kind(manifest.kind))
            && row.try_get_by_index::<Vec<u8>>(1)? == manifest.content_digest
            && row.try_get_by_index::<i64>(2)? == i64::try_from(manifest.encoded_bytes)?
            && row.try_get_by_index::<i64>(3)? == i64::try_from(manifest.item_count)?
            && row.try_get_by_index::<i64>(4)? == i64::from(manifest.fragment_count),
        "reserved context metadata mismatch"
    );
    if stored_kind == RESERVED_CONTEXT_KIND {
        ensure!(
            row.try_get_by_index::<Vec<u8>>(5)? == serde_json::to_vec(manifest)?,
            "reserved context metadata mismatch"
        );
    }
    let q = Query::select()
        .columns(["offset", "digest"].map(Alias::new))
        .expr_as(
            sea_orm::sea_query::Func::cust(Alias::new("length"))
                .arg(Expr::col(Alias::new("bytes"))),
            Alias::new("length"),
        )
        .from(Alias::new("sync_chunks"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns))
        .and_where(Expr::col(Alias::new("manifest")).eq(manifest.manifest_digest.to_vec()))
        .and_where(Expr::col(Alias::new("ordinal")).eq(i64::from(ordinal)))
        .to_owned();
    let row = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("reserved fragment missing"))?;
    let offset = u64::try_from(row.try_get_by_index::<i64>(0)?)?;
    let digest = row
        .try_get_by_index::<Vec<u8>>(1)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid fragment digest"))?;
    let length = u64::try_from(row.try_get_by_index::<i64>(2)?)?;
    ensure!(
        length > 0
            && length <= sync::protocol::MAX_PAGE_BYTES
            && offset
                .checked_add(length)
                .is_some_and(|n| n <= manifest.encoded_bytes),
        "invalid reserved fragment header"
    );
    Ok(ReservedFragmentHeader {
        ordinal,
        offset,
        digest,
        length,
    })
}

pub(crate) async fn read_reserved_fragment_body_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    manifest: &ManifestRef,
    header: &ReservedFragmentHeader,
    max_bytes: u64,
) -> Result<Fragment> {
    let required = header
        .length
        .checked_add(64)
        .ok_or(sync::protocol::SyncError::LogicalLimit)?;
    if max_bytes < required {
        return Err(sync::protocol::SyncError::LimitTooSmall { required }.into());
    }
    ensure!(
        header.ordinal < manifest.fragment_count
            && header.length > 0
            && header.length <= sync::protocol::MAX_PAGE_BYTES
            && header
                .offset
                .checked_add(header.length)
                .is_some_and(|n| n <= manifest.encoded_bytes),
        "invalid saved fragment header"
    );
    let q = Query::select()
        .column(Alias::new("bytes"))
        .from(Alias::new("sync_chunks"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("manifest")).eq(manifest.manifest_digest.to_vec()))
        .and_where(Expr::col(Alias::new("ordinal")).eq(i64::from(header.ordinal)))
        .and_where(
            sea_orm::sea_query::Func::cust(Alias::new("length"))
                .arg(Expr::col(Alias::new("bytes")))
                .eq(i64::try_from(header.length)?),
        )
        .to_owned();
    let row = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("reserved fragment missing"))?;
    let bytes: Vec<u8> = row.try_get_by_index(0)?;
    ensure!(
        bytes.len() as u64 == header.length,
        "reserved fragment size changed"
    );
    let fragment = Fragment {
        manifest: manifest.clone(),
        ordinal: header.ordinal,
        offset: header.offset,
        digest: header.digest,
        bytes,
    };
    ensure!(
        fragment_digest(&fragment)? == header.digest,
        "reserved fragment digest changed"
    );
    Ok(fragment)
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkNodeHeader {
    digest: Digest,
    content_digest: Digest,
    size: u64,
    children: u8,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkNodeEdges {
    digest: Digest,
    node: WorkNode,
    expected: Vec<Digest>,
    next: usize,
    after: Option<Digest>,
}

pub(crate) async fn work_node_header_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    digest: Digest,
    max_bytes: u64,
) -> Result<WorkNodeHeader> {
    const HEADER_BYTES: u64 = 96;
    if max_bytes < HEADER_BYTES {
        return Err(sync::protocol::SyncError::LimitTooSmall {
            required: HEADER_BYTES,
        }
        .into());
    }
    let q = Query::select()
        .columns(
            [
                "kind",
                "encoded_bytes",
                "content_digest",
                "item_count",
                "fragment_count",
            ]
            .map(Alias::new),
        )
        .expr_as(
            sea_orm::sea_query::Func::cust(Alias::new("length"))
                .arg(Expr::col(Alias::new("metadata"))),
            Alias::new("metadata_len"),
        )
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("digest")).eq(digest.to_vec()))
        .to_owned();
    let row = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing work node"))?;
    let size = u64::try_from(row.try_get_by_index::<i64>(1)?)?;
    let children = u8::try_from(row.try_get_by_index::<i64>(3)?)?;
    ensure!(
        row.try_get_by_index::<i64>(0)? == 6
            && row.try_get_by_index::<i64>(4)? == 0
            && size <= 4167
            && row.try_get_by_index::<i64>(5)? == i64::try_from(size)?
            && children <= 2,
        "invalid private work header"
    );
    let content_digest = row
        .try_get_by_index::<Vec<u8>>(2)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid content digest"))?;
    Ok(WorkNodeHeader {
        digest,
        content_digest,
        size,
        children,
    })
}

pub(crate) async fn work_node_body_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    header: &WorkNodeHeader,
    max_bytes: u64,
) -> Result<WorkNodeEdges> {
    let required = header
        .size
        .checked_add(64)
        .ok_or(sync::protocol::SyncError::LogicalLimit)?;
    if max_bytes < required {
        return Err(sync::protocol::SyncError::LimitTooSmall { required }.into());
    }
    ensure!(
        header.size <= 4167 && header.children <= 2,
        "invalid saved work header"
    );
    let q = Query::select()
        .column(Alias::new("metadata"))
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("digest")).eq(header.digest.to_vec()))
        .and_where(
            sea_orm::sea_query::Func::cust(Alias::new("length"))
                .arg(Expr::col(Alias::new("metadata")))
                .eq(i64::try_from(header.size)?),
        )
        .to_owned();
    let body: Vec<u8> = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("work node changed"))?
        .try_get_by_index(0)?;
    let mut hash = blake3::Hasher::new();
    hash.update(b"sync.orm.private.work-node.v1\0");
    hash.update(&body);
    ensure!(
        hash.finalize().as_bytes() == &header.digest
            && blake3::hash(&body).as_bytes() == &header.content_digest,
        "invalid work node digest"
    );
    let mut cursor = 0usize;
    ensure!(body.get(cursor) == Some(&1), "unknown work node version");
    cursor += 1;
    let mut read_child = || -> Result<Option<Digest>> {
        let flag = *body
            .get(cursor)
            .ok_or_else(|| anyhow::anyhow!("truncated work node"))?;
        cursor += 1;
        match flag {
            0 => Ok(None),
            1 => {
                let end = cursor
                    .checked_add(32)
                    .ok_or(sync::protocol::SyncError::LogicalLimit)?;
                let child = body
                    .get(cursor..end)
                    .ok_or_else(|| anyhow::anyhow!("truncated work node"))?
                    .try_into()?;
                cursor = end;
                Ok(Some(child))
            }
            _ => anyhow::bail!("invalid work node child flag"),
        }
    };
    let previous = read_child()?;
    let child = read_child()?;
    let end = cursor
        .checked_add(4)
        .ok_or(sync::protocol::SyncError::LogicalLimit)?;
    let length = u32::from_be_bytes(
        body.get(cursor..end)
            .ok_or_else(|| anyhow::anyhow!("truncated work node"))?
            .try_into()?,
    ) as usize;
    cursor = end;
    ensure!(
        length <= 4096 && body.len().checked_sub(cursor) == Some(length),
        "invalid work node payload"
    );
    let expected: Vec<_> = [previous, child]
        .into_iter()
        .flatten()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    ensure!(
        expected.len() == usize::from(header.children),
        "invalid work node child count"
    );
    Ok(WorkNodeEdges {
        digest: header.digest,
        node: WorkNode {
            previous,
            child,
            payload: body[cursor..].to_vec(),
        },
        expected,
        next: 0,
        after: None,
    })
}

/// Inspect one edge per call. Returns the node only after all expected edges and
/// the end of the stored edge range have been checked.
pub(crate) async fn work_node_edge_step_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    state: &mut WorkNodeEdges,
    max_bytes: u64,
) -> Result<Option<WorkNode>> {
    const EDGE_BYTES: u64 = 32;
    if max_bytes < EDGE_BYTES {
        return Err(sync::protocol::SyncError::LimitTooSmall {
            required: EDGE_BYTES,
        }
        .into());
    }
    let mut q = Query::select();
    q.column(Alias::new("child_manifest"))
        .from(Alias::new("sync_manifest_edges"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("parent_manifest")).eq(state.digest.to_vec()));
    if let Some(after) = state.after {
        q.and_where(Expr::col(Alias::new("child_manifest")).gt(after.to_vec()));
    }
    q.order_by(Alias::new("child_manifest"), Order::Asc)
        .limit(1);
    let Some(row) = tx.query_one(&q.to_owned()).await? else {
        ensure!(state.next == state.expected.len(), "missing work node edge");
        return Ok(Some(state.node.clone()));
    };
    let child: Digest = row
        .try_get_by_index::<Vec<u8>>(0)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid work edge"))?;
    ensure!(
        state.expected.get(state.next) == Some(&child),
        "unexpected work node edge"
    );
    state.next += 1;
    state.after = Some(child);
    Ok(None)
}

/// Retain one completed dependency of a private partition/root reservation.
/// Published manifests are compare-only; an absent edge is a corruption error.
pub(crate) async fn reserve_manifest_edge_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    parent: &ManifestRef,
    child: Digest,
) -> Result<()> {
    let expected_kind = match parent.kind {
        ManifestKind::ContextPartition => ManifestKind::Context,
        ManifestKind::ContextRoot => ManifestKind::ContextPartition,
        _ => anyhow::bail!("reserved manifest cannot have dependencies"),
    };
    lock_on(tx, namespace).await?;
    let ns = namespace.as_bytes().to_vec();
    let parent_query = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
        .and_where(Expr::col(Alias::new("digest")).eq(parent.manifest_digest.to_vec()))
        .to_owned();
    ensure!(
        tx.query_one(&parent_query).await?.is_some(),
        "parent reservation missing"
    );
    let reservation = reserve_manifest_on(tx, namespace, parent).await?;
    let child_query = Query::select()
        .column(Alias::new("kind"))
        .from(Alias::new("sync_manifests"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
        .and_where(Expr::col(Alias::new("digest")).eq(child.to_vec()))
        .to_owned();
    let child_row = tx
        .query_one(&child_query)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing manifest dependency"))?;
    ensure!(
        child_row.try_get_by_index::<i64>(0)? == kind(expected_kind),
        "invalid manifest dependency"
    );
    let edge = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sync_manifest_edges"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.clone()))
        .and_where(Expr::col(Alias::new("parent_manifest")).eq(parent.manifest_digest.to_vec()))
        .and_where(Expr::col(Alias::new("child_manifest")).eq(child.to_vec()))
        .to_owned();
    if tx.query_one(&edge).await?.is_some() {
        return Ok(());
    }
    ensure!(
        reservation == Reservation::Reserved,
        "published manifest dependency is missing"
    );
    let q = Query::insert()
        .into_table(Alias::new("sync_manifest_edges"))
        .columns(["namespace", "parent_manifest", "child_manifest"].map(Alias::new))
        .values_panic([
            ns.into(),
            parent.manifest_digest.to_vec().into(),
            child.to_vec().into(),
        ])
        .to_owned();
    tx.execute(&q).await?;
    Ok(())
}

/// Enforce one logical context digest for a published context manifest across
/// all verified staged segments. The namespace lock must be held by the caller.
pub(crate) async fn ensure_context_digest_on(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    manifest: Digest,
    digest: Digest,
) -> Result<()> {
    let q = Query::select()
        .column(Alias::new("context_digest"))
        .from(Alias::new("sync_context_ranges"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("context_manifest")).eq(manifest.to_vec()))
        .and_where(Expr::col(Alias::new("context_digest")).is_not_null())
        .limit(1)
        .to_owned();
    if let Some(row) = tx.query_one(&q).await? {
        ensure!(
            row.try_get_by_index::<Vec<u8>>(0)? == digest,
            "context manifest has a conflicting logical digest"
        );
    }
    Ok(())
}
