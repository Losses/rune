//! Bounded, restartable exact-context construction primitives.
//!
//! The enclosing builder owns the lease checkpoint. Each returned cursor and
//! any output it consumes must be committed in the same caller transaction.
use crate::causal_source::{CoveringScan, covering_step_on};
use anyhow::{Result, ensure};
#[cfg(test)]
use sea_orm::sea_query::Condition;
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, ExprTrait,
    sea_query::{Alias, Expr, Order, Query},
};
use serde::{Deserialize, Serialize};
use sync::protocol::{self, WorkLimits};
use uuid::Uuid;

/// A completion token produced only after the builder verifies all stored
/// fragments against its deterministic canonical stream. Its fields are not
/// constructible by callers outside this module.
pub(crate) struct ContextCompletionProof {
    namespace: Uuid,
    lease: Uuid,
    retained_root: protocol::Digest,
    manifest: protocol::ManifestRef,
    context_digest: protocol::Digest,
}
impl ContextCompletionProof {
    pub(crate) fn namespace(&self) -> Uuid {
        self.namespace
    }
    pub(crate) fn lease(&self) -> Uuid {
        self.lease
    }
    pub(crate) fn retained_root(&self) -> protocol::Digest {
        self.retained_root
    }
    pub(crate) fn manifest(&self) -> &protocol::ManifestRef {
        &self.manifest
    }
    pub(crate) fn context_digest(&self) -> protocol::Digest {
        self.context_digest
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnionInterval {
    pub writer: Uuid,
    pub first: u64,
    pub last: u64,
}
/// Durable scan state bound to a captured lease and exact key interval.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnionCursor {
    version: u8,
    namespace: Uuid,
    lease: Uuid,
    floor: i64,
    basis: Vec<u8>,
    state: i64,
    replica_generation: Vec<u8>,
    retained_root: Option<Vec<u8>>,
    collection: Vec<u8>,
    lower: Vec<u8>,
    upper: Vec<u8>,
    coverage: CoveringScan,
    after: Option<UnionInterval>,
    checking: Option<[u8; 32]>,
    best: Option<UnionInterval>,
    pending: Option<UnionInterval>,
    done: bool,
}
#[cfg(test)]
#[derive(Clone, Debug)]
pub enum UnionScanStep {
    Progress(UnionCursor),
    Interval(UnionCursor, UnionInterval),
    Done(UnionCursor),
}
#[cfg(test)]
fn bounded_step(step: UnionScanStep, max_bytes: u64) -> Result<UnionScanStep> {
    let (cursor, interval) = match &step {
        UnionScanStep::Progress(cursor) | UnionScanStep::Done(cursor) => (cursor, None),
        UnionScanStep::Interval(cursor, interval) => (cursor, Some(interval)),
    };
    let bytes = (serde_json::to_vec(&(cursor, interval))?.len() as u64)
        .checked_add(256)
        .ok_or(protocol::SyncError::LogicalLimit)?;
    if bytes > max_bytes {
        return Err(protocol::SyncError::LimitTooSmall { required: bytes }.into());
    }
    Ok(step)
}
impl UnionCursor {
    pub async fn begin_on(
        tx: &DatabaseTransaction,
        namespace: Uuid,
        lease: Uuid,
        now_ms: i64,
        collection: Vec<u8>,
        lower: Vec<u8>,
        upper: Vec<u8>,
    ) -> Result<Self> {
        ensure!(
            !namespace.is_nil()
                && !lease.is_nil()
                && !collection.is_empty()
                && collection.len() <= 128,
            "invalid union identity"
        );
        let pin = crate::causal_leases::building_on(tx, namespace, lease, now_ms).await?;
        let floor = pin
            .basis_state_seq
            .ok_or_else(|| anyhow::anyhow!("missing union basis"))?;
        let basis = pin
            .basis_generation
            .ok_or_else(|| anyhow::anyhow!("missing union generation"))?;
        ensure!(
            basis.len() == 16 && pin.generation.len() == 16,
            "invalid union basis"
        );
        let coverage = CoveringScan::new(lower.clone(), upper.clone())?;
        Ok(Self {
            version: 1,
            namespace,
            lease,
            floor,
            basis,
            state: pin.state_seq,
            replica_generation: pin.generation,
            retained_root: pin.retained_root,
            collection,
            lower,
            upper,
            coverage,
            after: None,
            checking: None,
            best: None,
            pending: None,
            done: false,
        })
    }
}
/// Advance one captured assertion or one interval row. Each selection pass sees
/// only manifests referenced at F..S, so concurrent later writes cannot extend
/// it. This constant-memory merge is bounded per step but may require multiple
/// passes over the retained assertions.
#[cfg(test)]
pub async fn union_scan_step_on(
    tx: &DatabaseTransaction,
    mut cursor: UnionCursor,
    now_ms: i64,
    limits: WorkLimits,
) -> Result<UnionScanStep> {
    ensure!(
        cursor.version == 1
            && !cursor.namespace.is_nil()
            && !cursor.lease.is_nil()
            && cursor.basis.len() == 16
            && cursor.replica_generation.len() == 16
            && cursor.floor >= 0
            && cursor.state >= cursor.floor
            && !cursor.collection.is_empty()
            && cursor.collection.len() <= 128
            && crate::causal_compaction::valid_bound(&cursor.lower)
            && crate::causal_compaction::valid_bound(&cursor.upper)
            && cursor.lower < cursor.upper,
        "invalid union cursor"
    );
    if limits.max_rows_scanned == 0 || limits.max_requests == 0 || limits.max_mutations == 0 {
        return Err(protocol::SyncError::LimitTooSmall { required: 1 }.into());
    }
    if limits.max_bytes < 256 {
        return Err(protocol::SyncError::LimitTooSmall { required: 256 }.into());
    }
    let pin = crate::causal_leases::building_on(tx, cursor.namespace, cursor.lease, now_ms).await?;
    ensure!(
        pin.basis_state_seq == Some(cursor.floor)
            && pin.basis_generation.as_deref() == Some(cursor.basis.as_slice())
            && pin.state_seq == cursor.state
            && pin.generation == cursor.replica_generation,
        "stale union lease"
    );
    ensure!(
        pin.retained_root == cursor.retained_root,
        protocol::SyncError::StaleWork
    );
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
        .and_where(Expr::col(Alias::new("namespace")).eq(cursor.namespace.as_bytes().to_vec()))
        .to_owned();
    let meta = tx
        .query_one(&meta)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing causal namespace"))?;
    ensure!(
        meta.try_get_by_index::<i64>(0)? == cursor.floor
            && meta.try_get_by_index::<Vec<u8>>(1)? == cursor.basis
            && meta.try_get_by_index::<Vec<u8>>(2)? == cursor.replica_generation
            && meta.try_get_by_index::<i64>(3)? >= cursor.state,
        protocol::SyncError::StaleWork
    );
    if cursor.done {
        return bounded_step(UnionScanStep::Done(cursor), limits.max_bytes);
    }
    if let Some(digest) = cursor.checking.take() {
        let mut q = Query::select();
        q.columns(["writer", "first", "last"].map(Alias::new))
            .from(Alias::new("sync_context_intervals"))
            .and_where(Expr::col(Alias::new("namespace")).eq(cursor.namespace.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("context_manifest")).eq(digest.to_vec()));
        if let Some(after) = &cursor.after {
            let writer = after.writer.as_bytes().to_vec();
            q.cond_where(
                Condition::any()
                    .add(Expr::col(Alias::new("writer")).gt(writer.clone()))
                    .add(
                        Condition::all()
                            .add(Expr::col(Alias::new("writer")).eq(writer.clone()))
                            .add(Expr::col(Alias::new("first")).gt(i64::try_from(after.first)?)),
                    )
                    .add(
                        Condition::all()
                            .add(Expr::col(Alias::new("writer")).eq(writer))
                            .add(Expr::col(Alias::new("first")).eq(i64::try_from(after.first)?))
                            .add(Expr::col(Alias::new("last")).gt(i64::try_from(after.last)?)),
                    ),
            );
        }
        q.order_by(Alias::new("writer"), Order::Asc)
            .order_by(Alias::new("first"), Order::Asc)
            .order_by(Alias::new("last"), Order::Asc)
            .limit(1);
        if let Some(row) = tx.query_one(&q.to_owned()).await? {
            let writer = Uuid::from_slice(&row.try_get_by_index::<Vec<u8>>(0)?)?;
            let first: i64 = row.try_get_by_index(1)?;
            let last: i64 = row.try_get_by_index(2)?;
            ensure!(
                !writer.is_nil() && first > 0 && last >= first,
                "invalid retained union interval"
            );
            let value = UnionInterval {
                writer,
                first: u64::try_from(first)?,
                last: u64::try_from(last)?,
            };
            if cursor.best.as_ref().is_none_or(|old| {
                (value.writer, value.first, value.last) < (old.writer, old.first, old.last)
            }) {
                cursor.best = Some(value);
            }
        }
        return bounded_step(UnionScanStep::Progress(cursor), limits.max_bytes);
    }
    if !cursor.coverage.done {
        if let Some(digest) = covering_step_on(
            tx,
            cursor.namespace,
            &cursor.collection,
            crate::causal_source::CapturedBasis {
                floor: cursor.floor,
                generation: &cursor.basis,
                state_seq: cursor.state,
            },
            &mut cursor.coverage,
            limits,
        )
        .await?
        {
            let reference =
                crate::causal_manifests::reference_on(tx, cursor.namespace, digest).await?;
            ensure!(
                reference.kind == protocol::ManifestKind::Context,
                "invalid retained context kind"
            );
            cursor.checking = Some(digest);
        }
        return bounded_step(UnionScanStep::Progress(cursor), limits.max_bytes);
    }
    if let Some(interval) = cursor.best.take() {
        cursor.after = Some(interval.clone());
        cursor.coverage = CoveringScan::new(cursor.lower.clone(), cursor.upper.clone())?;
        if let Some(pending) = cursor.pending.take() {
            if pending.writer == interval.writer && interval.first <= pending.last.saturating_add(1)
            {
                cursor.pending = Some(UnionInterval {
                    last: std::cmp::max(pending.last, interval.last),
                    ..pending
                });
                return bounded_step(UnionScanStep::Progress(cursor), limits.max_bytes);
            }
            ensure!(
                (pending.writer, pending.first) < (interval.writer, interval.first),
                "unordered union intervals"
            );
            cursor.pending = Some(interval);
            return bounded_step(UnionScanStep::Interval(cursor, pending), limits.max_bytes);
        }
        cursor.pending = Some(interval);
        return bounded_step(UnionScanStep::Progress(cursor), limits.max_bytes);
    }
    cursor.done = true;
    bounded_step(
        match cursor.pending.take() {
            Some(interval) => UnionScanStep::Interval(cursor, interval),
            None => UnionScanStep::Done(cursor),
        },
        limits.max_bytes,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{causal_leases, causal_manifests, causal_schema, causal_store::CausalReplica};
    use sea_orm::{Database, TransactionTrait};
    use serde::{Deserialize, Serialize};
    use sync::protocol::{ObjectKey, SyncModel};
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Model;
    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    enum Collection {
        Items,
    }
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct Value(String);
    impl SyncModel for Model {
        type Collection = Collection;
        type Payload = Value;
        fn model_id() -> &'static str {
            "items"
        }
        fn schema_id() -> &'static str {
            "1"
        }
        fn collections() -> Vec<Collection> {
            vec![Collection::Items]
        }
        fn collection_tag(_: &Collection) -> &'static str {
            "items"
        }
        fn collection_from_tag(tag: &str) -> Option<Collection> {
            (tag == "items").then_some(Collection::Items)
        }
        fn validate(_: &ObjectKey<Self>, _: &Option<Value>) -> protocol::Result<()> {
            Ok(())
        }
    }
    #[tokio::test]
    async fn union_scan_resumes_one_row_at_a_time_and_preserves_holes() -> Result<()> {
        let path =
            std::env::temp_dir().join(format!("rune-context-union-{}.sqlite", Uuid::new_v4()));
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let db = Database::connect(&url).await?;
        causal_schema::install_fresh(&db).await?;
        let writer_db = Database::connect(&url).await?;
        let ns = Uuid::new_v4();
        let _store = CausalReplica::<Model>::create_empty(db.clone(), ns, Uuid::new_v4()).await?;
        let tx = db.begin().await?;
        let writer = Uuid::new_v4();
        for (state, intervals) in [
            (1_i64, vec![(1_u64, 2_u64), (5, 5)]),
            (2, vec![(1, 4), (8, 9)]),
        ] {
            let mut context = sync::causal::Context::default();
            for (first, last) in intervals {
                context.insert_interval(writer, first, last)?;
            }
            let exact = protocol::exact_context(&context);
            let body = protocol::canonical_context(&exact)?;
            let (manifest, parts) =
                causal_manifests::fragment_body(protocol::ManifestKind::Context, &body, 1, 1024)?;
            causal_manifests::publish_on(&tx, ns, &manifest, &parts, 1024, |bytes| {
                protocol::decode_context(bytes)?;
                Ok(vec![])
            })
            .await?;
            let mut budget = 10;
            crate::causal_range::index_context_on(
                &tx,
                ns,
                manifest.manifest_digest,
                &context,
                &mut budget,
            )
            .await?;
            let query = Query::insert()
                .into_table(Alias::new("sync_context_ranges"))
                .columns(
                    [
                        "namespace",
                        "collection",
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
                    b"items".to_vec().into(),
                    state.into(),
                    0_i64.into(),
                    vec![0_u8].into(),
                    vec![2_u8].into(),
                    manifest.manifest_digest.to_vec().into(),
                ])
                .to_owned();
            tx.execute(&query).await?;
        }
        let query = Query::update()
            .table(Alias::new("sync_meta"))
            .value(Alias::new("state_seq"), 2_i64)
            .to_owned();
        tx.execute(&query).await?;
        let lease = Uuid::new_v4();
        causal_leases::create_build_on(
            &tx,
            crate::causal_leases::BuildLeaseRequest {
                namespace: ns,
                id: lease,
                scope: vec![1],
                now_ms: 0,
                expires_at_ms: 100,
                max_active: 10,
                max_ttl_ms: 1000,
            },
        )
        .await?;
        let mut cursor =
            UnionCursor::begin_on(&tx, ns, lease, 1, b"items".to_vec(), vec![0], vec![2]).await?;
        tx.commit().await?;
        let limits = WorkLimits {
            max_requests: 1,
            max_bytes: 4096,
            max_depth: 64,
            max_rows_scanned: 1,
            max_mutations: 1,
        };
        let mut result = Vec::new();
        for step_number in 0..100 {
            if step_number == 4 {
                let tx = writer_db.begin().await?;
                let mut later = sync::causal::Context::default();
                later.insert_interval(Uuid::new_v4(), 1, 1)?;
                let exact = protocol::exact_context(&later);
                let body = protocol::canonical_context(&exact)?;
                let (manifest, parts) = causal_manifests::fragment_body(
                    protocol::ManifestKind::Context,
                    &body,
                    1,
                    1024,
                )?;
                causal_manifests::publish_on(&tx, ns, &manifest, &parts, 1024, |body| {
                    protocol::decode_context(body)?;
                    Ok(vec![])
                })
                .await?;
                let mut budget = 10;
                crate::causal_range::index_context_on(
                    &tx,
                    ns,
                    manifest.manifest_digest,
                    &later,
                    &mut budget,
                )
                .await?;
                let insert = Query::insert()
                    .into_table(Alias::new("sync_context_ranges"))
                    .columns(
                        [
                            "namespace",
                            "collection",
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
                        b"items".to_vec().into(),
                        3_i64.into(),
                        0_i64.into(),
                        vec![0_u8].into(),
                        vec![2_u8].into(),
                        manifest.manifest_digest.to_vec().into(),
                    ])
                    .to_owned();
                tx.execute(&insert).await?;
                tx.execute(
                    &Query::update()
                        .table(Alias::new("sync_meta"))
                        .value(Alias::new("state_seq"), 3_i64)
                        .to_owned(),
                )
                .await?;
                tx.commit().await?;
            }
            let tx = db.begin().await?;
            let step = union_scan_step_on(&tx, cursor, 1, limits).await?;
            tx.commit().await?;
            let done = matches!(&step, UnionScanStep::Done(_));
            cursor = match step {
                UnionScanStep::Progress(cursor) | UnionScanStep::Done(cursor) => cursor,
                UnionScanStep::Interval(cursor, interval) => {
                    result.push(interval);
                    cursor
                }
            };
            cursor = serde_json::from_slice(&serde_json::to_vec(&cursor)?)?;
            if done {
                break;
            }
        }
        assert!(cursor.done);
        assert_eq!(
            result,
            vec![
                UnionInterval {
                    writer,
                    first: 1,
                    last: 5
                },
                UnionInterval {
                    writer,
                    first: 8,
                    last: 9
                }
            ]
        );
        let tx = db.begin().await?;
        let mut build =
            ContextBuildCursor::begin_on(&tx, ns, lease, 1, b"items".to_vec(), vec![0], vec![2])
                .await?;
        tx.commit().await?;
        let build_limits = WorkLimits {
            max_requests: 1,
            max_bytes: 65536,
            max_depth: 64,
            max_rows_scanned: 1,
            max_mutations: 32,
        };
        // A session accepts only its validated captured basis and cannot be
        // reused after a root-changing step, even before the caller commits.
        {
            let tx = db.begin().await?;
            let mut session = ContextBuildSession::begin(&tx, &build, 1).await?;
            let mut stale = build.clone();
            stale.scan.state += 1;
            assert!(session.step(stale, build_limits).await.is_err());
            assert!(matches!(
                session.step(build.clone(), build_limits).await?,
                ContextBuildStep::Progress { root: Some(_), .. }
            ));
            assert!(session.step(build.clone(), build_limits).await.is_err());
            tx.rollback().await?;
        }
        let mut completed = None;
        for _ in 0..1000 {
            let tx = db.begin().await?;
            match context_build_step_on(&tx, build, 1, build_limits).await? {
                ContextBuildStep::Progress { cursor, root } => {
                    if let Some(root) = root {
                        causal_leases::pin_root_on(&tx, ns, lease, root, 1).await?;
                    }
                    tx.commit().await?;
                    build = serde_json::from_slice(&serde_json::to_vec(&cursor)?)?;
                }
                ContextBuildStep::Ready(reference) => {
                    tx.commit().await?;
                    completed = Some(reference);
                    break;
                }
            }
        }
        let reference =
            completed.ok_or_else(|| anyhow::anyhow!("context builder did not finish"))?;
        let tx = db.begin().await?;
        let (_, bytes) =
            causal_manifests::read_on(&tx, ns, reference.manifest.manifest_digest, 65536, 100)
                .await?;
        let exact = protocol::decode_context(&bytes)?;
        assert_eq!(protocol::context_digest(&exact)?, reference.digest);
        assert_eq!(exact.writers.len(), 1);
        assert_eq!(exact.writers[0].prefix, 5);
        assert_eq!(
            exact.writers[0].exceptions,
            vec![protocol::CounterInterval {
                first: 8,
                last_inclusive: 9
            }]
        );
        tx.commit().await?;
        // Build a second context large enough to span several reserved chunks.
        // Every scan and verification step still commits separately.
        let extra_writer = Uuid::new_v4();
        let mut extra = sync::causal::Context::default();
        for n in 0..80_u64 {
            extra.insert_interval(extra_writer, 3 * n + 2, 3 * n + 2)?;
        }
        let exact_extra = protocol::exact_context(&extra);
        let body = protocol::canonical_context(&exact_extra)?;
        let (manifest, parts) =
            causal_manifests::fragment_body(protocol::ManifestKind::Context, &body, 1, 1024)?;
        let tx = db.begin().await?;
        causal_manifests::publish_on(&tx, ns, &manifest, &parts, 65536, |bytes| {
            protocol::decode_context(bytes)?;
            Ok(vec![])
        })
        .await?;
        let mut budget = 100;
        crate::causal_range::index_context_on(
            &tx,
            ns,
            manifest.manifest_digest,
            &extra,
            &mut budget,
        )
        .await?;
        let insert = Query::insert()
            .into_table(Alias::new("sync_context_ranges"))
            .columns(
                [
                    "namespace",
                    "collection",
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
                b"items".to_vec().into(),
                4_i64.into(),
                0_i64.into(),
                vec![0_u8].into(),
                vec![2_u8].into(),
                manifest.manifest_digest.to_vec().into(),
            ])
            .to_owned();
        tx.execute(&insert).await?;
        tx.execute(
            &Query::update()
                .table(Alias::new("sync_meta"))
                .value(Alias::new("state_seq"), 4_i64)
                .to_owned(),
        )
        .await?;
        let second_lease = Uuid::new_v4();
        causal_leases::create_build_on(
            &tx,
            crate::causal_leases::BuildLeaseRequest {
                namespace: ns,
                id: second_lease,
                scope: vec![1],
                now_ms: 0,
                expires_at_ms: 100,
                max_active: 10,
                max_ttl_ms: 1000,
            },
        )
        .await?;
        let mut build = ContextBuildCursor::begin_on(
            &tx,
            ns,
            second_lease,
            1,
            b"items".to_vec(),
            vec![0],
            vec![2],
        )
        .await?;
        tx.commit().await?;
        let mut completed = None;
        for _ in 0..5000 {
            let tx = db.begin().await?;
            match context_build_step_on(&tx, build, 1, build_limits).await? {
                ContextBuildStep::Progress { cursor, root } => {
                    if let Some(root) = root {
                        causal_leases::pin_root_on(&tx, ns, second_lease, root, 1).await?;
                    }
                    tx.commit().await?;
                    build = serde_json::from_slice(&serde_json::to_vec(&cursor)?)?;
                }
                ContextBuildStep::Ready(reference) => {
                    tx.commit().await?;
                    completed = Some(reference);
                    break;
                }
            }
        }
        let reference =
            completed.ok_or_else(|| anyhow::anyhow!("multi-fragment context did not finish"))?;
        assert!(reference.manifest.fragment_count > 1);
        let tx = db.begin().await?;
        let (_, bytes) =
            causal_manifests::read_on(&tx, ns, reference.manifest.manifest_digest, 65536, 100)
                .await?;
        let exact = protocol::decode_context(&bytes)?;
        assert_eq!(protocol::context_digest(&exact)?, reference.digest);
        assert_eq!(
            exact
                .writers
                .iter()
                .find(|entry| entry.writer_incarnation == extra_writer)
                .unwrap()
                .exceptions
                .len(),
            80
        );
        tx.commit().await?;
        drop(_store);
        db.close().await?;
        writer_db.close().await?;
        std::fs::remove_file(path)?;
        Ok(())
    }
}

/// Incrementally emits the exact canonical context JSON for an ordered,
/// coalesced interval stream. Each call emits a bounded piece and retains only
/// the current writer and counters; callers can serialize it between steps.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalContextEncoder {
    started: bool,
    finished: bool,
    current: Option<EncoderWriter>,
    writers: u64,
    exceptions: u64,
    bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EncoderWriter {
    writer: Uuid,
    prefix: u64,
    last: u64,
    exception_count: u64,
}
impl CanonicalContextEncoder {
    pub fn writer_count(&self) -> u64 {
        self.writers
    }
    pub fn exception_count(&self) -> u64 {
        self.exceptions
    }
    pub fn encoded_bytes(&self) -> u64 {
        self.bytes
    }
    fn account(&mut self, bytes: &[u8]) -> Result<()> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or(protocol::SyncError::LogicalLimit)?;
        ensure!(
            self.bytes <= protocol::MAX_LOGICAL_BYTES,
            protocol::SyncError::LogicalLimit
        );
        Ok(())
    }
    pub fn push(&mut self, interval: &UnionInterval) -> Result<Vec<u8>> {
        ensure!(
            !self.finished
                && !interval.writer.is_nil()
                && interval.first > 0
                && interval.first <= interval.last
                && interval.last <= protocol::MAX_NUMBER,
            "invalid canonical context interval"
        );
        let mut output = Vec::new();
        if !self.started {
            output.extend_from_slice(b"{\"writers\":[");
            self.started = true;
        }
        if let Some(current) = &self.current {
            ensure!(
                current.writer <= interval.writer,
                "unordered context writer"
            );
            if current.writer == interval.writer {
                ensure!(
                    interval.first > current.last.saturating_add(1),
                    "context intervals not coalesced"
                );
            } else {
                output.extend_from_slice(
                    format!(
                        "],\"prefix\":{},\"writer_incarnation\":\"{}\"}},",
                        current.prefix, current.writer
                    )
                    .as_bytes(),
                );
                self.current = None;
            }
        }
        if self.current.is_none() {
            self.writers = self
                .writers
                .checked_add(1)
                .ok_or(protocol::SyncError::LogicalLimit)?;
            ensure!(
                self.writers <= protocol::MAX_LOGICAL_ITEMS,
                protocol::SyncError::LogicalLimit
            );
            output.extend_from_slice(b"{\"exceptions\":[");
            self.current = Some(EncoderWriter {
                writer: interval.writer,
                prefix: if interval.first == 1 {
                    interval.last
                } else {
                    0
                },
                last: interval.last,
                exception_count: 0,
            });
            if interval.first == 1 {
                self.account(&output)?;
                return Ok(output);
            }
        }
        let current = self
            .current
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("missing context writer"))?;
        if current.exception_count > 0 {
            output.push(b',');
        }
        output.extend_from_slice(
            format!(
                "{{\"first\":{},\"last_inclusive\":{}}}",
                interval.first, interval.last
            )
            .as_bytes(),
        );
        current.exception_count += 1;
        current.last = interval.last;
        self.exceptions = self
            .exceptions
            .checked_add(1)
            .ok_or(protocol::SyncError::LogicalLimit)?;
        ensure!(
            self.exceptions <= protocol::MAX_LOGICAL_ITEMS,
            protocol::SyncError::LogicalLimit
        );
        self.account(&output)?;
        Ok(output)
    }
    pub fn finish(&mut self) -> Result<Vec<u8>> {
        ensure!(!self.finished, "context encoder already finished");
        let mut output = Vec::new();
        if !self.started {
            output.extend_from_slice(b"{\"writers\":[");
            self.started = true;
        }
        if let Some(current) = self.current.take() {
            output.extend_from_slice(
                format!(
                    "],\"prefix\":{},\"writer_incarnation\":\"{}\"}}",
                    current.prefix, current.writer
                )
                .as_bytes(),
            );
        }
        output.extend_from_slice(b"]}");
        self.finished = true;
        self.account(&output)?;
        Ok(output)
    }
}

#[cfg(test)]
mod encoder_tests {
    use super::*;
    #[test]
    fn streaming_context_matches_canonical_codec_across_checkpoints() -> Result<()> {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let intervals = [
            UnionInterval {
                writer: a,
                first: 1,
                last: 3,
            },
            UnionInterval {
                writer: a,
                first: 6,
                last: 9,
            },
            UnionInterval {
                writer: b,
                first: 4,
                last: 4,
            },
        ];
        let mut exact = sync::causal::Context::default();
        let mut encoder = CanonicalContextEncoder::default();
        let mut bytes = Vec::new();
        for interval in &intervals {
            exact.insert_interval(interval.writer, interval.first, interval.last)?;
            bytes.extend(encoder.push(interval)?);
            encoder = serde_json::from_slice(&serde_json::to_vec(&encoder)?)?;
        }
        bytes.extend(encoder.finish()?);
        assert_eq!(
            bytes,
            protocol::canonical_context(&protocol::exact_context(&exact))?
        );
        assert_eq!(encoder.writer_count(), 2);
        assert_eq!(encoder.exception_count(), 2);
        assert_eq!(encoder.encoded_bytes(), bytes.len() as u64);
        let mut empty = CanonicalContextEncoder::default();
        assert_eq!(
            empty.finish()?,
            protocol::canonical_context(&protocol::ExactContext { writers: vec![] })?
        );
        Ok(())
    }
}

const CONTEXT_FRAGMENT_BYTES: usize = 1024;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BuildPhase {
    ScratchCreate,
    Union,
    Count,
    PrepareHash,
    Hash,
    Reserve,
    PrepareEmit,
    Emit,
    VerifyHeader,
    VerifyBody,
    Promote,
}
/// Durable context construction state, embedded in the caller's checkpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContextBuildCursor {
    version: u8,
    scan: UnionCursor,
    scratch_id: Uuid,
    scratch_root: Option<protocol::Digest>,
    covering: CoveringScan,
    context: Option<protocol::Digest>,
    context_after: Option<crate::causal_source::IndexedInterval>,
    merge_pending: Option<UnionInterval>,
    merge_predecessor: bool,
    scratch_after: Option<crate::causal_source::IndexedInterval>,
    index_pending: Option<UnionInterval>,
    phase: BuildPhase,
    encoder: CanonicalContextEncoder,
    expected_bytes: u64,
    expected_writers: u64,
    expected_exceptions: u64,
    context_hash: Option<protocol::PortableHasher>,
    content_hash: Option<protocol::PortableHasher>,
    context_digest: Option<protocol::Digest>,
    manifest: Option<protocol::ManifestRef>,
    work_root: Option<protocol::Digest>,
    pending: Vec<u8>,
    emitted_offset: u64,
    emitted_ordinal: u32,
    stream_finished: bool,
    verify_ordinal: u32,
    verify_offset: u64,
    verify_header: Option<crate::causal_manifests::ReservedFragmentHeader>,
}
#[derive(Clone, Debug)]
pub(crate) enum ContextBuildStep {
    /// The caller must pin `root` when present and checkpoint `cursor` in this
    /// same transaction before reporting success.
    Progress {
        cursor: Box<ContextBuildCursor>,
        root: Option<protocol::Digest>,
    },
    /// The Context manifest and all indexes have been verified and promoted in
    /// this transaction; commit before exposing this reference.
    Ready(protocol::ContextRef),
}
fn tagged_hasher(tag: &[u8], body_len: u64) -> Result<protocol::PortableHasher> {
    let mut hash = protocol::PortableHasher::new();
    hash.update(&u32::try_from(tag.len())?.to_be_bytes())?;
    hash.update(tag)?;
    hash.update(&u32::try_from(body_len)?.to_be_bytes())?;
    Ok(hash)
}
fn charge_mutations(limits: WorkLimits, required: u64) -> Result<()> {
    if limits.max_mutations < required {
        return Err(protocol::SyncError::LimitTooSmall { required }.into());
    }
    Ok(())
}
fn bounded_build(
    cursor: ContextBuildCursor,
    root: Option<protocol::Digest>,
    limits: WorkLimits,
) -> Result<ContextBuildStep> {
    let required = (serde_json::to_vec(&cursor)?.len() as u64)
        .checked_add(4096)
        .ok_or(protocol::SyncError::LogicalLimit)?;
    if required > limits.max_bytes {
        return Err(protocol::SyncError::LimitTooSmall { required }.into());
    }
    Ok(ContextBuildStep::Progress {
        cursor: Box::new(cursor),
        root,
    })
}
impl ContextBuildCursor {
    /// Maximum content/index rows examined by the next step. Fixed-size lease
    /// and manifest identity checks are accounted separately by the caller.
    pub fn next_row_bound(&self) -> u64 {
        match self.phase {
            BuildPhase::ScratchCreate
            | BuildPhase::PrepareHash
            | BuildPhase::PrepareEmit
            | BuildPhase::Reserve
            | BuildPhase::Promote => 0,
            BuildPhase::Emit if self.stream_finished && self.pending.is_empty() => 0,
            _ => 1,
        }
    }
    fn fragment_bytes(&self) -> usize {
        self.manifest
            .as_ref()
            .map_or(CONTEXT_FRAGMENT_BYTES, |manifest| {
                usize::try_from(
                    manifest
                        .encoded_bytes
                        .div_ceil(u64::from(manifest.fragment_count)),
                )
                .unwrap_or(CONTEXT_FRAGMENT_BYTES)
            })
    }
    /// Conservative bound for the next step's own encoded state and row/body
    /// traffic. The enclosing checkpoint and optional pin have separate costs.
    pub fn next_encoded_byte_bound(&self) -> Result<u64> {
        (serde_json::to_vec(self)?.len() as u64)
            .checked_add(4096)
            .ok_or_else(|| protocol::SyncError::LogicalLimit.into())
    }
    /// Reserve enough space for a larger serialized cursor after one step.
    /// This is a preflight reserve, not a charge for work actually performed.
    pub fn next_output_growth_bound(&self) -> u64 {
        // Each of two hash checkpoints contains at most one 1024-byte chunk,
        // a 15-byte header, and 64 32-byte tree nodes; JSON encodes them as
        // hexadecimal. The pending JSON byte array can gain one fragment and
        // one encoded interval, at most four characters per byte. The final
        // allowance covers the fixed-size interval, manifest and header fields.
        const HASH_CHECKPOINT: u64 = 2 * (15 + 1024 + 64 * 32);
        const PENDING_ARRAY: u64 = 4 * 2 * CONTEXT_FRAGMENT_BYTES as u64;
        const FIXED_FIELDS: u64 = 4096;
        2 * HASH_CHECKPOINT + PENDING_ARRAY + FIXED_FIELDS
    }
    /// Worst-case SQL writes in the next step, including namespace lock writes.
    /// The boolean indicates whether the caller must also pin a returned root.
    pub fn next_mutation_bound(&self) -> (u64, bool) {
        match self.phase {
            BuildPhase::ScratchCreate => (4, true),
            BuildPhase::Union => (2, false),
            BuildPhase::Reserve => (7, true),
            BuildPhase::PrepareHash
            | BuildPhase::PrepareEmit
            | BuildPhase::Count
            | BuildPhase::Hash
            | BuildPhase::VerifyHeader
            | BuildPhase::VerifyBody => (1, false),
            BuildPhase::Emit if self.index_pending.is_some() => (2, false),
            BuildPhase::Emit if self.stream_finished && self.pending.is_empty() => (0, false),
            BuildPhase::Emit
                if self.stream_finished || self.pending.len() >= self.fragment_bytes() =>
            {
                (4, false)
            }
            BuildPhase::Emit => (1, false),
            BuildPhase::Promote => (2, false),
        }
    }
    pub async fn begin_on(
        tx: &DatabaseTransaction,
        namespace: Uuid,
        lease: Uuid,
        now_ms: i64,
        collection: Vec<u8>,
        lower: Vec<u8>,
        upper: Vec<u8>,
    ) -> Result<Self> {
        let scan =
            UnionCursor::begin_on(tx, namespace, lease, now_ms, collection, lower, upper).await?;
        let covering = CoveringScan::new(scan.lower.clone(), scan.upper.clone())?;
        Ok(Self {
            version: 1,
            scan,
            scratch_id: Uuid::new_v4(),
            scratch_root: None,
            covering,
            context: None,
            context_after: None,
            merge_pending: None,
            merge_predecessor: true,
            scratch_after: None,
            index_pending: None,
            phase: BuildPhase::ScratchCreate,
            encoder: CanonicalContextEncoder::default(),
            expected_bytes: 0,
            expected_writers: 0,
            expected_exceptions: 0,
            context_hash: None,
            content_hash: None,
            context_digest: None,
            manifest: None,
            work_root: None,
            pending: Vec::new(),
            emitted_offset: 0,
            emitted_ordinal: 0,
            stream_finished: false,
            verify_ordinal: 0,
            verify_offset: 0,
            verify_header: None,
        })
    }
}
fn hash_piece(cursor: &mut ContextBuildCursor, bytes: &[u8]) -> Result<()> {
    cursor
        .context_hash
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("missing context hasher"))?
        .update(bytes)?;
    cursor
        .content_hash
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("missing content hasher"))?
        .update(bytes)?;
    Ok(())
}
fn check_encoder(cursor: &ContextBuildCursor) -> Result<()> {
    ensure!(
        cursor.encoder.encoded_bytes() == cursor.expected_bytes
            && cursor.encoder.writer_count() == cursor.expected_writers
            && cursor.encoder.exception_count() == cursor.expected_exceptions,
        "context input changed across passes"
    );
    Ok(())
}
async fn verify_build_pin(
    tx: &DatabaseTransaction,
    cursor: &ContextBuildCursor,
    now_ms: i64,
) -> Result<()> {
    let pin =
        crate::causal_leases::building_on(tx, cursor.scan.namespace, cursor.scan.lease, now_ms)
            .await?;
    let expected = cursor
        .work_root
        .or(cursor.scratch_root)
        .map(|value| value.to_vec())
        .or_else(|| cursor.scan.retained_root.clone());
    ensure!(
        pin.basis_state_seq == Some(cursor.scan.floor)
            && pin.basis_generation.as_deref() == Some(cursor.scan.basis.as_slice())
            && pin.state_seq == cursor.scan.state
            && pin.generation == cursor.scan.replica_generation
            && pin.retained_root == expected,
        protocol::SyncError::StaleWork
    );
    let q = Query::select()
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
        .and_where(Expr::col(Alias::new("namespace")).eq(cursor.scan.namespace.as_bytes().to_vec()))
        .to_owned();
    let meta = tx
        .query_one(&q)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing causal namespace"))?;
    ensure!(
        meta.try_get_by_index::<i64>(0)? == cursor.scan.floor
            && meta.try_get_by_index::<Vec<u8>>(1)? == cursor.scan.basis
            && meta.try_get_by_index::<Vec<u8>>(2)? == cursor.scan.replica_generation
            && meta.try_get_by_index::<i64>(3)? >= cursor.scan.state,
        protocol::SyncError::StaleWork
    );
    Ok(())
}

/// Validation of an immutable build basis for a single database transaction.
/// A returned root ends the session; its caller must pin it before beginning
/// another session. The transaction reference cannot be changed by callers.
pub(crate) struct ContextBuildSession<'a> {
    tx: &'a DatabaseTransaction,
    now_ms: i64,
    namespace: Uuid,
    lease: Uuid,
    floor: i64,
    basis: Vec<u8>,
    state: i64,
    generation: Vec<u8>,
    root: Option<Vec<u8>>,
    closed: bool,
}
impl<'a> ContextBuildSession<'a> {
    pub(crate) async fn begin(
        tx: &'a DatabaseTransaction,
        cursor: &ContextBuildCursor,
        now_ms: i64,
    ) -> Result<Self> {
        verify_build_pin(tx, cursor, now_ms).await?;
        Ok(Self {
            tx,
            now_ms,
            namespace: cursor.scan.namespace,
            lease: cursor.scan.lease,
            floor: cursor.scan.floor,
            basis: cursor.scan.basis.clone(),
            state: cursor.scan.state,
            generation: cursor.scan.replica_generation.clone(),
            root: cursor
                .work_root
                .or(cursor.scratch_root)
                .map(|v| v.to_vec())
                .or_else(|| cursor.scan.retained_root.clone()),
            closed: false,
        })
    }
    fn validate(&self, cursor: &ContextBuildCursor) -> Result<()> {
        let root = cursor
            .work_root
            .or(cursor.scratch_root)
            .map(|v| v.to_vec())
            .or_else(|| cursor.scan.retained_root.clone());
        ensure!(
            !self.closed
                && cursor.scan.namespace == self.namespace
                && cursor.scan.lease == self.lease
                && cursor.scan.floor == self.floor
                && cursor.scan.basis == self.basis
                && cursor.scan.state == self.state
                && cursor.scan.replica_generation == self.generation
                && root == self.root,
            protocol::SyncError::StaleWork
        );
        Ok(())
    }
    pub(crate) async fn step(
        &mut self,
        cursor: ContextBuildCursor,
        limits: WorkLimits,
    ) -> Result<ContextBuildStep> {
        self.validate(&cursor)?;
        let result = context_build_checked_step_on(self, cursor, limits).await?;
        if matches!(
            &result,
            ContextBuildStep::Ready(_) | ContextBuildStep::Progress { root: Some(_), .. }
        ) {
            self.closed = true;
        }
        Ok(result)
    }
}

/// Advance one bounded context-build operation. The caller owns the outer
/// lease checkpoint and must roll back on error. A nonempty root result must be
/// pinned and checkpointed atomically with this step, before its transaction
/// commits. This function never opens a transaction or performs network I/O.
#[cfg(test)]
pub(crate) async fn context_build_step_on(
    tx: &DatabaseTransaction,
    cursor: ContextBuildCursor,
    now_ms: i64,
    limits: WorkLimits,
) -> Result<ContextBuildStep> {
    ContextBuildSession::begin(tx, &cursor, now_ms)
        .await?
        .step(cursor, limits)
        .await
}

async fn context_build_checked_step_on(
    session: &ContextBuildSession<'_>,
    mut cursor: ContextBuildCursor,
    limits: WorkLimits,
) -> Result<ContextBuildStep> {
    let tx = session.tx;
    let now_ms = session.now_ms;
    ensure!(
        cursor.version == 1 && limits.max_rows_scanned > 0 && limits.max_requests > 0,
        "invalid context build cursor or limits"
    );
    let estimated = (serde_json::to_vec(&cursor)?.len() as u64)
        .checked_add(4096)
        .ok_or(protocol::SyncError::LogicalLimit)?;
    if limits.max_bytes < estimated {
        return Err(protocol::SyncError::LimitTooSmall {
            required: estimated,
        }
        .into());
    }
    match cursor.phase {
        BuildPhase::ScratchCreate => {
            charge_mutations(limits, 4)?;
            session.validate(&cursor)?;
            let previous = cursor
                .scan
                .retained_root
                .as_ref()
                .map(|value| {
                    <[u8; 32]>::try_from(value.as_slice())
                        .map_err(|_| anyhow::anyhow!("invalid prior work root"))
                })
                .transpose()?;
            let mut payload = b"scratch-context-union-v1".to_vec();
            payload.extend_from_slice(cursor.scratch_id.as_bytes());
            let root = crate::causal_manifests::publish_work_node_on(
                tx,
                cursor.scan.namespace,
                previous,
                None,
                &payload,
                limits.max_bytes,
                limits.max_mutations - 1,
            )
            .await?;
            cursor.scratch_root = Some(root);
            cursor.phase = BuildPhase::Union;
            bounded_build(cursor, Some(root), limits)
        }
        BuildPhase::Union => {
            charge_mutations(limits, 2)?;
            session.validate(&cursor)?;
            let scratch = cursor
                .scratch_root
                .ok_or_else(|| anyhow::anyhow!("missing scratch root"))?;
            if let Some(mut pending) = cursor.merge_pending.take() {
                let mut query = Query::select();
                query
                    .columns(["first", "last"].map(Alias::new))
                    .from(Alias::new("sync_context_intervals"))
                    .and_where(
                        Expr::col(Alias::new("namespace")).eq(cursor
                            .scan
                            .namespace
                            .as_bytes()
                            .to_vec()),
                    )
                    .and_where(Expr::col(Alias::new("context_manifest")).eq(scratch.to_vec()))
                    .and_where(
                        Expr::col(Alias::new("writer")).eq(pending.writer.as_bytes().to_vec()),
                    );
                if cursor.merge_predecessor {
                    query
                        .and_where(
                            Expr::col(Alias::new("first")).lte(i64::try_from(pending.first)?),
                        )
                        .order_by(Alias::new("first"), Order::Desc);
                } else {
                    query
                        .and_where(
                            Expr::col(Alias::new("first")).gte(i64::try_from(pending.first)?),
                        )
                        .order_by(Alias::new("first"), Order::Asc);
                }
                query.limit(1);
                let found = tx.query_one(&query.to_owned()).await?;
                if let Some(row) = found {
                    let first = u64::try_from(row.try_get_by_index::<i64>(0)?)?;
                    let last = u64::try_from(row.try_get_by_index::<i64>(1)?)?;
                    ensure!(
                        first > 0 && first <= last && last <= protocol::MAX_NUMBER,
                        "invalid scratch interval"
                    );
                    let overlaps = if cursor.merge_predecessor {
                        last.saturating_add(1) >= pending.first
                    } else {
                        first <= pending.last.saturating_add(1)
                    };
                    if overlaps {
                        if first <= pending.first && last >= pending.last {
                            cursor.merge_predecessor = true;
                            return bounded_build(cursor, None, limits);
                        }
                        let delete = Query::delete()
                            .from_table(Alias::new("sync_context_intervals"))
                            .and_where(
                                Expr::col(Alias::new("namespace")).eq(cursor
                                    .scan
                                    .namespace
                                    .as_bytes()
                                    .to_vec()),
                            )
                            .and_where(
                                Expr::col(Alias::new("context_manifest")).eq(scratch.to_vec()),
                            )
                            .and_where(
                                Expr::col(Alias::new("writer"))
                                    .eq(pending.writer.as_bytes().to_vec()),
                            )
                            .and_where(Expr::col(Alias::new("first")).eq(i64::try_from(first)?))
                            .to_owned();
                        ensure!(
                            tx.execute(&delete).await?.rows_affected() == 1,
                            "scratch interval changed"
                        );
                        pending.first = std::cmp::min(pending.first, first);
                        pending.last = std::cmp::max(pending.last, last);
                        cursor.merge_pending = Some(pending);
                        cursor.merge_predecessor = false;
                        return bounded_build(cursor, None, limits);
                    }
                }
                if cursor.merge_predecessor {
                    cursor.merge_pending = Some(pending);
                    cursor.merge_predecessor = false;
                } else {
                    let insert = Query::insert()
                        .into_table(Alias::new("sync_context_intervals"))
                        .columns(
                            ["namespace", "context_manifest", "writer", "first", "last"]
                                .map(Alias::new),
                        )
                        .values_panic([
                            cursor.scan.namespace.as_bytes().to_vec().into(),
                            scratch.to_vec().into(),
                            pending.writer.as_bytes().to_vec().into(),
                            i64::try_from(pending.first)?.into(),
                            i64::try_from(pending.last)?.into(),
                        ])
                        .to_owned();
                    tx.execute(&insert).await?;
                    cursor.merge_predecessor = true;
                }
                return bounded_build(cursor, None, limits);
            }
            if let Some(context) = cursor.context {
                let interval = crate::causal_source::interval_step_on(
                    tx,
                    cursor.scan.namespace,
                    context,
                    cursor.context_after.as_ref(),
                    limits,
                )
                .await?;
                if let Some(interval) = interval {
                    cursor.context_after = Some(interval.clone());
                    cursor.merge_pending = Some(UnionInterval {
                        writer: interval.writer,
                        first: u64::try_from(interval.first)?,
                        last: u64::try_from(interval.last)?,
                    });
                    cursor.merge_predecessor = true;
                } else {
                    cursor.context = None;
                    cursor.context_after = None;
                }
                return bounded_build(cursor, None, limits);
            }
            if !cursor.covering.done {
                cursor.context = covering_step_on(
                    tx,
                    cursor.scan.namespace,
                    &cursor.scan.collection,
                    crate::causal_source::CapturedBasis {
                        floor: cursor.scan.floor,
                        generation: &cursor.scan.basis,
                        state_seq: cursor.scan.state,
                    },
                    &mut cursor.covering,
                    limits,
                )
                .await?;
                if let Some(digest) = cursor.context {
                    let reference =
                        crate::causal_manifests::reference_on(tx, cursor.scan.namespace, digest)
                            .await?;
                    ensure!(
                        reference.kind == protocol::ManifestKind::Context,
                        "invalid retained context kind"
                    );
                }
                return bounded_build(cursor, None, limits);
            }
            cursor.scratch_after = None;
            cursor.encoder = CanonicalContextEncoder::default();
            cursor.phase = BuildPhase::Count;
            bounded_build(cursor, None, limits)
        }
        BuildPhase::Emit if cursor.index_pending.is_some() => {
            charge_mutations(limits, 2)?;
            session.validate(&cursor)?;
            let interval = cursor
                .index_pending
                .take()
                .ok_or_else(|| anyhow::anyhow!("missing pending interval"))?;
            let manifest = cursor
                .manifest
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing reserved manifest"))?;
            let mut context = sync::causal::Context::default();
            context.insert_interval(interval.writer, interval.first, interval.last)?;
            let mut remaining = 1;
            crate::causal_range::index_context_on(
                tx,
                cursor.scan.namespace,
                manifest.manifest_digest,
                &context,
                &mut remaining,
            )
            .await?;
            bounded_build(cursor, None, limits)
        }
        BuildPhase::Count | BuildPhase::Hash | BuildPhase::Emit
            if !cursor.stream_finished
                && (!matches!(cursor.phase, BuildPhase::Emit)
                    || cursor.pending.len() < cursor.fragment_bytes()) =>
        {
            let is_emit = matches!(cursor.phase, BuildPhase::Emit);
            let is_hash = matches!(cursor.phase, BuildPhase::Hash);
            charge_mutations(limits, 1)?;
            session.validate(&cursor)?;
            let scratch = cursor
                .scratch_root
                .ok_or_else(|| anyhow::anyhow!("missing scratch root"))?;
            let step = crate::causal_source::interval_step_on(
                tx,
                cursor.scan.namespace,
                scratch,
                cursor.scratch_after.as_ref(),
                limits,
            )
            .await?;
            match step {
                Some(indexed) => {
                    cursor.scratch_after = Some(indexed.clone());
                    let interval = UnionInterval {
                        writer: indexed.writer,
                        first: u64::try_from(indexed.first)?,
                        last: u64::try_from(indexed.last)?,
                    };
                    let bytes = cursor.encoder.push(&interval)?;
                    if is_hash {
                        hash_piece(&mut cursor, &bytes)?;
                    }
                    if is_emit {
                        cursor.pending.extend_from_slice(&bytes);
                        cursor.index_pending = Some(interval);
                    }
                }
                None => {
                    let bytes = cursor.encoder.finish()?;
                    match cursor.phase {
                        BuildPhase::Count => {
                            cursor.expected_bytes = cursor.encoder.encoded_bytes();
                            cursor.expected_writers = cursor.encoder.writer_count();
                            cursor.expected_exceptions = cursor.encoder.exception_count();
                            cursor.phase = BuildPhase::PrepareHash;
                        }
                        BuildPhase::Hash => {
                            hash_piece(&mut cursor, &bytes)?;
                            check_encoder(&cursor)?;
                            let content = cursor
                                .content_hash
                                .as_ref()
                                .ok_or_else(|| anyhow::anyhow!("missing content hasher"))?
                                .finish();
                            cursor.context_digest = Some(
                                cursor
                                    .context_hash
                                    .as_ref()
                                    .ok_or_else(|| anyhow::anyhow!("missing context hasher"))?
                                    .finish(),
                            );
                            let fragments = cursor
                                .expected_bytes
                                .div_ceil(CONTEXT_FRAGMENT_BYTES as u64);
                            ensure!(
                                fragments > 0 && fragments <= u64::from(protocol::MAX_FRAGMENTS),
                                protocol::SyncError::LogicalLimit
                            );
                            let mut manifest = protocol::ManifestRef {
                                kind: protocol::ManifestKind::Context,
                                manifest_digest: [0; 32],
                                content_digest: content,
                                encoded_bytes: cursor.expected_bytes,
                                item_count: cursor.expected_writers,
                                fragment_count: u32::try_from(fragments)?,
                            };
                            manifest.manifest_digest = protocol::manifest_digest(&manifest)?;
                            cursor.manifest = Some(manifest);
                            cursor.context_hash = None;
                            cursor.content_hash = None;
                            cursor.phase = BuildPhase::Reserve;
                        }
                        BuildPhase::Emit => {
                            cursor.pending.extend_from_slice(&bytes);
                            check_encoder(&cursor)?;
                            cursor.stream_finished = true;
                        }
                        _ => unreachable!(),
                    }
                }
            }
            bounded_build(cursor, None, limits)
        }
        BuildPhase::PrepareHash | BuildPhase::PrepareEmit => {
            charge_mutations(limits, 1)?;
            session.validate(&cursor)?;
            cursor.scratch_after = None;
            cursor.encoder = CanonicalContextEncoder::default();
            if matches!(cursor.phase, BuildPhase::PrepareHash) {
                cursor.context_hash = Some(tagged_hasher(b"sync.context", cursor.expected_bytes)?);
                cursor.content_hash = Some(tagged_hasher(
                    b"sync.manifest.content",
                    cursor.expected_bytes,
                )?);
                cursor.phase = BuildPhase::Hash;
            } else {
                cursor.phase = BuildPhase::Emit;
            }
            bounded_build(cursor, None, limits)
        }
        BuildPhase::Reserve => {
            charge_mutations(limits, 7)?;
            let pin = crate::causal_leases::building_on(
                tx,
                cursor.scan.namespace,
                cursor.scan.lease,
                now_ms,
            )
            .await?;
            ensure!(
                pin.retained_root.as_deref() == cursor.scratch_root.as_ref().map(|d| d.as_slice())
                    && pin.basis_state_seq == Some(cursor.scan.floor)
                    && pin.basis_generation.as_deref() == Some(cursor.scan.basis.as_slice())
                    && pin.state_seq == cursor.scan.state,
                protocol::SyncError::StaleWork
            );
            let manifest = cursor
                .manifest
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing context manifest"))?;
            crate::causal_manifests::reserve_context_on(tx, cursor.scan.namespace, manifest)
                .await?;
            let previous = pin
                .retained_root
                .map(|v| <[u8; 32]>::try_from(v).map_err(|_| anyhow::anyhow!("invalid work root")))
                .transpose()?;
            let root = crate::causal_manifests::publish_work_node_on(
                tx,
                cursor.scan.namespace,
                previous,
                Some(manifest.manifest_digest),
                b"context-reservation-v1",
                limits.max_bytes,
                limits.max_mutations - 3,
            )
            .await?;
            cursor.work_root = Some(root);
            cursor.phase = BuildPhase::PrepareEmit;
            bounded_build(cursor, Some(root), limits)
        }
        BuildPhase::Emit => {
            if !cursor.pending.is_empty() {
                charge_mutations(limits, 4)?;
                let pin = crate::causal_leases::building_on(
                    tx,
                    cursor.scan.namespace,
                    cursor.scan.lease,
                    now_ms,
                )
                .await?;
                ensure!(
                    pin.retained_root.as_deref() == cursor.work_root.as_ref().map(|d| d.as_slice()),
                    protocol::SyncError::StaleWork
                );
                let fragment_bytes = cursor.fragment_bytes();
                let take = std::cmp::min(cursor.pending.len(), fragment_bytes);
                ensure!(
                    take == fragment_bytes || cursor.stream_finished,
                    "premature context fragment"
                );
                let bytes: Vec<u8> = cursor.pending.drain(..take).collect();
                let mut fragment = protocol::Fragment {
                    manifest: cursor
                        .manifest
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("missing context manifest"))?,
                    ordinal: cursor.emitted_ordinal,
                    offset: cursor.emitted_offset,
                    bytes,
                    digest: [0; 32],
                };
                let (expected_offset, expected_end) =
                    protocol::fragment_bounds(&fragment.manifest, fragment.ordinal)?;
                ensure!(
                    fragment.offset == expected_offset
                        && fragment.bytes.len() as u64 == expected_end - expected_offset,
                    "noncanonical context fragment layout"
                );
                fragment.digest = protocol::fragment_digest(&fragment)?;
                crate::causal_manifests::reserve_context_fragment_on(
                    tx,
                    cursor.scan.namespace,
                    &fragment,
                )
                .await?;
                cursor.emitted_offset = cursor
                    .emitted_offset
                    .checked_add(fragment.bytes.len() as u64)
                    .ok_or(protocol::SyncError::LogicalLimit)?;
                cursor.emitted_ordinal = cursor
                    .emitted_ordinal
                    .checked_add(1)
                    .ok_or(protocol::SyncError::LogicalLimit)?;
                bounded_build(cursor, None, limits)
            } else {
                let manifest = cursor
                    .manifest
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("missing context manifest"))?;
                ensure!(
                    cursor.stream_finished
                        && cursor.emitted_offset == manifest.encoded_bytes
                        && cursor.emitted_ordinal == manifest.fragment_count,
                    "incomplete context fragments"
                );
                cursor.context_hash = Some(tagged_hasher(b"sync.context", cursor.expected_bytes)?);
                cursor.content_hash = Some(tagged_hasher(
                    b"sync.manifest.content",
                    cursor.expected_bytes,
                )?);
                cursor.phase = BuildPhase::VerifyHeader;
                bounded_build(cursor, None, limits)
            }
        }
        BuildPhase::VerifyHeader => {
            charge_mutations(limits, 1)?;
            let pin = crate::causal_leases::building_on(
                tx,
                cursor.scan.namespace,
                cursor.scan.lease,
                now_ms,
            )
            .await?;
            ensure!(
                pin.retained_root.as_deref() == cursor.work_root.as_ref().map(|d| d.as_slice()),
                protocol::SyncError::StaleWork
            );
            let manifest = cursor
                .manifest
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing context manifest"))?;
            if cursor.verify_ordinal == manifest.fragment_count {
                ensure!(
                    cursor.verify_offset == manifest.encoded_bytes
                        && cursor
                            .context_hash
                            .as_ref()
                            .is_some_and(|h| Some(h.finish()) == cursor.context_digest)
                        && cursor
                            .content_hash
                            .as_ref()
                            .is_some_and(|h| h.finish() == manifest.content_digest),
                    "stored context verification failed"
                );
                cursor.phase = BuildPhase::Promote;
            } else {
                cursor.verify_header = Some(
                    crate::causal_manifests::read_reserved_fragment_header_on(
                        tx,
                        cursor.scan.namespace,
                        manifest,
                        cursor.verify_ordinal,
                        limits.max_bytes,
                    )
                    .await?,
                );
                cursor.phase = BuildPhase::VerifyBody;
            }
            bounded_build(cursor, None, limits)
        }
        BuildPhase::VerifyBody => {
            charge_mutations(limits, 1)?;
            let pin = crate::causal_leases::building_on(
                tx,
                cursor.scan.namespace,
                cursor.scan.lease,
                now_ms,
            )
            .await?;
            ensure!(
                pin.retained_root.as_deref() == cursor.work_root.as_ref().map(|d| d.as_slice()),
                protocol::SyncError::StaleWork
            );
            let manifest = cursor
                .manifest
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing context manifest"))?;
            let header = cursor
                .verify_header
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing context fragment header"))?;
            let fragment = crate::causal_manifests::read_reserved_fragment_body_on(
                tx,
                cursor.scan.namespace,
                manifest,
                header,
                limits.max_bytes,
            )
            .await?;
            ensure!(
                fragment.ordinal == cursor.verify_ordinal
                    && fragment.offset == cursor.verify_offset,
                "noncontiguous stored context fragment"
            );
            hash_piece(&mut cursor, &fragment.bytes)?;
            cursor.verify_offset = cursor
                .verify_offset
                .checked_add(fragment.bytes.len() as u64)
                .ok_or(protocol::SyncError::LogicalLimit)?;
            cursor.verify_ordinal = cursor
                .verify_ordinal
                .checked_add(1)
                .ok_or(protocol::SyncError::LogicalLimit)?;
            cursor.verify_header = None;
            cursor.phase = BuildPhase::VerifyHeader;
            bounded_build(cursor, None, limits)
        }
        BuildPhase::Promote => {
            charge_mutations(limits, 2)?;
            let proof = ContextCompletionProof {
                namespace: cursor.scan.namespace,
                lease: cursor.scan.lease,
                retained_root: cursor
                    .work_root
                    .ok_or_else(|| anyhow::anyhow!("missing context work root"))?,
                manifest: cursor
                    .manifest
                    .ok_or_else(|| anyhow::anyhow!("missing context manifest"))?,
                context_digest: cursor
                    .context_digest
                    .ok_or_else(|| anyhow::anyhow!("missing context digest"))?,
            };
            Ok(ContextBuildStep::Ready(
                crate::causal_manifests::promote_context_on(tx, proof.namespace, &proof, now_ms)
                    .await?,
            ))
        }
        _ => anyhow::bail!("invalid context build phase"),
    }
}
