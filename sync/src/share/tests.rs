use super::*;
use crate::protocol::{
    ApplyLimits, CheckpointKey, Coordinator, DEFAULT_LOGICAL_BYTES, DEFAULT_LOGICAL_ITEMS,
    DEFAULT_STAGE_BYTES, DEFAULT_STAGES, Event, EventRef, HistoryStatus, KeyDelta, KeyRange,
    LocalAuthor, MemorySyncStore, ModelId, ObjectKey as SyncObjectKey, PageLimits, PageRead,
    PageRequest, RangeSummary, ReceiveProgress, ReceiveStore, Result as ProtocolResult, Revision,
    Scope, SessionAdvance, SessionLimits, SnapshotSource, StageLimits, StoreIdentity, SyncError,
    SyncModel, SyncPeer, SyncStore, WorkLimits, WorkOutcome, decode_event, event_digest,
    fragment_digest, initial_anchor, next_anchor, scope_id,
};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env,
    io::{Read, Write},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Collection {
    Notes,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Payload {
    text: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Model;
impl SyncModel for Model {
    type Collection = Collection;
    type Payload = Payload;
    fn model_id() -> &'static str {
        "share-test"
    }
    fn schema_id() -> &'static str {
        "one"
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
    fn validate(_: &SyncObjectKey<Self>, _: &Option<Payload>) -> ProtocolResult<()> {
        Ok(())
    }
}

fn head_fixture() -> Head {
    Head {
        format: FORMAT_VERSION,
        library_id: Uuid::from_u128(1),
        stream: Uuid::from_u128(2),
        generation: Uuid::from_u128(3),
        local_replica: Uuid::from_u128(4),
        writer_session: Uuid::from_u128(5),
        counter: 6,
        head: Checkpoint {
            seq: 7,
            anchor: [8; 32],
        },
        floor: Checkpoint {
            seq: 1,
            anchor: [9; 32],
        },
        floor_base: FloorBase {
            owner_prefix: format!("{}.{}", Uuid::from_u128(2), Uuid::from_u128(3)),
            base_id: "a".repeat(64),
            covers_seq: 1,
            covers_anchor: [9; 32],
        },
        segments: Vec::new(),
        bases: Vec::new(),
        acks: BTreeMap::new(),
        local_cursor: 10,
        retired: Vec::new(),
        updated_ms: 11,
    }
}

fn segment_fixture() -> Segment<Model> {
    let identity = StoreIdentity {
        namespace: Uuid::from_u128(1),
        model: ModelId {
            model: Model::model_id().into(),
            schema: Model::schema_id().into(),
        },
        replica: Uuid::from_u128(2),
        generation: Uuid::from_u128(3),
    };
    let previous = initial_anchor(&identity).unwrap();
    let event = Event::Key(KeyDelta {
        key: SyncObjectKey {
            collection: Collection::Notes,
            id: Uuid::from_u128(6),
        },
        author: Revision {
            counter: 1,
            actor: Uuid::from_u128(4),
        },
        removed: Vec::new(),
        added: Vec::new(),
    });
    let digest = event_digest(&event).unwrap();
    let last = next_anchor::<Model>(previous, 1, &Collection::Notes, digest).unwrap();
    Segment {
        format: FORMAT_VERSION,
        prev_anchor: previous,
        links: vec![SegmentLink {
            seq: 1,
            collection: Collection::Notes,
            event,
        }],
        last_anchor: last,
    }
}

struct PeerFixture {
    objects: Arc<MemoryObjectStore>,
    source: Arc<MemorySyncStore<Model>>,
    identity: StoreIdentity,
    head: Head,
}

fn share_scope() -> Scope<Model> {
    Scope {
        collections: [Collection::Notes].into_iter().collect(),
    }
}

fn share_limits() -> SessionLimits {
    SessionLimits {
        pages: PageLimits {
            max_scanned: 8,
            max_bytes: 256 * 1024,
        },
        stages: StageLimits {
            max_total_bytes: DEFAULT_STAGE_BYTES,
            max_stages: DEFAULT_STAGES,
            max_event_bytes: DEFAULT_LOGICAL_BYTES,
            max_event_items: DEFAULT_LOGICAL_ITEMS,
            max_fragment_bytes: 64 * 1024,
        },
        apply: ApplyLimits {
            max_projection_items: 8,
            max_projection_bytes: 256 * 1024,
            max_mutations: 1024,
        },
        work: WorkLimits {
            max_requests: 128,
            max_bytes: 4 * 1024 * 1024,
            max_depth: 32,
            max_rows_scanned: 64,
            max_mutations: 4096,
        },
    }
}

fn share_prefix(identity: &StoreIdentity) -> String {
    format!("{}.{}", identity.replica, identity.generation)
}

fn share_cache_dir() -> PathBuf {
    env::temp_dir().join(format!("rune-share-test-{}", Uuid::new_v4()))
}

async fn base_peer_fixture(count: u64) -> PeerFixture {
    let fixture = peer_fixture(count).await;
    let descriptor = match SnapshotSource::open_snapshot(
        &*fixture.source,
        Uuid::new_v4(),
        share_scope(),
        share_limits().work,
        None,
    )
    .await
    .unwrap()
    {
        WorkOutcome::Ready(descriptor) => descriptor,
        WorkOutcome::Incomplete { .. } => panic!("fixture snapshot should be bounded"),
    };
    let root = descriptor.roots[0].clone();
    let mut after = None;
    let mut candidates = Vec::new();
    loop {
        let page = SnapshotSource::leaf(
            &*fixture.source,
            descriptor.snapshot_id,
            root.range.clone(),
            after,
            PageLimits {
                max_scanned: 128,
                max_bytes: 4 * 1024 * 1024,
            },
        )
        .await
        .unwrap();
        candidates.extend(page.candidates);
        after = page.next;
        if page.done {
            break;
        }
    }
    let values = candidates
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let canonical_candidates = serde_json::to_vec(&serde_json::Value::Array(values)).unwrap();
    let (chunk_name, chunk_bytes) = encode_chunk(canonical_candidates).unwrap();
    let prefix = share_prefix(&fixture.identity);
    fixture
        .objects
        .put(&chunk_key(&prefix, &chunk_name).unwrap(), &chunk_bytes)
        .await
        .unwrap();
    let context = &descriptor.contexts[0];
    let mut context_names = Vec::new();
    for ordinal in 0..context.context.manifest.fragment_count {
        let fragment = SnapshotSource::read_fragment(
            &*fixture.source,
            descriptor.snapshot_id,
            context.context.manifest.clone(),
            ordinal,
            256 * 1024,
        )
        .await
        .unwrap();
        let digest = blake3::hash(&fragment.bytes).to_hex().to_string();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&fragment.bytes).unwrap();
        let compressed = encoder.finish().unwrap();
        fixture
            .objects
            .put(&chunk_key(&prefix, &digest).unwrap(), &compressed)
            .await
            .unwrap();
        context_names.push(digest);
    }
    let head = fixture.source.head().await.unwrap();
    let checkpoint = Checkpoint {
        seq: head.sequence,
        anchor: head.anchor,
    };
    let bucket_count = if root.candidate_count == 0 { 0 } else { 1 };
    let buckets = if bucket_count == 0 {
        Vec::new()
    } else {
        vec![BucketSummary {
            range: root.range.clone(),
            candidate_count: root.candidate_count,
            candidate_bytes: root.candidate_bytes,
            aggregate: root.aggregate,
            chunk: chunk_name,
        }]
    };
    let manifest = BaseManifest {
        format: FORMAT_VERSION,
        covers: BTreeMap::from([(
            prefix.clone(),
            Checkpoint {
                seq: checkpoint.seq,
                anchor: checkpoint.anchor,
            },
        )]),
        collections: BTreeMap::from([(
            "notes".to_owned(),
            CollectionSummary {
                root,
                bucket_count,
                buckets,
                context_fragments: context_names,
            },
        )]),
    };
    let (base_id, manifest_bytes) = encode_manifest(&manifest).unwrap();
    fixture
        .objects
        .put(&base_key(&prefix, &base_id).unwrap(), &manifest_bytes)
        .await
        .unwrap();
    let mut new_head = fixture.head.clone();
    new_head.counter += 1;
    new_head.floor = Checkpoint {
        seq: checkpoint.seq,
        anchor: checkpoint.anchor,
    };
    new_head.floor_base = FloorBase {
        owner_prefix: prefix.clone(),
        base_id: base_id.clone(),
        covers_seq: checkpoint.seq,
        covers_anchor: checkpoint.anchor,
    };
    new_head.bases.push(BaseRef {
        base_id,
        covers: BTreeMap::from([(prefix.clone(), checkpoint)]),
    });
    let slot = (new_head.counter % 2) as u8;
    fixture
        .objects
        .put(
            &head_key(&prefix, slot).unwrap(),
            &encode_head(&new_head).unwrap(),
        )
        .await
        .unwrap();
    PeerFixture {
        head: new_head,
        ..fixture
    }
}

async fn advance_to_complete(peer: &SharePeer<Model>, target: &MemorySyncStore<Model>) {
    let mut coordinator = Coordinator::new(share_scope(), share_limits()).unwrap();
    for _ in 0..3000 {
        match coordinator
            .advance(target, target, target, peer, peer)
            .await
            .unwrap()
        {
            SessionAdvance::Complete(_) => return,
            SessionAdvance::BudgetExhausted => coordinator.start_pass(),
            SessionAdvance::Incomplete => (),
            SessionAdvance::Cancelled => panic!("unexpected cancellation"),
        }
    }
    panic!("base reconciliation did not complete");
}

async fn append_share_segment(fixture: &PeerFixture) {
    let key = SyncObjectKey {
        collection: Collection::Notes,
        id: Uuid::from_u128(900),
    };
    fixture
        .source
        .write_local(
            key,
            Some(Payload {
                text: "after base".into(),
            }),
        )
        .await
        .unwrap();
    let previous = ReceiveProgress {
        sequence: fixture.head.head.seq,
        anchor: fixture.head.head.anchor,
    };
    let through = fixture.source.head().await.unwrap();
    let page = fixture
        .source
        .read_page(PageRequest {
            source: fixture.identity.clone(),
            scope: share_scope(),
            after: previous.clone(),
            through: through.clone(),
            limits: share_limits().pages,
        })
        .await
        .unwrap();
    let PageRead::Page(page) = page else {
        panic!("small fixture event should be inline");
    };
    let link = page.links.into_iter().next().unwrap();
    let event = match link.event.unwrap() {
        EventRef::Inline(event) => event,
        EventRef::Fragmented(_) => panic!("small fixture event should be inline"),
    };
    let anchor = next_anchor::<Model>(
        previous.anchor,
        link.sequence,
        &link.collection,
        link.event_digest,
    )
    .unwrap();
    let segment = Segment {
        format: FORMAT_VERSION,
        prev_anchor: previous.anchor,
        links: vec![SegmentLink {
            seq: link.sequence,
            collection: link.collection,
            event,
        }],
        last_anchor: anchor,
    };
    let bytes = encode_segment(&segment).unwrap();
    let name = blake3::hash(&bytes).to_hex()[..16].to_owned();
    let prefix = share_prefix(&fixture.identity);
    fixture
        .objects
        .put(
            &segment_key(&prefix, link.sequence, link.sequence, &name).unwrap(),
            &bytes,
        )
        .await
        .unwrap();
    let mut head = fixture.head.clone();
    head.counter += 1;
    head.head = Checkpoint {
        seq: through.sequence,
        anchor: through.anchor,
    };
    head.segments.push(SegmentRef {
        level: 0,
        first: link.sequence,
        last: link.sequence,
        prev_anchor: previous.anchor,
        last_anchor: anchor,
        name,
        bytes: bytes.len() as u64,
    });
    let slot = (head.counter % 2) as u8;
    fixture
        .objects
        .put(
            &head_key(&prefix, slot).unwrap(),
            &encode_head(&head).unwrap(),
        )
        .await
        .unwrap();
}

fn fresh_receiver() -> MemorySyncStore<Model> {
    MemorySyncStore::new(
        StoreIdentity {
            namespace: Uuid::from_u128(1),
            model: ModelId {
                model: Model::model_id().into(),
                schema: Model::schema_id().into(),
            },
            replica: Uuid::new_v4(),
            generation: Uuid::new_v4(),
        },
        LocalAuthor {
            author: Uuid::new_v4(),
            writer_incarnation: Uuid::new_v4(),
        },
    )
    .unwrap()
}

#[tokio::test]
async fn share_base_reconciles_then_continues_from_its_cover() {
    let fixture = base_peer_fixture(3).await;
    let peer = SharePeer::<Model>::new(
        fixture.objects.clone(),
        fixture.identity.namespace,
        share_prefix(&fixture.identity),
        share_cache_dir(),
    )
    .unwrap();
    let target = fresh_receiver();
    advance_to_complete(&peer, &target).await;
    for id in 100..103 {
        let key = SyncObjectKey {
            collection: Collection::Notes,
            id: Uuid::from_u128(id),
        };
        assert_eq!(
            target.get(&key).await.unwrap(),
            fixture.source.get(&key).await.unwrap()
        );
    }

    append_share_segment(&fixture).await;
    let checkpoint_key = checkpoint_key(&fixture.identity);
    let checkpoint = target.checkpoint(&checkpoint_key).await.unwrap();
    assert!(matches!(
        peer.history_status(&checkpoint_key, checkpoint)
            .await
            .unwrap(),
        HistoryStatus::Continuous { .. }
    ));
    let page = peer
        .read_page(PageRequest {
            source: fixture.identity.clone(),
            scope: share_scope(),
            after: ReceiveProgress {
                sequence: fixture.head.head.seq,
                anchor: fixture.head.head.anchor,
            },
            through: peer.head().await.unwrap(),
            limits: share_limits().pages,
        })
        .await
        .unwrap();
    assert!(matches!(page, PageRead::Page(ref page) if page.links.len() == 1));
    advance_to_complete(&peer, &target).await;
    let key = SyncObjectKey {
        collection: Collection::Notes,
        id: Uuid::from_u128(900),
    };
    assert_eq!(
        target.get(&key).await.unwrap(),
        fixture.source.get(&key).await.unwrap()
    );
}

#[tokio::test]
async fn equal_base_buckets_use_precomputed_summaries_without_chunk_reads() {
    let fixture = base_peer_fixture(2).await;
    let prefix = share_prefix(&fixture.identity);
    let chunk = &fixture.head.bases[0].base_id;
    let peer = SharePeer::<Model>::new(
        fixture.objects.clone(),
        fixture.identity.namespace,
        prefix.clone(),
        share_cache_dir(),
    )
    .unwrap();
    let descriptor = match SnapshotSource::open_snapshot(
        &peer,
        Uuid::new_v4(),
        share_scope(),
        share_limits().work,
        None,
    )
    .await
    .unwrap()
    {
        WorkOutcome::Ready(descriptor) => descriptor,
        WorkOutcome::Incomplete { .. } => panic!("immutable base is immediately ready"),
    };
    let root = &descriptor.roots[0];
    let manifest = decode_manifest::<Model>(
        &fixture
            .objects
            .get(&base_key(&prefix, chunk).unwrap())
            .await
            .unwrap()
            .bytes,
        chunk,
    )
    .unwrap();
    let chunk_name = &manifest.collections["notes"].buckets[0].chunk;
    let object_key = chunk_key(&prefix, chunk_name).unwrap();
    let before = fixture.objects.get_count(&object_key);
    assert_eq!(
        SnapshotSource::summarize(
            &peer,
            descriptor.snapshot_id,
            root.range.clone(),
            share_limits().work,
            None
        )
        .await
        .unwrap(),
        WorkOutcome::Ready(root.clone())
    );
    let children = SnapshotSource::split(
        &peer,
        descriptor.snapshot_id,
        root.range.clone(),
        share_limits().work,
        None,
    )
    .await
    .unwrap();
    let WorkOutcome::Ready(children) = children else {
        panic!("split is immediate")
    };
    assert_eq!(children.len(), 1);
    let child = SnapshotSource::summarize(
        &peer,
        descriptor.snapshot_id,
        children[0].clone(),
        share_limits().work,
        None,
    )
    .await
    .unwrap();
    assert!(matches!(child, WorkOutcome::Ready(_)));
    assert_eq!(fixture.objects.get_count(&object_key), before);
}

#[tokio::test]
async fn share_base_chunk_corruption_is_typed() {
    let fixture = base_peer_fixture(2).await;
    let prefix = share_prefix(&fixture.identity);
    let peer = SharePeer::<Model>::new(
        fixture.objects.clone(),
        fixture.identity.namespace,
        prefix.clone(),
        share_cache_dir(),
    )
    .unwrap();
    let descriptor = match SnapshotSource::open_snapshot(
        &peer,
        Uuid::new_v4(),
        share_scope(),
        share_limits().work,
        None,
    )
    .await
    .unwrap()
    {
        WorkOutcome::Ready(descriptor) => descriptor,
        WorkOutcome::Incomplete { .. } => panic!("immutable base is immediately ready"),
    };
    let manifest = decode_manifest::<Model>(
        &fixture
            .objects
            .get(&base_key(&prefix, &fixture.head.floor_base.base_id).unwrap())
            .await
            .unwrap()
            .bytes,
        &fixture.head.floor_base.base_id,
    )
    .unwrap();
    let name = &manifest.collections["notes"].buckets[0].chunk;
    fixture
        .objects
        .put(&chunk_key(&prefix, name).unwrap(), b"corrupt gzip")
        .await
        .unwrap();
    let result = SnapshotSource::leaf(
        &peer,
        descriptor.snapshot_id,
        descriptor.roots[0].range.clone(),
        None,
        share_limits().pages,
    )
    .await;
    assert_eq!(result, Err(SyncError::SnapshotSourceUnavailable));
}

#[tokio::test]
async fn share_base_missing_manifest_is_typed() {
    let missing = base_peer_fixture(1).await;
    let missing_prefix = share_prefix(&missing.identity);
    missing
        .objects
        .delete(&base_key(&missing_prefix, &missing.head.floor_base.base_id).unwrap())
        .await
        .unwrap();
    let missing_peer = SharePeer::<Model>::new(
        missing.objects.clone(),
        missing.identity.namespace,
        missing_prefix,
        share_cache_dir(),
    )
    .unwrap();
    assert_eq!(
        SnapshotSource::open_snapshot(
            &missing_peer,
            Uuid::new_v4(),
            share_scope(),
            share_limits().work,
            None
        )
        .await,
        Err(SyncError::SnapshotSourceUnavailable)
    );
}

#[tokio::test]
async fn share_base_chunk_cache_hits_do_not_read_the_object_store() {
    let fixture = base_peer_fixture(2).await;
    let prefix = share_prefix(&fixture.identity);
    let peer = SharePeer::<Model>::new(
        fixture.objects.clone(),
        fixture.identity.namespace,
        prefix.clone(),
        share_cache_dir(),
    )
    .unwrap();
    let descriptor = match SnapshotSource::open_snapshot(
        &peer,
        Uuid::new_v4(),
        share_scope(),
        share_limits().work,
        None,
    )
    .await
    .unwrap()
    {
        WorkOutcome::Ready(descriptor) => descriptor,
        WorkOutcome::Incomplete { .. } => panic!("immutable base is immediately ready"),
    };
    let manifest = decode_manifest::<Model>(
        &fixture
            .objects
            .get(&base_key(&prefix, &fixture.head.floor_base.base_id).unwrap())
            .await
            .unwrap()
            .bytes,
        &fixture.head.floor_base.base_id,
    )
    .unwrap();
    let object_key = chunk_key(&prefix, &manifest.collections["notes"].buckets[0].chunk).unwrap();
    for _ in 0..2 {
        SnapshotSource::leaf(
            &peer,
            descriptor.snapshot_id,
            descriptor.roots[0].range.clone(),
            None,
            share_limits().pages,
        )
        .await
        .unwrap();
    }
    assert_eq!(fixture.objects.get_count(&object_key), 1);
}

async fn peer_fixture(count: u64) -> PeerFixture {
    let identity = StoreIdentity {
        namespace: Uuid::from_u128(1),
        model: ModelId {
            model: Model::model_id().into(),
            schema: Model::schema_id().into(),
        },
        replica: Uuid::from_u128(20),
        generation: Uuid::from_u128(21),
    };
    let source = Arc::new(
        MemorySyncStore::<Model>::new(
            identity.clone(),
            LocalAuthor {
                author: Uuid::from_u128(22),
                writer_incarnation: Uuid::from_u128(23),
            },
        )
        .unwrap(),
    );
    for n in 1..=count {
        source
            .write_local(
                SyncObjectKey {
                    collection: Collection::Notes,
                    id: Uuid::from_u128(100 + u128::from(n)),
                },
                Some(Payload {
                    text: format!("note {n}"),
                }),
            )
            .await
            .unwrap();
    }
    let initial = initial_anchor(&identity).unwrap();
    let page = match source
        .read_page(PageRequest {
            source: identity.clone(),
            scope: share_scope(),
            after: ReceiveProgress {
                sequence: 0,
                anchor: initial,
            },
            through: source.head().await.unwrap(),
            limits: share_limits().pages,
        })
        .await
        .unwrap()
    {
        PageRead::Page(page) => page,
        PageRead::FragmentNeeded { .. } => panic!("small share fixture unexpectedly fragmented"),
    };
    let mut previous = initial;
    let mut segments = Vec::new();
    let mut references = Vec::new();
    for scan in page.links {
        let Some(EventRef::Inline(event)) = scan.event else {
            panic!("fixture event missing");
        };
        let anchor =
            next_anchor::<Model>(previous, scan.sequence, &scan.collection, scan.event_digest)
                .unwrap();
        let segment = Segment {
            format: FORMAT_VERSION,
            prev_anchor: previous,
            links: vec![SegmentLink {
                seq: scan.sequence,
                collection: scan.collection,
                event,
            }],
            last_anchor: anchor,
        };
        let bytes = encode_segment(&segment).unwrap();
        let name = blake3::hash(&bytes).to_hex()[..16].to_owned();
        references.push(SegmentRef {
            level: 0,
            first: scan.sequence,
            last: scan.sequence,
            prev_anchor: previous,
            last_anchor: anchor,
            name: name.clone(),
            bytes: bytes.len() as u64,
        });
        segments.push((scan.sequence, name, bytes));
        previous = anchor;
    }
    let prefix = share_prefix(&identity);
    let objects = Arc::new(MemoryObjectStore::default());
    objects
        .put(
            &library_key(),
            &encode_library(&LibraryFile {
                format: FORMAT_VERSION,
                library_id: identity.namespace,
                model: identity.model.model.clone(),
                schema: identity.model.schema.clone(),
            })
            .unwrap(),
        )
        .await
        .unwrap();
    for (sequence, name, bytes) in segments {
        objects
            .put(
                &segment_key(&prefix, sequence, sequence, &name).unwrap(),
                &bytes,
            )
            .await
            .unwrap();
    }
    let head = Head {
        format: FORMAT_VERSION,
        library_id: identity.namespace,
        stream: identity.replica,
        generation: identity.generation,
        local_replica: Uuid::from_u128(24),
        writer_session: Uuid::from_u128(25),
        counter: 1,
        head: Checkpoint {
            seq: count,
            anchor: previous,
        },
        floor: Checkpoint {
            seq: 0,
            anchor: initial,
        },
        floor_base: FloorBase {
            owner_prefix: prefix.clone(),
            base_id: "a".repeat(64),
            covers_seq: 0,
            covers_anchor: initial,
        },
        segments: references,
        bases: Vec::new(),
        acks: BTreeMap::new(),
        local_cursor: count,
        retired: Vec::new(),
        updated_ms: 0,
    };
    objects
        .put(&head_key(&prefix, 1).unwrap(), &encode_head(&head).unwrap())
        .await
        .unwrap();
    PeerFixture {
        objects,
        source,
        identity,
        head,
    }
}

fn checkpoint_key(identity: &StoreIdentity) -> CheckpointKey {
    CheckpointKey {
        namespace: identity.namespace,
        source_replica: identity.replica,
        source_generation: identity.generation,
        scope_id: scope_id(&share_scope()).unwrap(),
    }
}

async fn coordinator_pull(fixture: &PeerFixture, target: &MemorySyncStore<Model>) {
    let peer = SharePeer::<Model>::new(
        fixture.objects.clone(),
        fixture.identity.namespace,
        share_prefix(&fixture.identity),
        share_cache_dir(),
    )
    .unwrap();
    let mut coordinator = Coordinator::new(share_scope(), share_limits()).unwrap();
    for _ in 0..3000 {
        match coordinator
            .advance(target, target, target, &peer, target)
            .await
            .unwrap()
        {
            SessionAdvance::Complete(_) => return,
            SessionAdvance::BudgetExhausted => coordinator.start_pass(),
            SessionAdvance::Incomplete => (),
            SessionAdvance::Cancelled => panic!("unexpected cancellation"),
        }
    }
    panic!("share peer coordinator did not finish");
}

#[tokio::test]
async fn share_peer_identity_and_history_statuses_follow_head_and_anchors() {
    let fixture = peer_fixture(3).await;
    let peer = SharePeer::<Model>::new(
        fixture.objects.clone(),
        fixture.identity.namespace,
        share_prefix(&fixture.identity),
        share_cache_dir(),
    )
    .unwrap();
    let identity = peer.identity().await.unwrap();
    assert_eq!(identity, fixture.identity);
    let key = checkpoint_key(&identity);
    let initial = ReceiveProgress {
        sequence: 0,
        anchor: initial_anchor(&identity).unwrap(),
    };
    assert!(matches!(
        peer.history_status(&key, None).await.unwrap(),
        HistoryStatus::Uninitialized { .. }
    ));
    assert!(matches!(
        peer.history_status(
            &key,
            Some(ReceiveProgress {
                sequence: 4,
                anchor: [1; 32]
            })
        )
        .await
        .unwrap(),
        HistoryStatus::HeadBehind { head: 3 }
    ));
    assert!(matches!(
        peer.history_status(
            &key,
            Some(ReceiveProgress {
                sequence: 1,
                anchor: [9; 32]
            })
        )
        .await
        .unwrap(),
        HistoryStatus::AnchorMismatch { .. }
    ));
    assert!(matches!(
        peer.history_status(&key, Some(initial.clone()))
            .await
            .unwrap(),
        HistoryStatus::Continuous { .. }
    ));
    let other_generation = CheckpointKey {
        source_generation: Uuid::from_u128(999),
        ..key.clone()
    };
    assert!(matches!(
        peer.history_status(&other_generation, Some(initial.clone()))
            .await
            .unwrap(),
        HistoryStatus::GenerationChanged { .. }
    ));
    assert_eq!(peer.genesis_checkpoint().await.unwrap(), Some(initial));
}

#[tokio::test]
async fn share_peer_reports_below_floor_and_missing_referenced_segments() {
    let fixture = peer_fixture(3).await;
    let prefix = share_prefix(&fixture.identity);
    let mut head = fixture.head.clone();
    head.floor = Checkpoint {
        seq: 1,
        anchor: head.segments[0].last_anchor,
    };
    head.floor_base.covers_seq = 1;
    head.floor_base.covers_anchor = head.floor.anchor;
    head.counter = 2;
    fixture
        .objects
        .put(&head_key(&prefix, 0).unwrap(), &encode_head(&head).unwrap())
        .await
        .unwrap();
    let peer = SharePeer::<Model>::new(
        fixture.objects.clone(),
        fixture.identity.namespace,
        prefix.clone(),
        share_cache_dir(),
    )
    .unwrap();
    let key = checkpoint_key(&fixture.identity);
    assert!(matches!(
        peer.history_status(
            &key,
            Some(ReceiveProgress {
                sequence: 0,
                anchor: initial_anchor(&fixture.identity).unwrap()
            })
        )
        .await
        .unwrap(),
        HistoryStatus::BelowFloor { .. }
    ));

    let current = fixture.head.segments[1].clone();
    fixture
        .objects
        .delete(&segment_key(&prefix, current.first, current.last, &current.name).unwrap())
        .await
        .unwrap();
    assert!(matches!(
        peer.read_page(PageRequest {
            source: fixture.identity.clone(),
            scope: share_scope(),
            after: ReceiveProgress {
                sequence: 1,
                anchor: head.floor.anchor,
            },
            through: peer.head().await.unwrap(),
            limits: PageLimits {
                max_scanned: 32,
                max_bytes: 64 * 1024,
            },
        })
        .await,
        Err(SyncError::NeedsReconciliation(_))
    ));
}

#[tokio::test]
async fn share_peer_uses_highest_valid_head_and_rejects_two_torn_slots() {
    let fixture = peer_fixture(2).await;
    let prefix = share_prefix(&fixture.identity);
    fixture
        .objects
        .put(
            &head_key(&prefix, 0).unwrap(),
            &encode_head(&fixture.head).unwrap(),
        )
        .await
        .unwrap();
    fixture
        .objects
        .put(&head_key(&prefix, 1).unwrap(), b"torn")
        .await
        .unwrap();
    let peer = SharePeer::<Model>::new(
        fixture.objects.clone(),
        fixture.identity.namespace,
        prefix.clone(),
        share_cache_dir(),
    )
    .unwrap();
    assert_eq!(peer.head().await.unwrap().sequence, 2);
    fixture
        .objects
        .put(&head_key(&prefix, 0).unwrap(), b"torn")
        .await
        .unwrap();
    assert!(matches!(peer.head().await, Err(SyncError::Storage(_))));
}

#[tokio::test]
async fn share_peer_page_verifies_links_and_maps_503_to_transient_error() {
    let fixture = peer_fixture(1).await;
    let prefix = share_prefix(&fixture.identity);
    let peer = SharePeer::<Model>::new(
        fixture.objects.clone(),
        fixture.identity.namespace,
        prefix.clone(),
        share_cache_dir(),
    )
    .unwrap();
    let request = PageRequest {
        source: fixture.identity.clone(),
        scope: share_scope(),
        after: ReceiveProgress {
            sequence: 0,
            anchor: initial_anchor(&fixture.identity).unwrap(),
        },
        through: peer.head().await.unwrap(),
        limits: share_limits().pages,
    };
    fixture.objects.inject_error(
        StoreOperation::Get,
        StoreError::Transient("HTTP 503".into()),
    );
    assert!(matches!(
        peer.read_page(request.clone()).await,
        Err(SyncError::Transport(_))
    ));
    fixture.objects.clear_error(StoreOperation::Get);
    let reference = fixture.head.segments[0].clone();
    let key = segment_key(&prefix, reference.first, reference.last, &reference.name).unwrap();
    let original = fixture.objects.get(&key).await.unwrap();
    let mut altered = decode_segment::<Model>(&original.bytes).unwrap();
    let (sequence, collection, digest) = {
        let link = &mut altered.links[0];
        let Event::Key(delta) = &mut link.event else {
            panic!("fixture event should be a key event");
        };
        delta.author.counter += 1;
        if let Some(candidate) = delta.added.first_mut() {
            candidate.revision = delta.author;
        }
        (
            link.seq,
            link.collection.clone(),
            event_digest(&link.event).unwrap(),
        )
    };
    altered.last_anchor =
        next_anchor::<Model>(altered.prev_anchor, sequence, &collection, digest).unwrap();
    fixture
        .objects
        .put(&key, &encode_segment(&altered).unwrap())
        .await
        .unwrap();
    assert!(matches!(
        peer.read_page(request).await,
        Err(SyncError::NeedsReconciliation(
            HistoryStatus::BelowFloor { .. }
        ))
    ));
}

#[tokio::test]
async fn share_peer_large_event_fragments_are_read_from_segment_bytes() {
    let fixture = peer_fixture(1).await;
    let event_record = fixture
        .source
        .write_local(
            SyncObjectKey {
                collection: Collection::Notes,
                id: Uuid::from_u128(777),
            },
            Some(Payload {
                text: "large event ".repeat(1000),
            }),
        )
        .await
        .unwrap();
    let after = fixture.source.head().await.unwrap();
    let source_page = match fixture
        .source
        .read_page(PageRequest {
            source: fixture.identity.clone(),
            scope: share_scope(),
            after: ReceiveProgress {
                sequence: 1,
                anchor: fixture.head.head.anchor,
            },
            through: after.clone(),
            limits: share_limits().pages,
        })
        .await
        .unwrap()
    {
        PageRead::Page(page) => page,
        PageRead::FragmentNeeded { .. } => {
            panic!("large fixture source page unexpectedly fragmented")
        }
    };
    let scan = &source_page.links[0];
    let EventRef::Inline(event) = scan.event.clone().unwrap() else {
        panic!("event missing")
    };
    let bytes = encode_segment(&Segment {
        format: FORMAT_VERSION,
        prev_anchor: fixture.head.head.anchor,
        links: vec![SegmentLink {
            seq: scan.sequence,
            collection: scan.collection.clone(),
            event: event.clone(),
        }],
        last_anchor: after.anchor,
    })
    .unwrap();
    let name = blake3::hash(&bytes).to_hex()[..16].to_owned();
    let prefix = share_prefix(&fixture.identity);
    fixture
        .objects
        .put(&segment_key(&prefix, 2, 2, &name).unwrap(), &bytes)
        .await
        .unwrap();
    let mut head = fixture.head.clone();
    head.counter = 2;
    head.head = Checkpoint {
        seq: 2,
        anchor: after.anchor,
    };
    head.segments.push(SegmentRef {
        level: 0,
        first: 2,
        last: 2,
        prev_anchor: fixture.head.head.anchor,
        last_anchor: after.anchor,
        name,
        bytes: bytes.len() as u64,
    });
    fixture
        .objects
        .put(&head_key(&prefix, 0).unwrap(), &encode_head(&head).unwrap())
        .await
        .unwrap();

    let peer = SharePeer::<Model>::new(
        fixture.objects.clone(),
        fixture.identity.namespace,
        prefix,
        share_cache_dir(),
    )
    .unwrap();
    let request = PageRequest {
        source: fixture.identity.clone(),
        scope: share_scope(),
        after: ReceiveProgress {
            sequence: 1,
            anchor: fixture.head.head.anchor,
        },
        through: after,
        limits: PageLimits {
            max_scanned: 8,
            max_bytes: 4096,
        },
    };
    let PageRead::FragmentNeeded {
        manifest,
        sequence,
        event_digest: digest,
        ..
    } = peer.read_page(request).await.unwrap()
    else {
        panic!("large event did not request fragments");
    };
    assert_eq!(sequence, 2);
    let mut encoded = Vec::new();
    for ordinal in 0..manifest.fragment_count {
        let fragment = SyncPeer::read_fragment(&peer, manifest.clone(), ordinal, 4096)
            .await
            .unwrap();
        assert_eq!(fragment.offset, encoded.len() as u64);
        assert_eq!(fragment.digest, fragment_digest(&fragment).unwrap());
        encoded.extend(fragment.bytes);
    }
    assert_eq!(
        event_digest(&decode_event::<Model>(&encoded).unwrap()).unwrap(),
        digest
    );
    assert_eq!(
        event_record.value,
        Some(Payload {
            text: "large event ".repeat(1000)
        })
    );
}

#[tokio::test]
async fn share_peer_coordinator_keeps_receiver_unchanged_on_503() {
    let fixture = peer_fixture(1).await;
    let peer = SharePeer::<Model>::new(
        fixture.objects.clone(),
        fixture.identity.namespace,
        share_prefix(&fixture.identity),
        share_cache_dir(),
    )
    .unwrap();
    let target = MemorySyncStore::<Model>::new(
        StoreIdentity {
            replica: Uuid::from_u128(41),
            generation: Uuid::from_u128(42),
            ..fixture.identity.clone()
        },
        LocalAuthor {
            author: Uuid::from_u128(43),
            writer_incarnation: Uuid::from_u128(44),
        },
    )
    .unwrap();
    let key = checkpoint_key(&fixture.identity);
    target
        .adopt_genesis_checkpoint(
            key.clone(),
            peer.genesis_checkpoint().await.unwrap().unwrap(),
        )
        .unwrap();
    let before = target.head().await.unwrap();
    let before_checkpoint = target.checkpoint(&key).await.unwrap();
    fixture.objects.inject_error(
        StoreOperation::Get,
        StoreError::Transient("HTTP 503".into()),
    );
    let mut coordinator = Coordinator::new(share_scope(), share_limits()).unwrap();
    let result = coordinator
        .advance(&target, &target, &target, &peer, &target)
        .await;
    assert!(matches!(result, Err(SyncError::Transport(_))));
    assert_eq!(target.head().await.unwrap(), before);
    assert_eq!(target.checkpoint(&key).await.unwrap(), before_checkpoint);
}

#[tokio::test]
async fn share_peer_rejects_unknown_library_and_head_versions() {
    let fixture = peer_fixture(1).await;
    let prefix = share_prefix(&fixture.identity);
    let peer = SharePeer::<Model>::new(
        fixture.objects.clone(),
        fixture.identity.namespace,
        prefix.clone(),
        share_cache_dir(),
    )
    .unwrap();
    fixture.objects.put(&library_key(), br#"{"format":2,"library_id":"00000000-0000-0000-0000-000000000001","model":"share-test","schema":"one"}"#).await.unwrap();
    assert!(
        matches!(peer.head().await, Err(SyncError::Storage(message)) if message.contains("unknown library format version 2"))
    );

    fixture
        .objects
        .put(
            &library_key(),
            &encode_library(&LibraryFile {
                format: FORMAT_VERSION,
                library_id: fixture.identity.namespace,
                model: fixture.identity.model.model.clone(),
                schema: fixture.identity.model.schema.clone(),
            })
            .unwrap(),
        )
        .await
        .unwrap();
    let encoded_head = encode_head(&fixture.head).unwrap();
    let body_end = encoded_head
        .iter()
        .rposition(|byte| *byte == b'\n')
        .unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(&encoded_head[..body_end]).unwrap();
    json["format"] = serde_json::json!(2);
    let body = serde_json::to_vec(&json).unwrap();
    let mut torn = body.clone();
    torn.push(b'\n');
    torn.extend_from_slice(blake3::hash(&body).to_hex().as_bytes());
    fixture
        .objects
        .put(&head_key(&prefix, 0).unwrap(), &torn)
        .await
        .unwrap();
    assert!(
        matches!(peer.head().await, Err(SyncError::Storage(message)) if message.contains("unknown head format version 2"))
    );
}

#[tokio::test]
async fn share_peer_pulls_full_and_incremental_prefix_through_coordinator() {
    let fixture = peer_fixture(3).await;
    let target_identity = StoreIdentity {
        replica: Uuid::from_u128(31),
        generation: Uuid::from_u128(32),
        ..fixture.identity.clone()
    };
    let target = MemorySyncStore::<Model>::new(
        target_identity,
        LocalAuthor {
            author: Uuid::from_u128(33),
            writer_incarnation: Uuid::from_u128(34),
        },
    )
    .unwrap();
    let key = checkpoint_key(&fixture.identity);
    target
        .adopt_genesis_checkpoint(
            key,
            ReceiveProgress {
                sequence: 0,
                anchor: initial_anchor(&fixture.identity).unwrap(),
            },
        )
        .unwrap();
    coordinator_pull(&fixture, &target).await;
    for n in 1..=3 {
        let key = SyncObjectKey {
            collection: Collection::Notes,
            id: Uuid::from_u128(100 + n),
        };
        assert_eq!(
            target.get(&key).await.unwrap(),
            fixture.source.get(&key).await.unwrap()
        );
    }

    let changed = fixture
        .source
        .write_local(
            SyncObjectKey {
                collection: Collection::Notes,
                id: Uuid::from_u128(104),
            },
            Some(Payload {
                text: "incremental".into(),
            }),
        )
        .await
        .unwrap();
    let after = fixture.source.head().await.unwrap();
    let page = match fixture
        .source
        .read_page(PageRequest {
            source: fixture.identity.clone(),
            scope: share_scope(),
            after: ReceiveProgress {
                sequence: 3,
                anchor: fixture.head.head.anchor,
            },
            through: after.clone(),
            limits: share_limits().pages,
        })
        .await
        .unwrap()
    {
        PageRead::Page(page) => page,
        PageRead::FragmentNeeded { .. } => {
            panic!("small incremental event unexpectedly fragmented")
        }
    };
    let EventRef::Inline(event) = page.links[0].event.clone().unwrap() else {
        panic!("event missing")
    };
    let previous = fixture.head.head.anchor;
    let link = &page.links[0];
    let segment = Segment {
        format: FORMAT_VERSION,
        prev_anchor: previous,
        links: vec![SegmentLink {
            seq: link.sequence,
            collection: link.collection.clone(),
            event,
        }],
        last_anchor: after.anchor,
    };
    let bytes = encode_segment(&segment).unwrap();
    let name = blake3::hash(&bytes).to_hex()[..16].to_owned();
    let prefix = share_prefix(&fixture.identity);
    fixture
        .objects
        .put(&segment_key(&prefix, 4, 4, &name).unwrap(), &bytes)
        .await
        .unwrap();
    let mut head = fixture.head.clone();
    head.counter = 2;
    head.head = Checkpoint {
        seq: 4,
        anchor: after.anchor,
    };
    head.segments.push(SegmentRef {
        level: 0,
        first: 4,
        last: 4,
        prev_anchor: previous,
        last_anchor: after.anchor,
        name: name.clone(),
        bytes: bytes.len() as u64,
    });
    fixture
        .objects
        .put(&head_key(&prefix, 0).unwrap(), &encode_head(&head).unwrap())
        .await
        .unwrap();
    assert_eq!(
        changed.value,
        Some(Payload {
            text: "incremental".into()
        })
    );
    coordinator_pull(&fixture, &target).await;
    assert_eq!(
        target
            .get(&SyncObjectKey {
                collection: Collection::Notes,
                id: Uuid::from_u128(104)
            })
            .await
            .unwrap()
            .unwrap()
            .value,
        Some(Payload {
            text: "incremental".into()
        })
    );
}

#[test]
fn all_share_object_types_round_trip() {
    let library = LibraryFile {
        format: FORMAT_VERSION,
        library_id: Uuid::from_u128(1),
        model: "share-test".into(),
        schema: "one".into(),
    };
    assert_eq!(
        decode_library(&encode_library(&library).unwrap()).unwrap(),
        library
    );
    let head = head_fixture();
    assert_eq!(decode_head(&encode_head(&head).unwrap()).unwrap(), head);
    let segment = segment_fixture();
    assert_eq!(
        decode_segment::<Model>(&encode_segment(&segment).unwrap()).unwrap(),
        segment
    );
    let manifest = BaseManifest {
        format: FORMAT_VERSION,
        covers: BTreeMap::from([(
            "stream.generation".into(),
            Checkpoint {
                seq: 1,
                anchor: [2; 32],
            },
        )]),
        collections: BTreeMap::from([(
            "notes".into(),
            CollectionSummary::<Model> {
                root: RangeSummary {
                    snapshot_id: Uuid::from_u128(7),
                    range: KeyRange {
                        collection: Collection::Notes,
                        lower: None,
                        upper: None,
                    },
                    candidate_count: 1,
                    candidate_bytes: 8,
                    aggregate: [3; 32],
                },
                bucket_count: 1,
                buckets: vec![BucketSummary {
                    range: KeyRange {
                        collection: Collection::Notes,
                        lower: None,
                        upper: None,
                    },
                    candidate_count: 1,
                    candidate_bytes: 8,
                    aggregate: [3; 32],
                    chunk: "b".repeat(64),
                }],
                context_fragments: vec!["c".repeat(64)],
            },
        )]),
    };
    let (base_id, manifest_bytes) = encode_manifest(&manifest).unwrap();
    assert_eq!(
        decode_manifest(&manifest_bytes, &base_id).unwrap(),
        manifest
    );
    let chunk_bytes = br#"[{"id":"one"}]"#.to_vec();
    let (name, gzip) = encode_chunk(chunk_bytes).unwrap();
    assert_eq!(
        decode_chunk(&name, &gzip).unwrap(),
        Chunk {
            format: FORMAT_VERSION,
            name,
            canonical_bytes: br#"{"candidates":[{"id":"one"}],"format":1}"#.to_vec()
        }
    );
}

#[test]
fn head_and_segment_have_stable_canonical_bytes() {
    let encoded = encode_head(&head_fixture()).unwrap();
    assert_eq!(
        String::from_utf8(encoded).unwrap(),
        "{\"acks\":{},\"bases\":[],\"counter\":6,\"floor\":{\"anchor\":[9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9],\"seq\":1},\"floor_base\":{\"base_id\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"covers_anchor\":[9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9],\"covers_seq\":1,\"owner_prefix\":\"00000000-0000-0000-0000-000000000002.00000000-0000-0000-0000-000000000003\"},\"format\":1,\"generation\":\"00000000-0000-0000-0000-000000000003\",\"head\":{\"anchor\":[8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8],\"seq\":7},\"library_id\":\"00000000-0000-0000-0000-000000000001\",\"local_cursor\":10,\"local_replica\":\"00000000-0000-0000-0000-000000000004\",\"retired\":[],\"segments\":[],\"stream\":\"00000000-0000-0000-0000-000000000002\",\"updated_ms\":11,\"writer_session\":\"00000000-0000-0000-0000-000000000005\"}\n6d5ed7361c77274b46a7c15156bccdd2f2de0042a48108a2838df6dbd96d3693"
    );
    let encoded_segment = encode_segment(&segment_fixture()).unwrap();
    let mut decoder = GzDecoder::new(encoded_segment.as_slice());
    let mut body = String::new();
    decoder.read_to_string(&mut body).unwrap();
    assert_eq!(
        body,
        "{\"format\":1,\"last_anchor\":[127,172,181,206,69,245,109,2,208,223,103,97,52,220,87,155,188,165,145,9,3,253,222,161,176,88,180,211,58,199,247,29],\"links\":[{\"collection\":\"notes\",\"event\":{\"added\":[],\"author\":{\"actor\":\"00000000-0000-0000-0000-000000000004\",\"counter\":1},\"key\":{\"collection\":\"notes\",\"id\":\"00000000-0000-0000-0000-000000000006\"},\"kind\":\"key\",\"removed\":[]},\"seq\":1}],\"prev_anchor\":[101,165,171,17,172,211,217,251,38,58,78,141,116,91,6,252,141,186,82,214,241,57,82,174,3,92,167,56,114,180,195,75]}"
    );
}

#[test]
fn verification_distinguishes_parse_version_checksum_digest_and_chain_failures() {
    assert!(matches!(decode_library(b"{"), Err(FormatError::Parse(_))));
    let unknown = br#"{"format":2,"library_id":"00000000-0000-0000-0000-000000000001","model":"share-test","schema":"one"}"#;
    assert!(matches!(
        decode_library(unknown),
        Err(FormatError::UnknownVersion { version: 2, .. })
    ));
    let mut head_bytes = encode_head(&head_fixture()).unwrap();
    head_bytes[0] ^= 1;
    assert_eq!(decode_head(&head_bytes), Err(FormatError::Checksum));
    let (chunk_name, chunk_gzip) = encode_chunk(br#"[{"x":1}]"#.to_vec()).unwrap();
    let mut altered_chunk = chunk_gzip;
    *altered_chunk.last_mut().unwrap() ^= 1;
    assert!(matches!(
        decode_chunk(&chunk_name, &altered_chunk),
        Err(FormatError::Compression(_)) | Err(FormatError::Digest)
    ));
    let (base_id, manifest) = encode_manifest(&BaseManifest::<Model> {
        format: 1,
        covers: BTreeMap::new(),
        collections: BTreeMap::new(),
    })
    .unwrap();
    assert_eq!(
        decode_manifest::<Model>(&manifest, &"0".repeat(64)),
        Err(FormatError::Digest)
    );
    assert_eq!(
        decode_manifest::<Model>(&manifest, &base_id)
            .unwrap()
            .format,
        1
    );
    let segment = segment_fixture();
    let mut changed_link = segment.clone();
    if let Event::Key(delta) = &mut changed_link.links[0].event {
        delta.author.counter = 2;
    }
    assert_eq!(encode_segment(&changed_link), Err(FormatError::Digest));
    let mut changed = segment.clone();
    changed.last_anchor[0] ^= 1;
    assert_eq!(encode_segment(&changed), Err(FormatError::Digest));
    let mut torn_segment = encode_segment(&segment).unwrap();
    torn_segment.truncate(torn_segment.len() - 4);
    assert!(matches!(
        decode_segment::<Model>(&torn_segment),
        Err(FormatError::Compression(_)) | Err(FormatError::Parse(_))
    ));
}

#[test]
fn keys_round_trip_and_foreign_names_are_rejected() {
    let prefix = format!("{}.{}", Uuid::from_u128(2), Uuid::from_u128(3));
    let examples = [
        (
            head_key(&prefix, 0).unwrap(),
            ObjectKey::Head {
                prefix: prefix.parse().unwrap(),
                slot: 0,
            },
        ),
        (
            segment_key(&prefix, 1, 2, "a1".repeat(8).as_str()).unwrap(),
            ObjectKey::Segment {
                prefix: prefix.parse().unwrap(),
                first: 1,
                last: 2,
                hash16: "a1".repeat(8),
            },
        ),
        (
            base_key(&prefix, &"b".repeat(64)).unwrap(),
            ObjectKey::Base {
                prefix: prefix.parse().unwrap(),
                base_id: "b".repeat(64),
            },
        ),
        (
            chunk_key(&prefix, &"c".repeat(64)).unwrap(),
            ObjectKey::Chunk {
                prefix: prefix.parse().unwrap(),
                digest: "c".repeat(64),
            },
        ),
    ];
    for (key, expected) in examples {
        assert_eq!(parse_key(&key).unwrap(), expected);
    }
    assert_eq!(parse_key(&library_key()).unwrap(), ObjectKey::Library);
    for foreign in [
        "devices/other/head.0.json",
        "bad/head.0.json",
        "x/obj/not-a-hash.gz",
        "x/seg/1-2-deadbeefdeadbeef.gz",
    ] {
        assert!(parse_key(foreign).is_err(), "{foreign}");
    }
}

#[tokio::test]
async fn memory_store_injects_errors_partial_writes_and_conditional_tags() {
    let store = MemoryObjectStore::default();
    store.inject_error(StoreOperation::Get, StoreError::NotFound);
    assert_eq!(store.get("missing").await, Err(StoreError::NotFound));
    store.inject_error(StoreOperation::Get, StoreError::Transient("timeout".into()));
    assert!(matches!(
        store.get("k").await,
        Err(StoreError::Transient(_))
    ));
    store.clear_error(StoreOperation::Get);
    store.inject_error(StoreOperation::Create, StoreError::PreconditionFailed);
    assert_eq!(
        store.create("blocked", b"x").await,
        Err(StoreError::PreconditionFailed)
    );
    store.clear_error(StoreOperation::Create);

    let tag1 = store.create("k", b"original").await.unwrap();
    let read = store.get("k").await.unwrap();
    assert_eq!(read.bytes, b"original");
    assert_eq!(read.tag, tag1);
    assert_eq!(
        store.get_if_none_match("k", &tag1).await,
        Ok(ConditionalGet::NotModified)
    );
    let stale = store.create("stale", b"older bytes").await.unwrap();
    assert!(matches!(
        store.get_if_none_match("k", &stale).await,
        Ok(ConditionalGet::Modified(ObjectValue { bytes, tag }))
            if bytes == b"original" && tag == tag1
    ));
    assert_eq!(
        store.put_if_match("k", &stale, b"replacement").await,
        Err(StoreError::PreconditionFailed)
    );
    let tag2 = store
        .put_if_match("k", &tag1, b"replacement")
        .await
        .unwrap();
    assert_ne!(tag1, tag2);
    assert_eq!(
        store.get_if_none_match("k", &tag1).await,
        Ok(ConditionalGet::Modified(ObjectValue {
            bytes: b"replacement".to_vec(),
            tag: tag2.clone(),
        }))
    );
    assert_eq!(
        store.get_if_none_match("k", &tag2).await,
        Ok(ConditionalGet::NotModified)
    );
    assert_eq!(store.put("same", b"replacement").await.unwrap(), tag2);
    assert_eq!(
        store.create("k", b"ignored").await,
        Err(StoreError::PreconditionFailed)
    );

    store.inject_error(StoreOperation::List, StoreError::Permanent("denied".into()));
    assert!(matches!(
        store.list_prefix("").await,
        Err(StoreError::Permanent(_))
    ));
    store.clear_error(StoreOperation::List);
    store.configure(MemoryStoreConfig {
        delay: Duration::from_millis(2),
        partial_write_bytes: Some(2),
    });
    assert!(matches!(
        store.put("partial", b"whole").await,
        Err(StoreError::Transient(_))
    ));
    assert_eq!(store.get("partial").await.unwrap().bytes, b"wh");
}
