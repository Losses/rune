use serde::{Deserialize, Serialize};
use sync::protocol::*;
#[derive(Clone, Debug, PartialEq, Eq)]
struct Notes;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Collection {
    Notes,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Note;
impl SyncModel for Notes {
    type Collection = Collection;
    type Payload = Note;
    fn model_id() -> &'static str {
        "n"
    }
    fn schema_id() -> &'static str {
        "s"
    }
    fn collections() -> Vec<Collection> {
        vec![Collection::Notes]
    }
    fn collection_tag(_: &Collection) -> &'static str {
        "notes"
    }
    fn collection_from_tag(s: &str) -> Option<Collection> {
        (s == "notes").then_some(Collection::Notes)
    }
    fn validate(_: &ObjectKey<Self>, _: &Option<Note>) -> Result<()> {
        Ok(())
    }
}
fn descriptor() -> StageDescriptor<Notes> {
    let mut m = ManifestRef {
        kind: ManifestKind::Leaf,
        manifest_digest: [0; 32],
        content_digest: content_digest(b"abcdef").unwrap(),
        encoded_bytes: 6,
        item_count: 1,
        fragment_count: 2,
    };
    m.manifest_digest = manifest_digest(&m).unwrap();
    StageDescriptor {
        origin: StageOrigin::Snapshot {
            session: Uuid::from_u128(1),
            snapshot_id: Uuid::from_u128(2),
            range: KeyRange {
                collection: Collection::Notes,
                lower: None,
                upper: None,
            },
        },
        manifest: m,
    }
}
fn fragment(m: &ManifestRef, ordinal: u32, offset: u64, bytes: &[u8]) -> Fragment {
    let mut f = Fragment {
        manifest: m.clone(),
        ordinal,
        offset,
        bytes: bytes.to_vec(),
        digest: [0; 32],
    };
    f.digest = fragment_digest(&f).unwrap();
    f
}
fn limits() -> StageLimits {
    StageLimits {
        max_total_bytes: 6,
        max_stages: 1,
        max_event_bytes: 6,
        max_event_items: 10,
        max_fragment_bytes: 4,
    }
}
#[test]
fn staged_fragments_reorder_duplicate_conflict_restart_and_quota() {
    let desc = descriptor();
    let m = desc.manifest.clone();
    let mut pool = FragmentStages::default();
    let id = Uuid::from_u128(3);
    pool.begin(id, desc.clone(), limits()).unwrap();
    assert!(matches!(
        pool.begin(Uuid::from_u128(4), desc, limits()),
        Err(SyncError::StageQuota)
    ));
    let second = fragment(&m, 1, 3, b"def");
    assert!(!pool.stage(id, second.clone()).unwrap().complete);
    let mut resumed = pool.clone();
    assert_eq!(resumed.status(id).unwrap().next_ordinal, 0);
    let first = fragment(&m, 0, 0, b"abc");
    assert!(resumed.stage(id, first.clone()).unwrap().complete);
    assert!(resumed.stage(id, first).unwrap().complete);
    assert_eq!(resumed.assembled(id).unwrap(), b"abcdef");
    let conflicting = fragment(&m, 0, 0, b"xyz");
    assert!(matches!(
        resumed.stage(id, conflicting),
        Err(SyncError::ConflictingFragment)
    ));
    let mut corrupt = second;
    corrupt.digest[0] ^= 1;
    assert!(resumed.stage(id, corrupt).is_err());
    assert_eq!(resumed.assembled(id).unwrap(), b"abcdef");
    resumed.abandon(id).unwrap();
}

#[test]
fn dependency_manifests_share_one_admission_and_one_graph_limit() {
    let mut pool = FragmentStages::<Notes>::default();
    let limits = StageLimits {
        max_total_bytes: 100,
        max_stages: 1,
        max_event_bytes: 30,
        max_event_items: 10,
        max_fragment_bytes: 4,
    };
    let original = descriptor();
    for i in 0..5u8 {
        let mut desc = original.clone();
        desc.manifest.content_digest = content_digest(&[i; 6]).unwrap();
        desc.manifest.manifest_digest = manifest_digest(&desc.manifest).unwrap();
        pool.begin(Uuid::from_u128(10 + i as u128), desc, limits)
            .unwrap();
    }
    let mut excess = original.clone();
    excess.manifest.content_digest = content_digest(b"another").unwrap();
    excess.manifest.manifest_digest = manifest_digest(&excess.manifest).unwrap();
    assert!(matches!(
        pool.begin(Uuid::from_u128(20), excess, limits),
        Err(SyncError::LogicalLimit)
    ));
    let mut other = original.clone();
    if let StageOrigin::Snapshot { session, .. } = &mut other.origin {
        *session = Uuid::from_u128(99)
    }
    assert!(matches!(
        pool.begin(Uuid::from_u128(21), other, limits),
        Err(SyncError::StageQuota)
    ));
    let ids = pool.abandon_group(Uuid::from_u128(10)).unwrap();
    assert_eq!(ids.len(), 5);
    pool.begin(Uuid::from_u128(22), original, limits).unwrap();
}
#[test]
fn manifest_layout_is_deterministic_and_rejects_empty_tail() {
    let mut m = descriptor().manifest;
    m.encoded_bytes = 100000;
    m.fragment_count = 3;
    assert_eq!(fragment_bounds(&m, 0).unwrap(), (0, 33334));
    assert_eq!(fragment_bounds(&m, 1).unwrap(), (33334, 66668));
    assert_eq!(fragment_bounds(&m, 2).unwrap(), (66668, 100000));
    assert!(fragment_bounds(&m, 3).is_err());
    m.encoded_bytes = 10;
    m.fragment_count = 6;
    assert!(fragment_bounds(&m, 0).is_err());
}
