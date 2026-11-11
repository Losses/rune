//! Export an immutable share base from a leased local snapshot.
use crate::causal_capabilities::CausalEndpoint;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use sync::{SnapshotSource, protocol as p, share};
use uuid::Uuid;

const TARGET_MIN_GZIP: u64 = 512 * 1024;
const TARGET_MAX_GZIP: u64 = 2 * 1024 * 1024;
const MAX_BUCKET_BITS: u32 = 31;
const SNAPSHOT_REQUESTS: u32 = p::MAX_ROUND_REQUESTS;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseObject {
    /// Content hash without the `.gz` suffix.
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseExport<M: p::SyncModel> {
    pub base_id: String,
    pub manifest: share::BaseManifest<M>,
    /// Upload order is chunks, context fragments, and finally the manifest.
    pub objects: Vec<BaseObject>,
    pub bucket_bits: BTreeMap<String, u32>,
}

#[derive(Clone, Debug)]
pub struct BaseExportOptions {
    /// This device's share-stream prefix (`<stream>.<generation>`).
    pub own_prefix: String,
    /// Checkpoints already applied locally, keyed by source prefix.
    pub covers: BTreeMap<String, share::Checkpoint>,
    /// Bucket counts from a previous manifest, keyed by collection tag.
    pub previous_bucket_bits: BTreeMap<String, u32>,
    /// Chunk names referenced by retained manifests owned by the caller.
    pub retained_chunk_names: BTreeSet<String>,
    pub work: p::WorkLimits,
    pub pages: p::PageLimits,
}

impl Default for BaseExportOptions {
    fn default() -> Self {
        Self {
            own_prefix: String::new(),
            covers: BTreeMap::new(),
            previous_bucket_bits: BTreeMap::new(),
            retained_chunk_names: BTreeSet::new(),
            work: p::WorkLimits {
                max_requests: SNAPSHOT_REQUESTS,
                max_bytes: p::MAX_ROUND_BYTES,
                max_depth: p::MAX_DEPTH,
                max_rows_scanned: p::MAX_ROWS,
                max_mutations: p::MAX_MUTATIONS,
            },
            pages: p::PageLimits {
                max_scanned: p::MAX_PAGE_ITEMS,
                max_bytes: p::MAX_PAGE_BYTES,
            },
        }
    }
}

/// The source only exposes registered replicated collections. Local DB columns,
/// credentials, and device settings are never queried or serialized here.
pub async fn export_base<M: p::SyncModel>(
    source: &CausalEndpoint<M>,
    options: BaseExportOptions,
) -> Result<BaseExport<M>> {
    ensure!(
        options.pages.max_scanned > 0 && options.pages.max_bytes > 0,
        "invalid export page limits"
    );
    ensure!(
        !options.own_prefix.is_empty(),
        "missing local share-stream prefix"
    );
    let scope = p::Scope {
        collections: M::collections().into_iter().collect(),
    };
    ensure!(
        !scope.collections.is_empty(),
        "cannot export a model without collections"
    );
    let request = Uuid::new_v4();
    let descriptor_result = async {
        let mut resume = None;
        loop {
            match source
                .open_snapshot_with_ttl(
                    request,
                    scope.clone(),
                    options.work,
                    resume.take(),
                    p::MAX_SNAPSHOT_TTL_MS,
                )
                .await
                .context("opening base snapshot")?
            {
                p::WorkOutcome::Ready(descriptor) => break Ok(descriptor),
                p::WorkOutcome::Incomplete { cursor } => resume = Some(cursor),
            }
        }
    }
    .await;
    let descriptor = match descriptor_result {
        Ok(descriptor) => descriptor,
        Err(error) => {
            source
                .release_request(request)
                .await
                .context("releasing failed base snapshot request")?;
            return Err(error);
        }
    };
    let mut covers = options.covers.clone();
    covers.insert(
        options.own_prefix.clone(),
        share::Checkpoint {
            seq: descriptor.head.sequence,
            anchor: descriptor.head.anchor,
        },
    );
    let mut export = BaseExport {
        base_id: String::new(),
        manifest: share::BaseManifest {
            format: share::FORMAT_VERSION,
            covers,
            collections: BTreeMap::new(),
        },
        objects: Vec::new(),
        bucket_bits: BTreeMap::new(),
    };
    let result = build_export(source, &descriptor, options, &mut export).await;
    source
        .close_snapshot(descriptor.snapshot_id)
        .await
        .context("closing base snapshot")?;
    result?;
    let (base_id, manifest_bytes) = share::encode_manifest(&export.manifest)?;
    export.base_id = base_id.clone();
    export.objects.push(BaseObject {
        name: base_id,
        bytes: manifest_bytes,
    });
    Ok(export)
}

async fn build_export<M: p::SyncModel>(
    source: &dyn p::SnapshotSource<M>,
    descriptor: &p::SnapshotDescriptor<M>,
    options: BaseExportOptions,
    export: &mut BaseExport<M>,
) -> Result<()> {
    let mut chunk_objects = Vec::new();
    let mut emitted_chunks = BTreeSet::new();
    let mut context_objects = Vec::new();
    for root in &descriptor.roots {
        let tag = M::collection_tag(&root.range.collection).to_owned();
        let bits = choose_initial_bits(
            root.candidate_bytes,
            options.previous_bucket_bits.get(&tag).copied(),
        );
        let (bits, buckets) = build_buckets(source, descriptor, root, bits, &options).await?;
        export.bucket_bits.insert(tag.clone(), bits);
        let mut chunk_summaries = Vec::with_capacity(buckets.len());
        for (summary, name, compressed) in buckets {
            if !options.retained_chunk_names.contains(&name) && emitted_chunks.insert(name.clone())
            {
                chunk_objects.push(BaseObject {
                    name: name.clone(),
                    bytes: compressed,
                });
            }
            chunk_summaries.push(share::BucketSummary {
                range: summary.range,
                candidate_count: summary.candidate_count,
                candidate_bytes: summary.candidate_bytes,
                aggregate: summary.aggregate,
                chunk: name,
            });
        }
        let mut context_names = Vec::new();
        if let Some(context) = descriptor
            .contexts
            .iter()
            .find(|value| value.collection == root.range.collection)
        {
            for ordinal in 0..context.context.manifest.fragment_count {
                let fragment = source
                    .read_fragment(
                        descriptor.snapshot_id,
                        context.context.manifest.clone(),
                        ordinal,
                        p::MAX_PAGE_BYTES,
                    )
                    .await
                    .context("reading base context fragment")?;
                ensure!(
                    fragment.ordinal == ordinal
                        && fragment.manifest == context.context.manifest
                        && p::fragment_digest(&fragment)? == fragment.digest,
                    "snapshot context fragment failed integrity validation"
                );
                let name = blake3::hash(&fragment.bytes).to_hex().to_string();
                let bytes = gzip(&fragment.bytes)?;
                context_names.push(name.clone());
                if !options.retained_chunk_names.contains(&name)
                    && emitted_chunks.insert(name.clone())
                {
                    context_objects.push(BaseObject { name, bytes });
                }
            }
        }
        export.manifest.collections.insert(
            tag,
            share::CollectionSummary {
                root: root.clone(),
                bucket_count: u32::try_from(chunk_summaries.len())
                    .context("bucket count overflow")?,
                buckets: chunk_summaries,
                context_fragments: context_names,
            },
        );
    }
    export.objects.extend(chunk_objects);
    export.objects.extend(context_objects);
    Ok(())
}

async fn build_buckets<M: p::SyncModel>(
    source: &dyn p::SnapshotSource<M>,
    descriptor: &p::SnapshotDescriptor<M>,
    root: &p::RangeSummary<M>,
    mut bits: u32,
    options: &BaseExportOptions,
) -> Result<(u32, Vec<(p::RangeSummary<M>, String, Vec<u8>)>)> {
    if root.candidate_count == 0 {
        return Ok((0, Vec::new()));
    }
    let mut visited = BTreeSet::new();
    let mut best_bits = bits;
    let mut best_gap = u64::MAX;
    loop {
        visited.insert(bits);
        let buckets = build_bucket_values(source, descriptor, root, bits, options).await?;
        let total_gzip = buckets
            .iter()
            .map(|(_, _, bytes)| bytes.len() as u64)
            .sum::<u64>();
        let count = 1_u32.checked_shl(bits).context("bucket count overflow")?;
        let average = total_gzip / u64::from(count);
        let gap = bucket_band_gap(average);
        if gap < best_gap {
            best_gap = gap;
            best_bits = bits;
        }
        let Some(next_bits) = next_bucket_bits(bits, average) else {
            return if bits == best_bits {
                Ok((bits, buckets))
            } else {
                Ok((
                    best_bits,
                    build_bucket_values(source, descriptor, root, best_bits, options).await?,
                ))
            };
        };
        if visited.contains(&next_bits) {
            return Ok((
                best_bits,
                build_bucket_values(source, descriptor, root, best_bits, options).await?,
            ));
        }
        bits = next_bits;
    }
}

async fn build_bucket_values<M: p::SyncModel>(
    source: &dyn p::SnapshotSource<M>,
    descriptor: &p::SnapshotDescriptor<M>,
    root: &p::RangeSummary<M>,
    bits: u32,
    options: &BaseExportOptions,
) -> Result<Vec<(p::RangeSummary<M>, String, Vec<u8>)>> {
    let count = 1_u32.checked_shl(bits).context("bucket count overflow")?;
    let mut buckets = Vec::with_capacity(count as usize);
    for index in 0..count {
        let range = prefix_range(root.range.collection.clone(), bits, index);
        let summary =
            summarize(source, descriptor.snapshot_id, range.clone(), options.work).await?;
        let candidates = read_leaf(source, descriptor.snapshot_id, range, options.pages).await?;
        let raw = canonical_json(&candidates).context("encoding bucket candidates")?;
        let (name, compressed) = share::encode_chunk(raw)?;
        buckets.push((summary, name, compressed));
    }
    Ok(buckets)
}

fn next_bucket_bits(bits: u32, average_gzip_bytes: u64) -> Option<u32> {
    if (TARGET_MIN_GZIP..=TARGET_MAX_GZIP).contains(&average_gzip_bytes) {
        None
    } else if average_gzip_bytes > TARGET_MAX_GZIP && bits < MAX_BUCKET_BITS {
        Some(bits + 1)
    } else if average_gzip_bytes < TARGET_MIN_GZIP && bits > 0 {
        Some(bits - 1)
    } else {
        None
    }
}

fn bucket_band_gap(average_gzip_bytes: u64) -> u64 {
    if average_gzip_bytes < TARGET_MIN_GZIP {
        TARGET_MIN_GZIP - average_gzip_bytes
    } else {
        average_gzip_bytes.saturating_sub(TARGET_MAX_GZIP)
    }
}

async fn summarize<M: p::SyncModel>(
    source: &dyn p::SnapshotSource<M>,
    snapshot: Uuid,
    range: p::KeyRange<M>,
    limits: p::WorkLimits,
) -> Result<p::RangeSummary<M>> {
    let mut resume = None;
    loop {
        match source
            .summarize(snapshot, range.clone(), limits, resume.take())
            .await?
        {
            p::WorkOutcome::Ready(summary) => return Ok(summary),
            p::WorkOutcome::Incomplete { cursor } => resume = Some(cursor),
        }
    }
}

async fn read_leaf<M: p::SyncModel>(
    source: &dyn p::SnapshotSource<M>,
    snapshot: Uuid,
    range: p::KeyRange<M>,
    limits: p::PageLimits,
) -> Result<Vec<p::Candidate<M>>> {
    let mut candidates = Vec::new();
    let mut after = None;
    loop {
        let page = source.leaf(snapshot, range.clone(), after, limits).await?;
        candidates.extend(page.candidates);
        if page.done {
            return Ok(candidates);
        }
        after = page.next;
        ensure!(after.is_some(), "snapshot leaf page did not advance");
    }
}

fn choose_initial_bits(bytes: u64, previous: Option<u32>) -> u32 {
    if let Some(bits) = previous.filter(|bits| *bits <= MAX_BUCKET_BITS) {
        return bits;
    }
    let mut bits = 0;
    let mut average = bytes;
    while average > TARGET_MAX_GZIP && bits < MAX_BUCKET_BITS {
        bits += 1;
        average /= 2;
    }
    while average < TARGET_MIN_GZIP && bits > 0 {
        bits -= 1;
        average = average.saturating_mul(2);
    }
    bits
}

fn prefix_range<M: p::SyncModel>(
    collection: M::Collection,
    bits: u32,
    index: u32,
) -> p::KeyRange<M> {
    if bits == 0 {
        return p::KeyRange {
            collection,
            lower: None,
            upper: None,
        };
    }
    let shift = 128 - bits;
    let lower = u128::from(index) << shift;
    let upper = lower | ((1_u128 << shift) - 1);
    p::KeyRange {
        collection,
        lower: Some(Uuid::from_u128(lower)),
        upper: Some(Uuid::from_u128(upper)),
    }
}

fn gzip(bytes: &[u8]) -> Result<Vec<u8>> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(bytes)
        .context("compressing context fragment")?;
    encoder
        .finish()
        .context("finishing context fragment compression")
}

fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    fn write(value: &Value, output: &mut Vec<u8>) -> Result<()> {
        match value {
            Value::Object(object) => {
                output.push(b'{');
                let sorted: BTreeMap<_, _> = object.iter().collect();
                for (index, (key, value)) in sorted.iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    serde_json::to_writer(&mut *output, key)?;
                    output.push(b':');
                    write(value, output)?;
                }
                output.push(b'}');
            }
            Value::Array(values) => {
                output.push(b'[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    write(value, output)?;
                }
                output.push(b']');
            }
            _ => serde_json::to_writer(output, value)?,
        }
        Ok(())
    }
    let value = serde_json::to_value(value)?;
    let mut output = Vec::new();
    write(&value, &mut output)?;
    Ok(output)
}

/// A helper for callers and tests that need the canonical manifest bytes.
pub fn canonical_manifest<M: p::SyncModel>(
    manifest: &share::BaseManifest<M>,
) -> Result<(String, Vec<u8>)> {
    share::encode_manifest(manifest).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::{TARGET_MAX_GZIP, TARGET_MIN_GZIP, next_bucket_bits};

    #[test]
    fn bucket_bits_are_stable_inside_target_band_and_adjust_outside_it() {
        assert_eq!(next_bucket_bits(4, TARGET_MIN_GZIP), None);
        assert_eq!(next_bucket_bits(4, TARGET_MAX_GZIP), None);
        assert_eq!(next_bucket_bits(4, TARGET_MAX_GZIP + 1), Some(5));
        assert_eq!(next_bucket_bits(4, TARGET_MIN_GZIP - 1), Some(3));
        assert_eq!(next_bucket_bits(0, TARGET_MIN_GZIP - 1), None);
        assert_eq!(next_bucket_bits(31, TARGET_MAX_GZIP + 1), None);
    }
}
