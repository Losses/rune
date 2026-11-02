//! Atomic bounded floor advancement with exact qualified context partitions.
use crate::{
    causal_leases::{lock_on, minimum_pins_on},
    causal_manifests::{fragment_body, publish_on, read_on},
};
use anyhow::{Result, ensure};
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, ExprTrait, FromQueryResult,
    sea_query::{Alias, Expr, Func, Query},
};
use std::collections::{BTreeMap, BTreeSet};
use sync::protocol::{
    self, ContextPartitionRef, ContextRef, ContextRoot, ContextSegment, KeyRange, ManifestKind,
    Scope, SyncModel,
};
use uuid::Uuid;

#[derive(FromQueryResult)]
struct Meta {
    model: String,
    schema: String,
    generation: Vec<u8>,
    head: i64,
    state_seq: i64,
    history_floor: i64,
    floor_state_seq: i64,
    baseline_generation: Vec<u8>,
}
#[derive(FromQueryResult)]
struct Assertion {
    collection: Vec<u8>,
    state_seq: i64,
    lower_bound: Vec<u8>,
    upper_bound: Vec<u8>,
    context_manifest: Vec<u8>,
}
pub(crate) fn valid_bound(bytes: &[u8]) -> bool {
    bytes == [0] || bytes == [2] || (bytes.len() == 17 && bytes[0] == 1)
}
pub(crate) fn decode_bound(bytes: &[u8], lower: bool) -> Result<Option<Uuid>> {
    if (lower && bytes == [0]) || (!lower && bytes == [2]) {
        return Ok(None);
    }
    ensure!(bytes.len() == 17 && bytes[0] == 1, "invalid finite bound");
    Ok(Some(Uuid::from_slice(&bytes[1..])?))
}
fn encode_bound(value: Option<Uuid>, lower: bool) -> Vec<u8> {
    match value {
        None => vec![if lower { 0 } else { 2 }],
        Some(id) => {
            let mut bytes = vec![1];
            bytes.extend_from_slice(id.as_bytes());
            bytes
        }
    }
}
pub(crate) type Interval = (Vec<u8>, Vec<u8>, sync::causal::Context);
/// Normalize overlapping assertions without collapsing unobserved gaps.
pub(crate) fn normalize(
    input: &[Interval],
    max_comparisons: u64,
    max_bytes: u64,
) -> Result<(Vec<Interval>, u64, u64)> {
    let mut bounds = BTreeSet::from([vec![0_u8], vec![2_u8]]);
    for (lo, hi, _) in input {
        ensure!(
            valid_bound(lo) && valid_bound(hi) && lo < hi,
            "invalid context bounds"
        );
        bounds.insert(lo.clone());
        bounds.insert(hi.clone());
    }
    let bounds: Vec<_> = bounds.into_iter().collect();
    let mut output: Vec<Interval> = Vec::new();
    let mut comparisons = 0_u64;
    let mut bytes = 0_u64;
    for pair in bounds.windows(2) {
        let mut context = sync::causal::Context::default();
        for (lo, hi, value) in input {
            comparisons = comparisons
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("context work overflow"))?;
            ensure!(
                comparisons <= max_comparisons,
                protocol::SyncError::WorkLimit
            );
            if *lo <= pair[0] && pair[1] <= *hi {
                context = context.union(value)?;
            }
        }
        if let Some(last) = output.last_mut()
            && last.2 == context
        {
            last.1 = pair[1].clone();
            continue;
        }
        let body = protocol::canonical_context(&protocol::exact_context(&context))?;
        bytes = bytes
            .checked_add(body.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("context byte overflow"))?;
        ensure!(bytes <= max_bytes, protocol::SyncError::WorkLimit);
        output.push((pair[0].clone(), pair[1].clone(), context));
    }
    Ok((output, comparisons, bytes))
}
async fn count(tx: &DatabaseTransaction, ns: Uuid, table: &str, condition: Expr) -> Result<u64> {
    let q = Query::select()
        .expr(Func::count(Expr::val(1)))
        .from(Alias::new(table))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(condition)
        .to_owned();
    Ok(u64::try_from(
        tx.query_one(&q)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing count"))?
            .try_get_by_index::<i64>(0)?,
    )?)
}
/// Advance a floor in one bounded caller transaction. Budget overflow is
/// an error; callers must roll back. No network call occurs in this transaction.
pub async fn compact_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    ns: Uuid,
    through_state: i64,
    max_rows: u64,
    max_bytes: u64,
) -> Result<()> {
    ensure!(
        !ns.is_nil()
            && max_rows > 0
            && max_rows <= 1024
            && max_bytes > 0
            && max_bytes <= protocol::DEFAULT_LOGICAL_BYTES,
        "invalid compaction budget"
    );
    protocol::validate_model::<M>()?;
    lock_on(tx, ns).await?;
    let q = Query::select()
        .columns(
            [
                "model",
                "schema",
                "generation",
                "head",
                "state_seq",
                "history_floor",
                "floor_state_seq",
                "baseline_generation",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_meta"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .to_owned();
    let meta = Meta::find_by_statement(tx.get_database_backend().build(&q))
        .one(tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing causal namespace"))?;
    ensure!(
        meta.model == M::model_id()
            && meta.schema == M::schema_id()
            && through_state > meta.floor_state_seq
            && through_state <= meta.state_seq,
        "invalid compaction state"
    );
    let generation = Uuid::from_slice(&meta.generation)?;
    let other_generation = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sync_journal"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("generation")).ne(generation.as_bytes().to_vec()))
        .limit(1)
        .to_owned();
    ensure!(
        tx.query_one(&other_generation).await?.is_none(),
        "multiple journal generations require generation-aware compaction"
    );
    let q = Query::select()
        .columns(["sequence", "anchor_after"].map(Alias::new))
        .from(Alias::new("sync_journal"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("generation")).eq(generation.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("state_seq")).eq(through_state))
        .to_owned();
    let row = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing floor publication"))?;
    let floor: i64 = row.try_get_by_index(0)?;
    let anchor: Vec<u8> = row.try_get_by_index(1)?;
    ensure!(
        floor > meta.history_floor && floor <= meta.head && anchor.len() == 32,
        "invalid floor publication"
    );
    if let Some((pin_state, pin_head)) = minimum_pins_on(tx, ns).await? {
        ensure!(
            through_state <= pin_state && floor <= pin_head,
            "floor is protected by a live lease"
        );
    }
    let q = Query::select()
        .columns(
            [
                "collection",
                "state_seq",
                "lower_bound",
                "upper_bound",
                "context_manifest",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_context_ranges"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("state_seq")).gte(meta.floor_state_seq))
        .and_where(Expr::col(Alias::new("state_seq")).lte(through_state))
        .cond_where(crate::causal_context::active_basis_condition(
            meta.floor_state_seq,
            meta.baseline_generation.clone(),
        ))
        .limit(max_rows + 1)
        .to_owned();
    let assertions = Assertion::find_by_statement(tx.get_database_backend().build(&q))
        .all(tx)
        .await?;
    ensure!(
        assertions.len() as u64 <= max_rows,
        "compaction work limit exceeded"
    );
    let mut collections = M::collections();
    collections.sort_by(|a, b| {
        M::collection_tag(a)
            .as_bytes()
            .cmp(M::collection_tag(b).as_bytes())
    });
    let mut assertions_by_tag: BTreeMap<String, Vec<Interval>> = collections
        .iter()
        .map(|c| (M::collection_tag(c).to_string(), Vec::new()))
        .collect();
    type Bounds = (Vec<u8>, Vec<u8>);
    let mut baseline_ranges: BTreeMap<String, Vec<Bounds>> = BTreeMap::new();
    let mut read_bytes = 0_u64;
    let mut read_fragments = 0_u64;
    for a in &assertions {
        ensure!(
            valid_bound(&a.lower_bound)
                && valid_bound(&a.upper_bound)
                && a.lower_bound < a.upper_bound,
            "invalid context bounds"
        );
        let tag = std::str::from_utf8(&a.collection)?;
        let contexts = assertions_by_tag
            .get_mut(tag)
            .ok_or_else(|| anyhow::anyhow!("unknown assertion collection"))?;
        if meta.floor_state_seq > 0 && a.state_seq == meta.floor_state_seq {
            baseline_ranges
                .entry(tag.to_owned())
                .or_default()
                .push((a.lower_bound.clone(), a.upper_bound.clone()));
        }
        let digest: [u8; 32] = a
            .context_manifest
            .clone()
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid context digest"))?;
        let remaining = max_bytes
            .checked_sub(read_bytes)
            .ok_or_else(|| anyhow::anyhow!("compaction byte limit"))?;
        ensure!(remaining > 0, "compaction byte limit");
        let (manifest, body) = read_on(tx, ns, digest, remaining, u32::try_from(max_rows)?).await?;
        read_fragments = read_fragments
            .checked_add(u64::from(manifest.fragment_count))
            .ok_or_else(|| anyhow::anyhow!("compaction fragment overflow"))?;
        ensure!(read_fragments <= max_rows, "compaction fragment limit");
        ensure!(
            manifest.kind == ManifestKind::Context,
            "invalid context manifest kind"
        );
        read_bytes = read_bytes
            .checked_add(body.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("compaction byte overflow"))?;
        let decoded = protocol::decode_context(&body)?;
        contexts.push((
            a.lower_bound.clone(),
            a.upper_bound.clone(),
            protocol::causal_context(&decoded)?,
        ));
    }
    if meta.floor_state_seq > 0 {
        for collection in &collections {
            let tag = M::collection_tag(collection);
            let mut ranges = baseline_ranges
                .remove(tag)
                .ok_or_else(|| anyhow::anyhow!("incomplete baseline coverage"))?;
            ranges.sort();
            let mut end = vec![0_u8];
            for (lo, hi) in ranges {
                ensure!(lo == end, "baseline gap or overlap");
                end = hi;
            }
            ensure!(end == [2], "incomplete baseline coverage");
        }
    }
    // Produce the exact observation function on a complete disjoint partition.
    // The quadratic scan is explicitly admitted against the work budget.
    let mut contexts: BTreeMap<String, Vec<Interval>> = BTreeMap::new();
    let mut comparisons = 0_u64;
    let mut normalized_bytes = 0_u64;
    for collection in &collections {
        let tag = M::collection_tag(collection);
        let input = assertions_by_tag
            .remove(tag)
            .ok_or_else(|| anyhow::anyhow!("missing collection"))?;
        let remaining_work = max_rows
            .checked_sub(comparisons)
            .ok_or_else(|| anyhow::anyhow!("compaction comparison limit"))?;
        let remaining_bytes = max_bytes
            .checked_sub(read_bytes)
            .and_then(|v| v.checked_sub(normalized_bytes))
            .ok_or_else(|| anyhow::anyhow!("compaction byte limit"))?;
        let (output, work, bytes) = normalize(&input, remaining_work, remaining_bytes)?;
        comparisons += work;
        normalized_bytes += bytes;
        contexts.insert(tag.to_owned(), output);
    }
    let journal_rows = count(
        tx,
        ns,
        "sync_journal",
        Expr::col(Alias::new("sequence")).lte(floor),
    )
    .await?;
    let old_ranges = count(
        tx,
        ns,
        "sync_context_ranges",
        Expr::col(Alias::new("state_seq")).lte(through_state),
    )
    .await?;
    let dead_candidates = count(
        tx,
        ns,
        "sync_candidates",
        sea_orm::sea_query::Condition::any()
            .add(
                sea_orm::sea_query::Condition::all()
                    .add(Expr::col(Alias::new("live_until")).is_not_null())
                    .add(Expr::col(Alias::new("live_until")).lte(through_state)),
            )
            .add(
                sea_orm::sea_query::Condition::all()
                    .add(Expr::col(Alias::new("live_from")).is_null())
                    .add(Expr::col(Alias::new("retention_seq")).lte(through_state)),
            )
            .into(),
    )
    .await?;
    let intervals: u64 = contexts
        .values()
        .flatten()
        .map(|(_, _, context)| {
            context
                .entries()
                .map(|(_, prefix, intervals)| u64::from(prefix > 0) + intervals.len() as u64)
                .sum::<u64>()
        })
        .sum();
    let segment_count: u64 = contexts
        .values()
        .map(|segments| segments.len() as u64)
        .sum();
    let projected = journal_rows
        .checked_add(old_ranges)
        .and_then(|v| v.checked_add(dead_candidates))
        .and_then(|v| v.checked_add(intervals))
        .and_then(|v| v.checked_add(segment_count.saturating_mul(8)))
        .and_then(|v| v.checked_add((collections.len() as u64) * 10 + 10))
        .ok_or_else(|| anyhow::anyhow!("compaction work overflow"))?;
    ensure!(projected <= max_rows, "compaction mutation limit exceeded");
    let scope = Scope {
        collections: collections.iter().cloned().collect(),
    };
    let mut partitions = Vec::new();
    for collection in collections {
        let tag = M::collection_tag(&collection);
        let pieces = contexts
            .remove(tag)
            .ok_or_else(|| anyhow::anyhow!("missing collection"))?;
        let range = KeyRange {
            collection: collection.clone(),
            lower: None,
            upper: None,
        };
        let mut segments = Vec::new();
        let mut child_contexts = Vec::new();
        for (lower, upper, context) in pieces {
            let exact = protocol::exact_context(&context);
            let bytes = protocol::canonical_context(&exact)?;
            let (manifest, fragments) = fragment_body(
                ManifestKind::Context,
                &bytes,
                exact.writers.len() as u64,
                1024 * 1024,
            )?;
            let new = publish_on(tx, ns, &manifest, &fragments, max_bytes, |body| {
                protocol::decode_context(body)?;
                Ok(vec![])
            })
            .await?;
            if new {
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
                                ns.as_bytes().to_vec().into(),
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
            child_contexts.push(manifest.manifest_digest);
            segments.push(ContextSegment {
                range: KeyRange {
                    collection: collection.clone(),
                    lower: decode_bound(&lower, true)?,
                    upper: decode_bound(&upper, false)?,
                },
                context: ContextRef {
                    digest: protocol::context_digest(&exact)?,
                    manifest,
                },
            });
        }
        let part_bytes = protocol::canonical_context_partition(&range, &segments)?;
        let (part, fragments) = fragment_body(
            ManifestKind::ContextPartition,
            &part_bytes,
            segments.len() as u64,
            1024 * 1024,
        )?;
        publish_on(tx, ns, &part, &fragments, max_bytes, |body| {
            protocol::decode_context_partition::<M>(&range, body)?;
            Ok(child_contexts)
        })
        .await?;
        partitions.push(ContextPartitionRef {
            collection,
            partition: part,
        });
        // Defer baseline row insertion until the old partition has been removed.
    }
    let root = ContextRoot {
        scope: scope.clone(),
        partitions,
    };
    let root_bytes = protocol::canonical_context_root(&root)?;
    let (root_manifest, fragments) = fragment_body(
        ManifestKind::ContextRoot,
        &root_bytes,
        root.partitions.len() as u64,
        1024 * 1024,
    )?;
    let children = root
        .partitions
        .iter()
        .map(|entry| entry.partition.manifest_digest)
        .collect::<Vec<_>>();
    publish_on(tx, ns, &root_manifest, &fragments, max_bytes, |body| {
        protocol::decode_context_root::<M>(&scope, body)?;
        Ok(children)
    })
    .await?;
    let new_basis = Uuid::new_v4();
    let q = Query::delete()
        .from_table(Alias::new("sync_context_ranges"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("state_seq")).lte(through_state))
        .cond_where(crate::causal_context::active_basis_condition(
            meta.floor_state_seq,
            meta.baseline_generation.clone(),
        ))
        .to_owned();
    tx.execute(&q).await?;
    for part in &root.partitions {
        let range = KeyRange {
            collection: part.collection.clone(),
            lower: None,
            upper: None,
        };
        let (_, body) = read_on(
            tx,
            ns,
            part.partition.manifest_digest,
            max_bytes,
            u32::try_from(max_rows)?,
        )
        .await?;
        let segments = protocol::decode_context_partition::<M>(&range, &body)?;
        for (ordinal, segment) in segments.iter().enumerate() {
            let lower = encode_bound(segment.range.lower, true);
            let upper = encode_bound(segment.range.upper, false);
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
                    ns.as_bytes().to_vec().into(),
                    M::collection_tag(&part.collection)
                        .as_bytes()
                        .to_vec()
                        .into(),
                    new_basis.as_bytes().to_vec().into(),
                    through_state.into(),
                    i64::try_from(ordinal)?.into(),
                    lower.into(),
                    upper.into(),
                    segment.context.manifest.manifest_digest.to_vec().into(),
                ])
                .to_owned();
            tx.execute(&q).await?;
        }
    }
    let q = Query::update()
        .table(Alias::new("sync_meta"))
        .value(Alias::new("history_floor"), floor)
        .value(Alias::new("floor_anchor"), anchor)
        .value(Alias::new("floor_state_seq"), through_state)
        .value(
            Alias::new("baseline_generation"),
            new_basis.as_bytes().to_vec(),
        )
        .value(
            Alias::new("baseline_manifest"),
            root_manifest.manifest_digest.to_vec(),
        )
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .to_owned();
    ensure!(
        tx.execute(&q).await?.rows_affected() == 1,
        "missing causal namespace"
    );
    let q = Query::delete()
        .from_table(Alias::new("sync_journal"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("sequence")).lte(floor))
        .to_owned();
    tx.execute(&q).await?;
    let dead = sea_orm::sea_query::Condition::any()
        .add(
            sea_orm::sea_query::Condition::all()
                .add(Expr::col(Alias::new("live_until")).is_not_null())
                .add(Expr::col(Alias::new("live_until")).lte(through_state)),
        )
        .add(
            sea_orm::sea_query::Condition::all()
                .add(Expr::col(Alias::new("live_from")).is_null())
                .add(Expr::col(Alias::new("retention_seq")).lte(through_state)),
        );
    let q = Query::delete()
        .from_table(Alias::new("sync_candidates"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .cond_where(dead)
        .to_owned();
    tx.execute(&q).await?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FloorStep {
    Advanced { state_seq: i64 },
    Pinned,
    CaughtUp,
}
/// Advance by at most `max_events` retained events in the caller transaction.
/// Call repeatedly until `CaughtUp`; each completed floor is durable at commit.
/// If a single baseline exceeds the admitted work, returns an error and the
/// caller must roll back; resumable large-baseline construction is separate.
pub async fn maintenance_step_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    ns: Uuid,
    max_events: u64,
    max_rows: u64,
    max_bytes: u64,
) -> Result<FloorStep> {
    ensure!(
        max_events > 0 && max_events <= 128,
        "invalid maintenance event budget"
    );
    lock_on(tx, ns).await?;
    let q = Query::select()
        .columns(
            [
                "model",
                "schema",
                "generation",
                "head",
                "state_seq",
                "history_floor",
                "floor_state_seq",
                "baseline_generation",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_meta"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .to_owned();
    let meta = Meta::find_by_statement(tx.get_database_backend().build(&q))
        .one(tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing causal namespace"))?;
    ensure!(
        meta.model == M::model_id() && meta.schema == M::schema_id(),
        "incompatible model"
    );
    if meta.floor_state_seq == meta.state_seq {
        return Ok(FloorStep::CaughtUp);
    }
    let pins = minimum_pins_on(tx, ns).await?;
    let state_cap = pins.map_or(meta.state_seq, |(state, _)| {
        std::cmp::min(state, meta.state_seq)
    });
    let head_cap = pins.map_or(meta.head, |(_, head)| std::cmp::min(head, meta.head));
    if state_cap <= meta.floor_state_seq || head_cap <= meta.history_floor {
        return Ok(FloorStep::Pinned);
    }
    let q = Query::select()
        .column(Alias::new("state_seq"))
        .from(Alias::new("sync_journal"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("generation")).eq(meta.generation))
        .and_where(Expr::col(Alias::new("state_seq")).gt(meta.floor_state_seq))
        .and_where(Expr::col(Alias::new("state_seq")).lte(state_cap))
        .and_where(Expr::col(Alias::new("sequence")).lte(head_cap))
        .order_by(Alias::new("state_seq"), sea_orm::sea_query::Order::Asc)
        .limit(max_events)
        .to_owned();
    let rows = tx.query_all(&q).await?;
    let target = rows
        .last()
        .ok_or_else(|| anyhow::anyhow!("missing retained history before maintenance target"))?
        .try_get_by_index::<i64>(0)?;
    compact_on::<M>(tx, ns, target, max_rows, max_bytes).await?;
    Ok(FloorStep::Advanced { state_seq: target })
}

#[derive(Clone, Copy, Debug)]
pub struct BaselineStageBudget {
    pub max_bytes: u64,
    pub max_mutations: u64,
}

/// Persist one normalized baseline segment for a claimed build generation.
/// Rows are invisible to readers until a later atomic floor/pointer switch.
/// Repeating the same segment is idempotent; changed content or gaps fail.
/// A resumable builder must record its matching proof node and checkpoint in
/// this same transaction; this operation alone does not prove a complete floor.
pub async fn stage_baseline_segment_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    ns: Uuid,
    lease_id: Uuid,
    now_ms: i64,
    ordinal: u64,
    segment: &ContextSegment<M>,
    budget: BaselineStageBudget,
) -> Result<()> {
    let BaselineStageBudget {
        max_bytes,
        max_mutations,
    } = budget;
    ensure!(
        max_bytes > 0
            && max_bytes <= protocol::MAX_LOGICAL_BYTES
            && max_mutations > 0
            && max_mutations <= protocol::MAX_MUTATIONS,
        "invalid baseline segment budget"
    );
    let lease = crate::causal_leases::building_on(tx, ns, lease_id, now_ms).await?;
    let output = lease
        .output_generation
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("baseline output generation not claimed"))?;
    let basis = lease
        .basis_generation
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing captured baseline generation"))?;
    ensure!(
        output != basis,
        "baseline output must differ from captured generation"
    );
    let floor = lease
        .basis_state_seq
        .ok_or_else(|| anyhow::anyhow!("missing captured baseline position"))?;
    let meta = Query::select()
        .columns(
            [
                "floor_state_seq",
                "baseline_generation",
                "generation",
                "state_seq",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_meta"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .to_owned();
    let meta = tx
        .query_one(&meta)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing causal namespace"))?;
    ensure!(
        meta.try_get_by_index::<i64>(0)? == floor
            && meta.try_get_by_index::<Vec<u8>>(1)? == *basis
            && meta.try_get_by_index::<Vec<u8>>(2)? == lease.generation
            && meta.try_get_by_index::<i64>(3)? >= lease.state_seq,
        "stale baseline build basis"
    );
    let tag = M::collection_tag(&segment.range.collection);
    ensure!(
        M::collection_from_tag(tag) == Some(segment.range.collection.clone())
            && M::collections().contains(&segment.range.collection),
        "invalid baseline collection"
    );
    let lower = encode_bound(segment.range.lower, true);
    let upper = encode_bound(segment.range.upper, false);
    ensure!(
        lower < upper && ordinal <= i64::MAX as u64,
        "invalid baseline segment"
    );
    let reference =
        crate::causal_manifests::reference_on(tx, ns, segment.context.manifest.manifest_digest)
            .await?;
    ensure!(
        reference.encoded_bytes <= max_bytes,
        protocol::SyncError::WorkLimit
    );
    let (manifest, body) = read_on(
        tx,
        ns,
        segment.context.manifest.manifest_digest,
        max_bytes,
        protocol::MAX_FRAGMENTS,
    )
    .await?;
    ensure!(
        manifest == segment.context.manifest && manifest.kind == ManifestKind::Context,
        "invalid baseline context manifest"
    );
    let exact = protocol::decode_context(&body)?;
    ensure!(
        manifest.item_count == exact.writers.len() as u64
            && protocol::context_digest(&exact)? == segment.context.digest,
        "baseline context commitment mismatch"
    );
    crate::causal_manifests::ensure_context_digest_on(
        tx,
        ns,
        manifest.manifest_digest,
        segment.context.digest,
    )
    .await?;
    let context = protocol::causal_context(&exact)?;
    let ordinal = i64::try_from(ordinal)?;
    if ordinal == 0 {
        ensure!(
            lower == [0],
            "baseline partition does not start at lower bound"
        );
    } else {
        let prior = Query::select()
            .columns(["upper_bound", "context_manifest"].map(Alias::new))
            .from(Alias::new("sync_context_ranges"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("collection")).eq(tag.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("basis_generation")).eq(output.clone()))
            .and_where(Expr::col(Alias::new("state_seq")).eq(lease.state_seq))
            .and_where(Expr::col(Alias::new("ordinal")).eq(ordinal - 1))
            .to_owned();
        let prior = tx
            .query_one(&prior)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing preceding baseline segment"))?;
        ensure!(
            prior.try_get_by_index::<Vec<u8>>(0)? == lower,
            "baseline segment gap or overlap"
        );
        let prior_digest: [u8; 32] = prior
            .try_get_by_index::<Vec<u8>>(1)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid preceding context manifest"))?;
        let remaining_bytes = max_bytes
            .checked_sub(body.len() as u64)
            .ok_or_else(|| anyhow::Error::new(protocol::SyncError::WorkLimit))?;
        ensure!(remaining_bytes > 0, protocol::SyncError::WorkLimit);
        let prior_ref = crate::causal_manifests::reference_on(tx, ns, prior_digest).await?;
        ensure!(
            prior_ref.encoded_bytes <= remaining_bytes,
            protocol::SyncError::WorkLimit
        );
        let (prior_manifest, prior_body) = read_on(
            tx,
            ns,
            prior_digest,
            remaining_bytes,
            protocol::MAX_FRAGMENTS,
        )
        .await?;
        ensure!(
            prior_manifest.kind == ManifestKind::Context,
            "invalid preceding context kind"
        );
        let prior_context = protocol::decode_context(&prior_body)?;
        ensure!(
            protocol::context_digest(&prior_context)? != segment.context.digest,
            "adjacent equal baseline contexts must be coalesced"
        );
    }
    let existing = Query::select()
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
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("collection")).eq(tag.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("basis_generation")).eq(output.clone()))
        .and_where(Expr::col(Alias::new("state_seq")).eq(lease.state_seq))
        .and_where(Expr::col(Alias::new("ordinal")).eq(ordinal))
        .to_owned();
    if let Some(row) = tx.query_one(&existing).await? {
        ensure!(
            row.try_get_by_index::<Vec<u8>>(0)? == lower
                && row.try_get_by_index::<Vec<u8>>(1)? == upper
                && row.try_get_by_index::<Vec<u8>>(2)? == manifest.manifest_digest
                && row.try_get_by_index::<Option<Vec<u8>>>(3)?.as_deref()
                    == Some(segment.context.digest.as_slice()),
            "conflicting staged baseline segment"
        );
        return Ok(());
    }
    let mut remaining = max_mutations
        .checked_sub(1)
        .ok_or_else(|| anyhow::Error::new(protocol::SyncError::WorkLimit))?;
    crate::causal_range::index_context_on(
        tx,
        ns,
        manifest.manifest_digest,
        &context,
        &mut remaining,
    )
    .await?;
    let query = Query::insert()
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
            ns.as_bytes().to_vec().into(),
            tag.as_bytes().to_vec().into(),
            output.clone().into(),
            lease.state_seq.into(),
            ordinal.into(),
            lower.into(),
            upper.into(),
            manifest.manifest_digest.to_vec().into(),
            segment.context.digest.to_vec().into(),
        ])
        .to_owned();
    tx.execute(&query).await?;
    Ok(())
}

/// Remove at most `max_rows` journal, retired-candidate, and assertion rows
/// made obsolete by an already committed floor. Safe to resume after a crash;
/// live baselines and all history above the floor remain untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneStep {
    pub removed: u64,
    pub more: bool,
}
pub async fn prune_history_step_on(
    tx: &DatabaseTransaction,
    ns: Uuid,
    max_rows: u64,
) -> Result<PruneStep> {
    use sea_orm::sea_query::{Condition, Order};
    ensure!(
        !ns.is_nil() && max_rows > 0 && max_rows <= 1024,
        "invalid history prune budget"
    );
    lock_on(tx, ns).await?;
    let meta = Query::select()
        .columns(
            [
                "history_floor",
                "floor_state_seq",
                "baseline_generation",
                "head",
                "state_seq",
                "baseline_manifest",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_meta"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .to_owned();
    let row = tx
        .query_one(&meta)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing causal namespace"))?;
    let head_floor: i64 = row.try_get_by_index(0)?;
    let state_floor: i64 = row.try_get_by_index(1)?;
    let basis: Vec<u8> = row.try_get_by_index(2)?;
    ensure!(
        head_floor >= 0
            && state_floor >= 0
            && basis.len() == 16
            && head_floor <= row.try_get_by_index::<i64>(3)?
            && state_floor <= row.try_get_by_index::<i64>(4)?,
        "invalid floor metadata"
    );
    ensure!(
        state_floor == 0 || basis != Uuid::nil().as_bytes(),
        "missing active baseline generation"
    );
    if state_floor > 0 {
        let root: Option<Vec<u8>> = row.try_get_by_index(5)?;
        let root = root
            .filter(|value| value.len() == 32)
            .ok_or_else(|| anyhow::anyhow!("missing active baseline root"))?;
        let query = Query::select()
            .expr(Expr::val(1))
            .from(Alias::new("sync_manifests"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("digest")).eq(root))
            .and_where(Expr::col(Alias::new("kind")).eq(5_i64))
            .limit(1)
            .to_owned();
        ensure!(
            tx.query_one(&query).await?.is_some(),
            "missing active baseline root"
        );
    }
    if let Some((pinned_state, pinned_head)) = minimum_pins_on(tx, ns).await? {
        ensure!(
            pinned_state >= state_floor && pinned_head >= head_floor,
            "history floor exceeds a live lease pin"
        );
    }
    let nil = Uuid::nil().as_bytes().to_vec();
    let mut removed = 0_u64;
    let journal = Query::select()
        .columns(["generation", "sequence"].map(Alias::new))
        .from(Alias::new("sync_journal"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("sequence")).lte(head_floor))
        .order_by(Alias::new("sequence"), Order::Asc)
        .limit(max_rows)
        .to_owned();
    for item in tx.query_all(&journal).await? {
        let generation: Vec<u8> = item.try_get_by_index(0)?;
        let sequence: i64 = item.try_get_by_index(1)?;
        let query = Query::delete()
            .from_table(Alias::new("sync_journal"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("generation")).eq(generation))
            .and_where(Expr::col(Alias::new("sequence")).eq(sequence))
            .to_owned();
        removed += tx.execute(&query).await?.rows_affected();
    }
    let dead = || {
        Condition::any()
            .add(
                Condition::all()
                    .add(Expr::col(Alias::new("live_until")).is_not_null())
                    .add(Expr::col(Alias::new("live_until")).lte(state_floor)),
            )
            .add(
                Condition::all()
                    .add(Expr::col(Alias::new("live_from")).is_null())
                    .add(Expr::col(Alias::new("retention_seq")).lte(state_floor)),
            )
    };
    if removed < max_rows {
        let query = Query::select()
            .columns(["collection", "writer", "counter"].map(Alias::new))
            .from(Alias::new("sync_candidates"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
            .cond_where(dead())
            .limit(max_rows - removed)
            .to_owned();
        for item in tx.query_all(&query).await? {
            let collection: Vec<u8> = item.try_get_by_index(0)?;
            let writer: Vec<u8> = item.try_get_by_index(1)?;
            let counter: i64 = item.try_get_by_index(2)?;
            let query = Query::delete()
                .from_table(Alias::new("sync_candidates"))
                .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                .and_where(Expr::col(Alias::new("collection")).eq(collection))
                .and_where(Expr::col(Alias::new("writer")).eq(writer))
                .and_where(Expr::col(Alias::new("counter")).eq(counter))
                .to_owned();
            removed += tx.execute(&query).await?.rows_affected();
        }
    }
    if removed < max_rows {
        let query = Query::select()
            .columns(["collection", "state_seq", "ordinal"].map(Alias::new))
            .from(Alias::new("sync_context_ranges"))
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("basis_generation")).eq(nil.clone()))
            .and_where(Expr::col(Alias::new("state_seq")).lte(state_floor))
            .limit(max_rows - removed)
            .to_owned();
        for item in tx.query_all(&query).await? {
            let collection: Vec<u8> = item.try_get_by_index(0)?;
            let state: i64 = item.try_get_by_index(1)?;
            let ordinal: i64 = item.try_get_by_index(2)?;
            let query = Query::delete()
                .from_table(Alias::new("sync_context_ranges"))
                .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                .and_where(Expr::col(Alias::new("collection")).eq(collection))
                .and_where(Expr::col(Alias::new("basis_generation")).eq(nil.clone()))
                .and_where(Expr::col(Alias::new("state_seq")).eq(state))
                .and_where(Expr::col(Alias::new("ordinal")).eq(ordinal))
                .to_owned();
            removed += tx.execute(&query).await?.rows_affected();
        }
    }
    // A full page may have exhausted the last rows. A subsequent empty step is
    // safe and avoids an unbounded count or cross-table scan here.
    Ok(PruneStep {
        removed,
        more: removed == max_rows,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaselineAdvance {
    Incomplete { cursor: protocol::WorkCursor },
    Pinned { cursor: protocol::WorkCursor },
    Advanced { state_seq: u64 },
}

/// Build in bounded transactions, then switch the floor and baseline pointers
/// together. History reclamation is a separate bounded operation.
pub async fn advance_baseline_on<M: SyncModel>(
    tx: &DatabaseTransaction,
    source: protocol::StoreIdentity,
    request: Uuid,
    now_ms: i64,
    limits: protocol::WorkLimits,
    resume: Option<protocol::WorkCursor>,
) -> Result<BaselineAdvance> {
    ensure!(
        limits.max_rows_scanned >= 16 && limits.max_mutations >= 16,
        protocol::SyncError::LimitTooSmall { required: 16 }
    );
    lock_on(tx, source.namespace).await?;
    let proof = match crate::causal_source::prepare_baseline_on::<M>(
        tx,
        source,
        request,
        now_ms,
        limits,
        resume.clone(),
    )
    .await?
    {
        protocol::WorkOutcome::Incomplete { cursor } => {
            return Ok(BaselineAdvance::Incomplete { cursor });
        }
        protocol::WorkOutcome::Ready(proof) => proof,
    };
    let evidence = proof.source();
    let ns = evidence.namespace();
    let query = Query::select()
        .columns(
            [
                "model",
                "schema",
                "generation",
                "head",
                "state_seq",
                "history_floor",
                "floor_state_seq",
                "baseline_generation",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_meta"))
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .to_owned();
    let meta = Meta::find_by_statement(tx.get_database_backend().build(&query))
        .one(tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing namespace"))?;
    let scope = Scope::<M> {
        collections: M::collections().into_iter().collect(),
    };
    ensure!(
        meta.model == M::model_id()
            && meta.schema == M::schema_id()
            && meta.generation == evidence.generation().as_bytes()
            && meta.floor_state_seq == evidence.floor()
            && meta.baseline_generation == evidence.basis().as_bytes()
            && evidence.target() >= meta.floor_state_seq
            && evidence.target() <= meta.state_seq
            && evidence.head() >= meta.history_floor
            && evidence.head() <= meta.head
            && evidence.scope() == protocol::scope_id(&scope)?
            && proof.segments() >= scope.collections.len() as u64
            && !proof.generation().is_nil()
            && proof.generation() != evidence.basis(),
        protocol::SyncError::StaleWork
    );
    if let Some((state, head)) =
        crate::causal_leases::minimum_pins_except_on(tx, ns, Some(evidence.lease())).await?
        && (state < evidence.target() || head < evidence.head())
    {
        return Ok(BaselineAdvance::Pinned {
            cursor: resume.ok_or(protocol::SyncError::StaleWork)?,
        });
    }
    let update = Query::update()
        .table(Alias::new("sync_meta"))
        .value(Alias::new("history_floor"), evidence.head())
        .value(Alias::new("floor_anchor"), evidence.anchor().to_vec())
        .value(Alias::new("floor_state_seq"), evidence.target())
        .value(
            Alias::new("baseline_generation"),
            proof.generation().as_bytes().to_vec(),
        )
        .value(
            Alias::new("baseline_manifest"),
            evidence.root().manifest_digest.to_vec(),
        )
        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
        .and_where(Expr::col(Alias::new("floor_state_seq")).eq(evidence.floor()))
        .and_where(
            Expr::col(Alias::new("baseline_generation")).eq(evidence.basis().as_bytes().to_vec()),
        )
        .to_owned();
    ensure!(
        tx.execute(&update).await?.rows_affected() == 1,
        protocol::SyncError::StaleWork
    );
    crate::causal_leases::revoke_on(tx, ns, evidence.lease()).await?;
    Ok(BaselineAdvance::Advanced {
        state_seq: evidence.target().try_into()?,
    })
}
