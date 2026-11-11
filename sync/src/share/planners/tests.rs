use super::*;
use crate::protocol::{
    Candidate, Dot, Event, KeyDelta, ModelId, ObjectKey, Revision, StoreIdentity, SyncModel,
};
use crate::share::{base_key, chunk_key, segment_key};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
enum Collection {
    Notes,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Payload {
    text: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Model;
impl SyncModel for Model {
    type Collection = Collection;
    type Payload = Payload;
    fn model_id() -> &'static str {
        "planner-test"
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
    fn validate(_: &ObjectKey<Self>, _: &Option<Payload>) -> crate::protocol::Result<()> {
        Ok(())
    }
}

fn identity() -> StoreIdentity {
    StoreIdentity {
        namespace: Uuid::from_u128(1),
        model: ModelId {
            model: Model::model_id().into(),
            schema: Model::schema_id().into(),
        },
        replica: Uuid::from_u128(2),
        generation: Uuid::from_u128(3),
    }
}

fn entry(local_sequence: u64, origin: Option<StoreIdentity>, text: &str) -> JournalEvent<Model> {
    let identity = identity();
    let event = Event::Key(KeyDelta {
        key: ObjectKey {
            collection: Collection::Notes,
            id: Uuid::from_u128((local_sequence + 100) as u128),
        },
        author: Revision {
            counter: local_sequence.max(1),
            actor: Uuid::from_u128(4),
        },
        removed: Vec::new(),
        added: vec![Candidate {
            model: identity.model.clone(),
            namespace: identity.namespace,
            key: ObjectKey {
                collection: Collection::Notes,
                id: Uuid::from_u128((local_sequence + 100) as u128),
            },
            dot: Dot {
                writer_incarnation: Uuid::from_u128(5),
                counter: local_sequence.max(1),
            },
            revision: Revision {
                counter: local_sequence.max(1),
                actor: Uuid::from_u128(4),
            },
            payload: Payload { text: text.into() },
        }],
    });
    JournalEvent {
        local_sequence,
        event,
        origin,
    }
}

struct PublishOptions<'a> {
    head: Checkpoint,
    newest_base_gzip_bytes: BaseSize,
    export_cursor: ExportCursorStatus,
    max_events_per_segment: usize,
    existing_segments: &'a [SegmentRef],
}

fn publish_input<'a>(
    identity: &'a StoreIdentity,
    entries: &'a [JournalEvent<Model>],
    share_stream_ids: &'a BTreeSet<Uuid>,
    share_local_replicas: &'a BTreeSet<Uuid>,
    options: PublishOptions<'a>,
) -> PublishInput<'a, Model> {
    PublishInput {
        identity,
        after_cursor: 0,
        entries,
        share_stream_ids,
        share_local_replicas,
        head: options.head,
        existing_segments: options.existing_segments,
        newest_base_gzip_bytes: options.newest_base_gzip_bytes,
        export_cursor: options.export_cursor,
        max_events_per_segment: options.max_events_per_segment,
    }
}

#[test]
fn publication_filters_origins_and_numbers_only_selected_events() {
    let id = identity();
    let mut lan = identity();
    lan.replica = Uuid::from_u128(20);
    let mut share = identity();
    share.replica = Uuid::from_u128(21);
    let entries = vec![
        entry(1, None, "local"),
        entry(2, Some(lan.clone()), "lan relay"),
        entry(3, Some(share.clone()), "share relay"),
        entry(4, Some(identity()), "same local replica"),
        entry(5, Some(lan), "other origin"),
    ];
    let streams = [share.replica].into_iter().collect();
    let locals = [identity().replica].into_iter().collect();
    let PublishDecision::PublishSegments {
        events,
        export_cursor,
        ..
    } = plan_publish(publish_input(
        &id,
        &entries,
        &streams,
        &locals,
        PublishOptions {
            head: Checkpoint {
                seq: 7,
                anchor: [8; 32],
            },
            newest_base_gzip_bytes: BaseSize::Absent,
            export_cursor: ExportCursorStatus::Current,
            max_events_per_segment: 10,
            existing_segments: &[],
        },
    ))
    .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        events
            .iter()
            .map(|item| item.local_sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 5]
    );
    assert_eq!(
        events
            .iter()
            .map(|item| item.share_sequence)
            .collect::<Vec<_>>(),
        vec![8, 9, 10]
    );
    assert_eq!(export_cursor, 5);
}

#[test]
fn filtered_relay_only_advances_export_cursor_without_publishing_a_link() {
    let id = identity();
    let mut share_origin = identity();
    share_origin.replica = Uuid::from_u128(21);
    let entries = [entry(7, Some(share_origin.clone()), "already in share")];
    let streams = [share_origin.replica].into_iter().collect();
    let local_replicas = BTreeSet::new();
    assert_eq!(
        plan_publish(publish_input(
            &id,
            &entries,
            &streams,
            &local_replicas,
            PublishOptions {
                head: Checkpoint {
                    seq: 4,
                    anchor: [4; 32],
                },
                newest_base_gzip_bytes: BaseSize::Absent,
                export_cursor: ExportCursorStatus::Current,
                max_events_per_segment: 10,
                existing_segments: &[],
            },
        ))
        .unwrap(),
        PublishDecision::NothingToPublish { export_cursor: 7 }
    );
}

#[test]
fn publication_uses_segments_and_rolls_up_each_sixteen_contiguous_inputs() {
    let id = identity();
    let entries: Vec<_> = (1..=256).map(|n| entry(n, None, "x")).collect();
    let streams = BTreeSet::new();
    let locals = BTreeSet::new();
    let PublishDecision::PublishSegments {
        segments, rollups, ..
    } = plan_publish(publish_input(
        &id,
        &entries,
        &streams,
        &locals,
        PublishOptions {
            head: Checkpoint {
                seq: 0,
                anchor: [1; 32],
            },
            newest_base_gzip_bytes: BaseSize::Absent,
            export_cursor: ExportCursorStatus::Current,
            max_events_per_segment: 1,
            existing_segments: &[],
        },
    ))
    .unwrap()
    else {
        panic!()
    };
    assert_eq!(segments.len(), 256);
    assert_eq!(rollups.iter().filter(|plan| plan.level == 1).count(), 16);
    assert_eq!(rollups.iter().filter(|plan| plan.level == 2).count(), 1);
    assert_eq!(
        rollups.last().map(|plan| (plan.first, plan.last)),
        Some((1, 256))
    );
}

#[test]
fn export_pin_expiry_cuts_base_even_without_pending_events() {
    let id = identity();
    let empty = Vec::new();
    let PublishDecision::CutBase { reason, covers_seq } = plan_publish(publish_input(
        &id,
        &empty,
        &BTreeSet::new(),
        &BTreeSet::new(),
        PublishOptions {
            head: Checkpoint {
                seq: 9,
                anchor: [0; 32],
            },
            newest_base_gzip_bytes: BaseSize::Absent,
            export_cursor: ExportCursorStatus::NeedsBase,
            max_events_per_segment: 10,
            existing_segments: &[],
        },
    ))
    .unwrap() else {
        panic!()
    };
    assert_eq!(reason, BulkReason::ExportCursorNeedsBase);
    assert_eq!(covers_seq, 10);
}

#[test]
fn base_fraction_is_strictly_greater_than_one_quarter() {
    let id = identity();
    let entries = vec![entry(1, None, "one")];
    let streams = BTreeSet::new();
    let locals = BTreeSet::new();
    let args = || {
        publish_input(
            &id,
            &entries,
            &streams,
            &locals,
            PublishOptions {
                head: Checkpoint {
                    seq: 0,
                    anchor: [0; 32],
                },
                newest_base_gzip_bytes: BaseSize::Absent,
                export_cursor: ExportCursorStatus::Current,
                max_events_per_segment: 10,
                existing_segments: &[],
            },
        )
    };
    let PublishDecision::PublishSegments {
        compressed_bytes, ..
    } = plan_publish(args()).unwrap()
    else {
        panic!()
    };
    let PublishDecision::PublishSegments { .. } = plan_publish(publish_input(
        &id,
        &entries,
        &streams,
        &locals,
        PublishOptions {
            head: Checkpoint {
                seq: 0,
                anchor: [0; 32],
            },
            newest_base_gzip_bytes: BaseSize::GzipBytes(compressed_bytes * 4),
            export_cursor: ExportCursorStatus::Current,
            max_events_per_segment: 10,
            existing_segments: &[],
        },
    ))
    .unwrap() else {
        panic!("equal quarter must stay incremental")
    };
    let PublishDecision::CutBase { reason, .. } = plan_publish(publish_input(
        &id,
        &entries,
        &streams,
        &locals,
        PublishOptions {
            head: Checkpoint {
                seq: 0,
                anchor: [0; 32],
            },
            newest_base_gzip_bytes: BaseSize::GzipBytes(compressed_bytes * 4 - 1),
            export_cursor: ExportCursorStatus::Current,
            max_events_per_segment: 10,
            existing_segments: &[],
        },
    ))
    .unwrap() else {
        panic!("above quarter must cut")
    };
    assert_eq!(reason, BulkReason::BaseFractionExceeded);
}

#[test]
fn bulk_gzip_limit_is_strictly_greater_than_sixteen_mib() {
    assert_eq!(
        bulk_assessment(
            BULK_GZIP_LIMIT,
            BaseSize::Absent,
            ExportCursorStatus::Current
        ),
        BulkAssessment::Incremental
    );
    assert_eq!(
        bulk_assessment(
            BULK_GZIP_LIMIT + 1,
            BaseSize::Absent,
            ExportCursorStatus::Current
        ),
        BulkAssessment::Bulk(BulkReason::GzipLimitExceeded)
    );
    assert_eq!(
        bulk_assessment(1, BaseSize::GzipBytes(0), ExportCursorStatus::Current),
        BulkAssessment::Bulk(BulkReason::BaseFractionExceeded)
    );
}

#[test]
fn gzip_limit_boundary_selects_bulk_path() {
    let id = identity();
    let mut random = 0x1234_5678_u32;
    let text: String = (0..62_000)
        .map(|_| {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            char::from(b'a' + (random % 26) as u8)
        })
        .collect();
    let entries: Vec<_> = (1..=500).map(|n| entry(n, None, &text)).collect();
    let decision = plan_publish(publish_input(
        &id,
        &entries,
        &BTreeSet::new(),
        &BTreeSet::new(),
        PublishOptions {
            head: Checkpoint {
                seq: 0,
                anchor: [0; 32],
            },
            newest_base_gzip_bytes: BaseSize::Absent,
            export_cursor: ExportCursorStatus::Current,
            max_events_per_segment: 500,
            existing_segments: &[],
        },
    ))
    .unwrap();
    assert!(matches!(
        decision,
        PublishDecision::CutBase {
            reason: BulkReason::GzipLimitExceeded,
            ..
        }
    ));
}

#[test]
fn keeper_and_base_cut_boundaries_follow_local_observation_time() {
    let me = Uuid::from_u128(1);
    let other = Uuid::from_u128(2);
    let activities = [
        StreamActivity {
            stream_id: other,
            last_head_change_ms: 1,
        },
        StreamActivity {
            stream_id: me,
            last_head_change_ms: 1,
        },
    ];
    assert_eq!(
        plan_keeper(me, 100, &activities, NewestBase::Absent, 0),
        KeeperDecision::CutBase(BaseCutReason::Cadence)
    );
    assert_eq!(
        plan_keeper(
            me,
            1 + KEEPER_ACTIVE_MS,
            &activities,
            NewestBase::Present {
                created_ms: 1,
                gzip_bytes: 100
            },
            25
        ),
        KeeperDecision::Wait
    );
    assert_eq!(
        plan_keeper(
            me,
            1 + KEEPER_ACTIVE_MS,
            &activities,
            NewestBase::Present {
                created_ms: 1,
                gzip_bytes: 100
            },
            26
        ),
        KeeperDecision::CutBase(BaseCutReason::TailFraction)
    );
    assert_eq!(
        plan_keeper(
            other,
            2 + KEEPER_ACTIVE_MS,
            &activities,
            NewestBase::Present {
                created_ms: 2 + KEEPER_ACTIVE_MS,
                gzip_bytes: 100
            },
            0
        ),
        KeeperDecision::NotKeeper {
            keeper: Keeper::None
        }
    );
    let fresh_activities = [
        StreamActivity {
            stream_id: other,
            last_head_change_ms: 1 + BASE_CADENCE_MS,
        },
        StreamActivity {
            stream_id: me,
            last_head_change_ms: 1 + BASE_CADENCE_MS,
        },
    ];
    assert_eq!(
        plan_keeper(
            other,
            1 + BASE_CADENCE_MS,
            &fresh_activities,
            NewestBase::Present {
                created_ms: 1,
                gzip_bytes: 100
            },
            0
        ),
        KeeperDecision::NotKeeper {
            keeper: Keeper::Stream(me)
        }
    );
    assert_eq!(
        plan_keeper(
            me,
            1 + BASE_CADENCE_MS,
            &[
                StreamActivity {
                    stream_id: me,
                    last_head_change_ms: 1 + BASE_CADENCE_MS
                },
                StreamActivity {
                    stream_id: other,
                    last_head_change_ms: 1 + BASE_CADENCE_MS
                },
            ],
            NewestBase::Present {
                created_ms: 1,
                gzip_bytes: 100
            },
            0
        ),
        KeeperDecision::CutBase(BaseCutReason::Cadence)
    );
    assert_eq!(
        plan_keeper(
            other,
            1 + BASE_MAX_AGE_MS,
            &[
                StreamActivity {
                    stream_id: other,
                    last_head_change_ms: 1 + BASE_MAX_AGE_MS
                },
                StreamActivity {
                    stream_id: me,
                    last_head_change_ms: 1 + BASE_MAX_AGE_MS
                },
            ],
            NewestBase::Present {
                created_ms: 1,
                gzip_bytes: 100
            },
            0
        ),
        KeeperDecision::NotKeeper {
            keeper: Keeper::Stream(me)
        }
    );
    assert_eq!(
        plan_keeper(
            other,
            1 + BASE_MAX_AGE_MS + 1,
            &[
                StreamActivity {
                    stream_id: other,
                    last_head_change_ms: 1 + BASE_MAX_AGE_MS + 1
                },
                StreamActivity {
                    stream_id: me,
                    last_head_change_ms: 1 + BASE_MAX_AGE_MS + 1
                },
            ],
            NewestBase::Present {
                created_ms: 1,
                gzip_bytes: 100
            },
            0
        ),
        KeeperDecision::CutBase(BaseCutReason::MaximumAge)
    );
}

#[test]
fn gc_obeys_ack_retention_pins_and_rechecks_retirement() {
    let device_prefix = DevicePrefix {
        stream: Uuid::from_u128(30),
        generation: Uuid::from_u128(31),
    };
    let covered = Checkpoint {
        seq: 40,
        anchor: [4; 32],
    };
    let base = BaseRef {
        base_id: "new".into(),
        covers: [(device_prefix.to_string(), covered.clone())]
            .into_iter()
            .collect(),
    };
    let manifest = RetainedManifest {
        base: base.clone(),
        status: ManifestStatus::Live,
        referenced_chunks: ["kept".into()].into_iter().collect(),
    };
    let segments = vec![
        OwnSegment {
            reference: SegmentRef {
                level: 0,
                first: 1,
                last: 10,
                prev_anchor: [1; 32],
                last_anchor: [2; 32],
                name: "a".into(),
                bytes: 1,
            },
            last_changed_ms: RETIREMENT_MS - OBJECT_UNREFERENCED_MS,
            key: segment_key(&device_prefix.to_string(), 1, 10, "a000000000000000").unwrap(),
        },
        OwnSegment {
            reference: SegmentRef {
                level: 0,
                first: 11,
                last: 20,
                prev_anchor: [2; 32],
                last_anchor: [3; 32],
                name: "b".into(),
                bytes: 1,
            },
            last_changed_ms: RETIREMENT_MS - OBJECT_UNREFERENCED_MS,
            key: segment_key(&device_prefix.to_string(), 11, 20, "b000000000000000").unwrap(),
        },
        OwnSegment {
            reference: SegmentRef {
                level: 0,
                first: 21,
                last: 30,
                prev_anchor: [3; 32],
                last_anchor: [4; 32],
                name: "c".into(),
                bytes: 1,
            },
            last_changed_ms: RETIREMENT_MS - SEGMENT_RETENTION_MS,
            key: segment_key(&device_prefix.to_string(), 21, 30, "c000000000000000").unwrap(),
        },
    ];
    let bases = [OwnBase {
        base_id: "old".into(),
        key: base_key(&device_prefix.to_string(), &"b".repeat(64)).unwrap(),
        superseded_since: SupersededSince::Since(RETIREMENT_MS - SUPERSEDED_BASE_RETENTION_MS),
    }];
    let old_manifest = RetainedManifest {
        base: BaseRef {
            base_id: "old".into(),
            covers: [(
                device_prefix.to_string(),
                Checkpoint {
                    seq: 10,
                    anchor: [1; 32],
                },
            )]
            .into_iter()
            .collect(),
        },
        status: ManifestStatus::Superseded,
        referenced_chunks: ["oldchunk".into()].into_iter().collect(),
    };
    let mut pins = InProgressCut {
        pinned_chunks: ["pinned".into()].into_iter().collect(),
        pinned_bases: ["old".into()].into_iter().collect(),
    };
    let chunks = ["orphan", "pinned", "kept", "oldchunk"]
        .iter()
        .map(|name| OwnChunk {
            name: (*name).into(),
            key: chunk_key(
                &device_prefix.to_string(),
                blake3::hash(name.as_bytes()).to_hex().as_ref(),
            )
            .unwrap(),
            first_unreferenced_ms: 0,
            second_unreferenced_observation: GcObservation::ObservedAt(OBJECT_UNREFERENCED_MS),
        })
        .collect::<Vec<_>>();
    let retired_prefix = DevicePrefix {
        stream: Uuid::from_u128(11),
        generation: Uuid::from_u128(12),
    };
    let retired = RetirementCandidate {
        prefix: retired_prefix.clone(),
        stream_id: retired_prefix.stream,
        generation: retired_prefix.generation,
        head_counter: 4,
        head: Checkpoint {
            seq: 8,
            anchor: [8; 32],
        },
        unchanged_since_ms: 0,
        read_status: RetirementReadStatus::ConsistentlyReadable,
    };
    let mut covers = base.covers.clone();
    covers.insert(retired_prefix.to_string(), retired.head.clone());
    let mut current = manifest.clone();
    current.base.covers = covers;
    let manifests = [current, old_manifest];
    let acks = [(Uuid::from_u128(20), 10)].into_iter().collect();
    let decision = plan_gc(GcInput {
        own_prefix: &device_prefix,
        now_ms: RETIREMENT_MS,
        head: Checkpoint {
            seq: 50,
            anchor: [5; 32],
        },
        segments: &segments,
        bases: &bases,
        manifests: &manifests,
        active_acks: &acks,
        in_progress: &pins,
        chunks: &chunks,
        retirements: std::slice::from_ref(&retired),
    })
    .unwrap();
    assert_eq!(
        decision.delete_segments,
        vec![
            segment_key(&device_prefix.to_string(), 1, 10, "a000000000000000").unwrap(),
            segment_key(&device_prefix.to_string(), 21, 30, "c000000000000000").unwrap(),
        ]
    );
    assert_eq!(
        decision.floor_base,
        FloorBaseSelection::Selected {
            base_id: "new".into(),
            covers: covered
        }
    );
    assert!(decision.delete_bases.is_empty());
    assert_eq!(
        decision.delete_chunks,
        vec![
            chunk_key(
                &device_prefix.to_string(),
                blake3::hash(b"orphan").to_hex().as_ref(),
            )
            .unwrap()
        ]
    );
    assert_eq!(decision.retirement_preconditions.len(), 1);
    assert_eq!(decision.retirement_preconditions[0].expected_counter, 4);
    assert_eq!(decision.floor.seq, 10);
    pins.pinned_chunks.clear();
    pins.pinned_bases.clear();
    let no_acks = [(Uuid::from_u128(20), 0)].into_iter().collect();
    let pinned_decision = plan_gc(GcInput {
        own_prefix: &device_prefix,
        now_ms: RETIREMENT_MS,
        head: Checkpoint {
            seq: 50,
            anchor: [5; 32],
        },
        segments: &segments,
        bases: &bases,
        manifests: &manifests,
        active_acks: &no_acks,
        in_progress: &pins,
        chunks: &chunks,
        retirements: std::slice::from_ref(&retired),
    })
    .unwrap();
    assert!(
        !pinned_decision.delete_segments.contains(
            &segment_key(&device_prefix.to_string(), 11, 20, "b000000000000000").unwrap()
        )
    );
    assert_eq!(
        pinned_decision.delete_bases,
        vec![base_key(&device_prefix.to_string(), &"b".repeat(64)).unwrap()]
    );
}

#[test]
fn gc_rejects_any_non_owned_object_delete_candidate() {
    let segments = [OwnSegment {
        reference: SegmentRef {
            level: 0,
            first: 1,
            last: 1,
            prev_anchor: [0; 32],
            last_anchor: [1; 32],
            name: "foreign".into(),
            bytes: 1,
        },
        last_changed_ms: 0,
        key: "devices/someone/seg/x".into(),
    }];
    let result = plan_gc(GcInput {
        own_prefix: &DevicePrefix {
            stream: Uuid::from_u128(20),
            generation: Uuid::from_u128(21),
        },
        now_ms: 0,
        head: Checkpoint {
            seq: 1,
            anchor: [1; 32],
        },
        segments: &segments,
        bases: &[],
        manifests: &[],
        active_acks: &BTreeMap::new(),
        in_progress: &InProgressCut {
            pinned_chunks: BTreeSet::new(),
            pinned_bases: BTreeSet::new(),
        },
        chunks: &[],
        retirements: &[],
    });
    assert!(result.is_err());
}
