use serde::{Deserialize, Serialize};
use sync::protocol::*;
#[derive(Clone, Debug, PartialEq, Eq)]
struct Notes;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Collection {
    Zebra,
    Alpha,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Note {
    text: String,
}
impl SyncModel for Notes {
    type Collection = Collection;
    type Payload = Note;
    fn model_id() -> &'static str {
        "notes"
    }
    fn schema_id() -> &'static str {
        "one"
    }
    fn collections() -> Vec<Collection> {
        vec![Collection::Zebra, Collection::Alpha]
    }
    fn collection_tag(c: &Collection) -> &'static str {
        match c {
            Collection::Zebra => "zebra",
            Collection::Alpha => "alpha",
        }
    }
    fn collection_from_tag(tag: &str) -> Option<Collection> {
        match tag {
            "zebra" => Some(Collection::Zebra),
            "alpha" => Some(Collection::Alpha),
            _ => None,
        }
    }
    fn validate(_: &ObjectKey<Self>, _: &Option<Note>) -> Result<()> {
        Ok(())
    }
    fn validate_revision(_: &ObjectKey<Self>, _: &Option<Note>, _: &Revision) -> Result<()> {
        Ok(())
    }
}
#[test]
fn candidate_encoding_uses_stable_tags_and_sorted_keys() {
    let c = Candidate::<Notes> {
        model: ModelId {
            model: "notes".into(),
            schema: "one".into(),
        },
        namespace: Uuid::from_u128(1),
        key: ObjectKey {
            collection: Collection::Alpha,
            id: Uuid::from_u128(2),
        },
        dot: Dot {
            writer_incarnation: Uuid::from_u128(3),
            counter: 1,
        },
        revision: Revision {
            actor: Uuid::from_u128(4),
            counter: 1,
        },
        payload: Note {
            text: "é\"".into()
        },
    };
    let actual = String::from_utf8(canonical_candidate(&c).unwrap()).unwrap();
    let expected = r#"{"dot":{"counter":1,"writer_incarnation":"00000000-0000-0000-0000-000000000003"},"key":{"collection":"alpha","id":"00000000-0000-0000-0000-000000000002"},"model":{"model":"notes","schema":"one"},"namespace":"00000000-0000-0000-0000-000000000001","payload":{"text":"é\""},"revision":{"actor":"00000000-0000-0000-0000-000000000004","counter":1}}"#;
    assert_eq!(actual, expected);
    assert_eq!(decode_candidate::<Notes>(actual.as_bytes()).unwrap(), c);
    let with_unknown = actual.replacen("{\"dot\":", "{\"extra\":0,\"dot\":", 1);
    assert!(decode_candidate::<Notes>(with_unknown.as_bytes()).is_err());
    let duplicate = actual.replacen("{\"dot\":", "{\"dot\":null,\"dot\":", 1);
    assert!(decode_candidate::<Notes>(duplicate.as_bytes()).is_err());
    assert_eq!(
        candidate_digest(&c).unwrap(),
        [
            0x96, 0x79, 0xa4, 0x0f, 0x77, 0x28, 0x33, 0x0d, 0x3e, 0x40, 0x10, 0xbe, 0xea, 0xe4,
            0xa2, 0x8e, 0x88, 0x31, 0x2c, 0x54, 0x1a, 0xbe, 0x16, 0xac, 0x44, 0x45, 0xdd, 0xa3,
            0x08, 0x17, 0x58, 0x2a
        ]
    );
}
#[test]
fn tags_control_object_order_independent_of_enum_ordinal() {
    let a = ObjectKey::<Notes> {
        collection: Collection::Alpha,
        id: Uuid::from_u128(1),
    };
    let z = ObjectKey::<Notes> {
        collection: Collection::Zebra,
        id: Uuid::from_u128(1),
    };
    assert!(a < z);
}
#[test]
fn key_event_roundtrip_rejects_unknown_and_noncanonical() {
    let e = Event::<Notes>::Key(KeyDelta {
        key: ObjectKey {
            collection: Collection::Alpha,
            id: Uuid::from_u128(2),
        },
        author: Revision {
            counter: 3,
            actor: Uuid::from_u128(4),
        },
        removed: vec![Dot {
            writer_incarnation: Uuid::from_u128(3),
            counter: 1,
        }],
        added: vec![],
    });
    let b = canonical_event(&e).unwrap();
    assert_eq!(decode_event::<Notes>(&b).unwrap(), e);
    let envelope = serde_json::to_value(&e).unwrap();
    assert_eq!(
        envelope,
        serde_json::from_slice::<serde_json::Value>(&b).unwrap()
    );
    assert_eq!(serde_json::from_value::<Event<Notes>>(envelope).unwrap(), e);
    let s = String::from_utf8(b).unwrap();
    let invalid = s.replacen("{\"added\":", "{\"surprise\":0,\"added\":", 1);
    assert!(decode_event::<Notes>(invalid.as_bytes()).is_err());
}

#[test]
fn transport_envelopes_use_tags_hex_and_reject_unknown_fields() {
    let scope = Scope::<Notes> {
        collections: [Collection::Zebra, Collection::Alpha].into_iter().collect(),
    };
    let value = serde_json::to_value(&scope).unwrap();
    assert_eq!(value["collections"], serde_json::json!(["alpha", "zebra"]));
    assert_eq!(
        serde_json::from_value::<Scope<Notes>>(value).unwrap(),
        scope
    );
    assert!(
        serde_json::from_str::<Scope<Notes>>(r#"{"collections":["alpha"],"extra":0}"#).is_err()
    );
    assert!(serde_json::from_str::<Scope<Notes>>(r#"{"collections":["zebra","alpha"]}"#).is_err());
    assert!(serde_json::from_str::<Scope<Notes>>(r#"{"collections":["unknown"]}"#).is_err());
    let error = SyncError::MissingDependency([0xab; 32]);
    let encoded = serde_json::to_value(&error).unwrap();
    assert_eq!(encoded["data"], "ab".repeat(32));
    assert_eq!(
        serde_json::from_value::<SyncError>(encoded.clone()).unwrap(),
        error
    );
    let mut uppercase = encoded;
    uppercase["data"] = "AB".repeat(32).into();
    assert!(serde_json::from_value::<SyncError>(uppercase).is_err());
    let retry = SyncError::RetryAt(ReceiveProgress {
        sequence: 3,
        anchor: [0x42; 32],
    });
    assert_eq!(
        serde_json::from_slice::<SyncError>(&serde_json::to_vec(&retry).unwrap()).unwrap(),
        retry
    );
    let fragment = Fragment {
        manifest: ManifestRef {
            kind: ManifestKind::Context,
            manifest_digest: [1; 32],
            content_digest: [2; 32],
            encoded_bytes: 2,
            item_count: 1,
            fragment_count: 1,
        },
        ordinal: 0,
        offset: 0,
        bytes: vec![0xab, 0x01],
        digest: [3; 32],
    };
    let encoded = serde_json::to_value(&fragment).unwrap();
    assert_eq!(encoded["bytes"], "ab01");
    assert_eq!(
        serde_json::from_value::<Fragment>(encoded).unwrap(),
        fragment
    );
}
#[test]
fn filtered_links_still_commit_every_scanned_sequence() {
    let source = StoreIdentity {
        namespace: Uuid::from_u128(1),
        model: ModelId {
            model: "notes".into(),
            schema: "one".into(),
        },
        replica: Uuid::from_u128(2),
        generation: Uuid::from_u128(3),
    };
    let start = ReceiveProgress {
        sequence: 0,
        anchor: initial_anchor(&source).unwrap(),
    };
    let omitted = [17u8; 32];
    let first = next_anchor::<Notes>(start.anchor, 1, &Collection::Zebra, omitted).unwrap();
    let event = Event::Key(KeyDelta::<Notes> {
        key: ObjectKey {
            collection: Collection::Alpha,
            id: Uuid::from_u128(8),
        },
        author: Revision {
            counter: 1,
            actor: Uuid::from_u128(2),
        },
        removed: vec![],
        added: vec![],
    });
    let digest = event_digest(&event).unwrap();
    let end = ReceiveProgress {
        sequence: 2,
        anchor: next_anchor::<Notes>(first, 2, &Collection::Alpha, digest).unwrap(),
    };
    let mut page = ChangePage {
        request: PageRequest {
            source,
            scope: Scope {
                collections: [Collection::Alpha].into_iter().collect(),
            },
            after: start,
            through: end.clone(),
            limits: PageLimits {
                max_scanned: 4,
                max_bytes: 4096,
            },
        },
        next: end,
        done: true,
        links: vec![
            ScanLink {
                sequence: 1,
                collection: Collection::Zebra,
                event_digest: omitted,
                event: None,
            },
            ScanLink {
                sequence: 2,
                collection: Collection::Alpha,
                event_digest: digest,
                event: Some(EventRef::Inline(event)),
            },
        ],
    };
    validate_scanned_links(&page).unwrap();
    page.links[0].event_digest[0] ^= 1;
    assert!(validate_scanned_links(&page).is_err());
}
#[test]
fn exact_context_rejects_gaps_being_filled_and_noncanonical_forms() {
    let context = ExactContext {
        writers: vec![WriterContext {
            writer_incarnation: Uuid::from_u128(4),
            prefix: 1,
            exceptions: vec![CounterInterval {
                first: 3,
                last_inclusive: 5,
            }],
        }],
    };
    let bytes = canonical_context(&context).unwrap();
    assert_eq!(decode_context(&bytes).unwrap(), context);
    let adjacent = ExactContext {
        writers: vec![WriterContext {
            writer_incarnation: Uuid::from_u128(4),
            prefix: 1,
            exceptions: vec![CounterInterval {
                first: 2,
                last_inclusive: 5,
            }],
        }],
    };
    assert!(canonical_context(&adjacent).is_err());
    let mut tampered = String::from_utf8(bytes).unwrap();
    tampered = tampered.replacen("{\"writers\":", "{\"other\":0,\"writers\":", 1);
    assert!(decode_context(tampered.as_bytes()).is_err());
}
#[test]
fn context_root_requires_exact_scope_and_canonical_manifest_order() {
    let scope = Scope::<Notes> {
        collections: [Collection::Alpha, Collection::Zebra].into_iter().collect(),
    };
    let mut manifest = ManifestRef {
        kind: ManifestKind::Context,
        manifest_digest: [0; 32],
        content_digest: [7; 32],
        encoded_bytes: 5,
        item_count: 1,
        fragment_count: 1,
    };
    manifest.manifest_digest = manifest_digest(&manifest).unwrap();
    let root = ContextRoot {
        scope: scope.clone(),
        contexts: vec![
            CollectionContext {
                collection: Collection::Alpha,
                context: ContextRef {
                    digest: [3; 32],
                    manifest: manifest.clone(),
                },
            },
            CollectionContext {
                collection: Collection::Zebra,
                context: ContextRef {
                    digest: [4; 32],
                    manifest,
                },
            },
        ],
    };
    let bytes = canonical_context_root(&root).unwrap();
    assert_eq!(decode_context_root::<Notes>(&scope, &bytes).unwrap(), root);
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["contexts"][0]["collection"], "alpha");
    assert_eq!(json["contexts"][1]["collection"], "zebra");
    assert_eq!(
        json["scope"],
        scope_id(&scope)
            .unwrap()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let mut wrong = root.clone();
    wrong.contexts.reverse();
    assert!(canonical_context_root(&wrong).is_err());
    wrong = root.clone();
    wrong.contexts.pop();
    assert!(canonical_context_root(&wrong).is_err());
    wrong = root.clone();
    wrong.contexts[0].context.manifest.kind = ManifestKind::MergeEvent;
    assert!(canonical_context_root(&wrong).is_err());
    let other = Scope::<Notes> {
        collections: [Collection::Alpha].into_iter().collect(),
    };
    assert!(decode_context_root::<Notes>(&other, &bytes).is_err());
}

#[test]
fn streaming_range_hash_validates_coverage_order_and_exact_preimage() {
    let scope = Scope::<Notes> {
        collections: [Collection::Alpha].into_iter().collect(),
    };
    let range = KeyRange::<Notes> {
        collection: Collection::Alpha,
        lower: None,
        upper: None,
    };
    let hasher = RangeHasher::new(&scope, range.clone(), 0, 0).unwrap();
    let actual = hasher.finish().unwrap();
    let mut expected = blake3::Hasher::new();
    let mut part = |data: &[u8]| {
        expected.update(&(data.len() as u32).to_be_bytes());
        expected.update(data);
    };
    part(b"sync.range");
    part(b"notes");
    part(b"one");
    expected.update(&scope_id(&scope).unwrap());
    let bounds = br#"{"collection":"alpha","lower":null,"upper":null}"#;
    expected.update(&(bounds.len() as u32).to_be_bytes());
    expected.update(bounds);
    expected.update(&0u64.to_be_bytes());
    expected.update(&0u64.to_be_bytes());
    assert_eq!(actual, *expected.finalize().as_bytes());
    let unfinished = RangeHasher::new(&scope, range, 1, 1).unwrap();
    assert!(matches!(
        unfinished.finish(),
        Err(SyncError::IncompleteCoverage)
    ));
}

fn candidate() -> Candidate<Notes> {
    Candidate {
        model: ModelId {
            model: "notes".into(),
            schema: "one".into(),
        },
        namespace: Uuid::from_u128(1),
        key: ObjectKey {
            collection: Collection::Alpha,
            id: Uuid::from_u128(2),
        },
        dot: Dot {
            writer_incarnation: Uuid::from_u128(3),
            counter: 1,
        },
        revision: Revision {
            counter: 1,
            actor: Uuid::from_u128(4),
        },
        payload: Note { text: "abc".into() },
    }
}
#[test]
fn merge_event_roundtrip_and_canonical() {
    let delta = MergeDelta::<Notes> {
        collection: Collection::Alpha,
        added: vec![candidate()],
        observed: ExactContext { writers: vec![] },
    };
    let event = Event::Merge(Box::new(delta.clone()));
    let bytes = canonical_event(&event).unwrap();
    let expected = r#"{"added":[{"dot":{"counter":1,"writer_incarnation":"00000000-0000-0000-0000-000000000003"},"key":{"collection":"alpha","id":"00000000-0000-0000-0000-000000000002"},"model":{"model":"notes","schema":"one"},"namespace":"00000000-0000-0000-0000-000000000001","payload":{"text":"abc"},"revision":{"actor":"00000000-0000-0000-0000-000000000004","counter":1}}],"collection":"alpha","kind":"merge","observed":{"writers":[]}}"#;
    assert_eq!(bytes, expected.as_bytes());
    assert_eq!(decode_event_remote::<Notes>(&bytes).unwrap(), event);
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(
        json,
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
    );
    assert_eq!(serde_json::from_value::<Event<Notes>>(json).unwrap(), event);
    let mut wrong = delta.clone();
    wrong.added.clear();
    assert!(canonical_event(&Event::Merge(Box::new(wrong))).is_err());
    let mut wrong = delta.clone();
    wrong.collection = Collection::Zebra;
    assert!(canonical_event(&Event::Merge(Box::new(wrong))).is_err());
    let mut wrong = delta.clone();
    let mut other = candidate();
    other.key.id = Uuid::from_u128(1);
    other.dot.counter = 2;
    wrong.added.push(other);
    assert!(canonical_event(&Event::Merge(Box::new(wrong))).is_err());
    let mut wrong = delta;
    wrong.observed.writers.push(WriterContext {
        writer_incarnation: Uuid::from_u128(3),
        prefix: 1,
        exceptions: vec![],
    });
    assert!(canonical_event(&Event::Merge(Box::new(wrong))).is_err());
    let unknown = expected.replacen("{", "{\"extra\":0,", 1);
    assert!(decode_event_remote::<Notes>(unknown.as_bytes()).is_err());
}
#[test]
fn peer_decode_errors_are_invalid_and_local_decode_errors_are_storage() {
    assert!(matches!(
        decode_candidate_remote::<Notes>(b"{"),
        Err(SyncError::Invalid(_))
    ));
    assert!(matches!(
        decode_candidate::<Notes>(b"{"),
        Err(SyncError::Storage(_))
    ));
    assert!(matches!(
        decode_event_remote::<Notes>(b"{"),
        Err(SyncError::Invalid(_))
    ));
    assert!(matches!(
        decode_event::<Notes>(b"{"),
        Err(SyncError::Storage(_))
    ));
    assert!(matches!(
        decode_context_remote(b"{"),
        Err(SyncError::Invalid(_))
    ));
    assert!(matches!(decode_context(b"{"), Err(SyncError::Storage(_))));
    let scope = Scope::<Notes> {
        collections: [Collection::Alpha].into_iter().collect(),
    };
    assert!(matches!(
        decode_context_root_remote(&scope, b"{"),
        Err(SyncError::Invalid(_))
    ));
    assert!(matches!(
        decode_context_root(&scope, b"{"),
        Err(SyncError::Storage(_))
    ));
    let mut payload = serde_json::to_value(candidate()).unwrap();
    payload["payload"]["text"] = 42.into();
    let bytes = serde_json::to_vec(&payload).unwrap();
    assert!(matches!(
        decode_candidate_remote::<Notes>(&bytes),
        Err(SyncError::Invalid(_))
    ));
    assert!(matches!(
        decode_candidate::<Notes>(&bytes),
        Err(SyncError::Storage(_))
    ));
}

#[test]
fn range_hash_checkpoint_resumes_every_feed_and_rejects_wrong_scope() {
    let scope = Scope::<Notes> {
        collections: [Collection::Alpha].into_iter().collect(),
    };
    let parent = KeyRange::<Notes> {
        collection: Collection::Alpha,
        lower: None,
        upper: None,
    };
    let candidate = Candidate::<Notes> {
        model: ModelId {
            model: "notes".into(),
            schema: "one".into(),
        },
        namespace: Uuid::from_u128(1),
        key: ObjectKey {
            collection: Collection::Alpha,
            id: Uuid::from_u128(20),
        },
        dot: Dot {
            writer_incarnation: Uuid::from_u128(2),
            counter: 1,
        },
        revision: Revision {
            counter: 1,
            actor: Uuid::from_u128(3),
        },
        payload: Note { text: "abc".into() },
    };
    let byte_count = canonical_candidate(&candidate).unwrap().len() as u64;
    let mut full = RangeHasher::new(&scope, parent.clone(), 1, byte_count).unwrap();
    let mut resumed =
        RangeHasher::from_checkpoint(&scope, &parent, &full.checkpoint().unwrap()).unwrap();
    full.push_candidate(&candidate).unwrap();
    resumed.push_candidate(&candidate).unwrap();
    resumed =
        RangeHasher::from_checkpoint(&scope, &parent, &resumed.checkpoint().unwrap()).unwrap();
    assert_eq!(full.finish().unwrap(), resumed.finish().unwrap());
    let bad = Scope::<Notes> {
        collections: [Collection::Zebra].into_iter().collect(),
    };
    let state = RangeHasher::new(&scope, parent.clone(), 0, 0)
        .unwrap()
        .checkpoint()
        .unwrap();
    assert!(RangeHasher::<Notes>::from_checkpoint(&bad, &parent, &state).is_err());
    let mut json: serde_json::Value = serde_json::from_slice(&state).unwrap();
    json["candidates_seen"] = 3.into();
    assert!(
        RangeHasher::<Notes>::from_checkpoint(&scope, &parent, &serde_json::to_vec(&json).unwrap())
            .is_err()
    );
}
