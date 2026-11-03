//! Atomic publication of complete, validated causal manifests.
use crate::causal_leases::lock_on;
use anyhow::{Result, bail, ensure};
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, ExprTrait,
    sea_query::{Alias, Condition, Expr, Func, Order, Query},
};
use std::collections::BTreeSet;
use sync::protocol::SyncError;
use sync::protocol::{
    Digest, Fragment, MAX_FRAGMENTS, MAX_LOGICAL_BYTES, MAX_LOGICAL_ITEMS, ManifestKind,
    ManifestRef, content_digest, fragment_digest, manifest_digest,
};
use uuid::Uuid;

fn kind(k: ManifestKind) -> i64 {
    match k {
        ManifestKind::KeyEvent => 0,
        ManifestKind::MergeEvent => 1,
        ManifestKind::Context => 2,
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
    ensure!(count <= MAX_FRAGMENTS as usize, SyncError::LogicalLimit);
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
        1 => ManifestKind::MergeEvent,
        2 => ManifestKind::Context,
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
        SyncError::WorkLimit
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragment_body_uses_canonical_balanced_bounds() -> Result<()> {
        let (manifest, fragments) = fragment_body(ManifestKind::Context, b"0123456789", 0, 6)?;
        assert_eq!(fragments.len(), 2);
        for fragment in &fragments {
            let (start, end) = sync::protocol::fragment_bounds(&manifest, fragment.ordinal)?;
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
}
