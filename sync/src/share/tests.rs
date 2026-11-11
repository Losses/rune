use super::*;
use crate::protocol::{
    Event, KeyDelta, KeyRange, ModelId, ObjectKey as SyncObjectKey, RangeSummary,
    Result as ProtocolResult, Revision, StoreIdentity, SyncModel, event_digest, initial_anchor,
    next_anchor,
};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Read, time::Duration};
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
        (head_key(&prefix, 0).unwrap(), ObjectKey::Head { slot: 0 }),
        (
            segment_key(&prefix, 1, 2, "a1".repeat(8).as_str()).unwrap(),
            ObjectKey::Segment {
                first: 1,
                last: 2,
                hash16: "a1".repeat(8),
            },
        ),
        (
            base_key(&prefix, &"b".repeat(64)).unwrap(),
            ObjectKey::Base {
                base_id: "b".repeat(64),
            },
        ),
        (
            chunk_key(&prefix, &"c".repeat(64)).unwrap(),
            ObjectKey::Chunk {
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
