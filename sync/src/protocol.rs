//! Neutral synchronization core.
use serde::de;
use serde::ser::Error;
mod codec;
mod coordinator;
mod memory;
#[allow(dead_code)]
mod memory_core;
mod merge;
mod portable_hash;
pub use merge::plan_leaf_merge;
mod receiver;
pub use codec::*;
pub use coordinator::{Coordinator, PassUsage, SessionAdvance, SessionLimits};
pub use memory::MemorySyncStore;
pub use portable_hash::PortableHasher;
pub use receiver::{
    CompletionAdmission, MAX_RECEIVER_EXPANDED_BYTES, MAX_RECEIVER_METADATA_BYTES,
    MAX_RECEIVER_METADATA_ITEMS, MAX_RECEIVER_RETRY_RECEIPTS, PreparedEvent, PreparedLeaf,
    PromotionAdmission, PromotionStep, RangeAdmission, ReceiverLedger,
};
use std::{collections::BTreeSet, fmt::Debug, future::Future, pin::Pin, sync::Arc};
pub use uuid::Uuid;
pub type Digest = [u8; 32];
pub type Result<T> = std::result::Result<T, SyncError>;
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
pub const MAX_NUMBER: u64 = i64::MAX as u64;
pub const MAX_CANDIDATE_BYTES: u64 = 64 * 1024;
pub const MAX_TAG_BYTES: usize = 128;
pub const MAX_COLLECTIONS: usize = 256;
pub const DEFAULT_PAGE_ITEMS: u32 = 128;
pub const MAX_PAGE_ITEMS: u32 = 1024;
pub const DEFAULT_PAGE_BYTES: u64 = 1024 * 1024;
pub const MAX_PAGE_BYTES: u64 = 4 * 1024 * 1024;
pub const DEFAULT_LOGICAL_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_LOGICAL_BYTES: u64 = 64 * 1024 * 1024;
pub const DEFAULT_STAGE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_STAGE_BYTES: u64 = 512 * 1024 * 1024;
pub const DEFAULT_STAGES: u32 = 4;
pub const MAX_STAGES: u32 = 16;
pub const DEFAULT_PROJECTION_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_PROJECTION_BYTES: u64 = 16 * 1024 * 1024;
pub const DEFAULT_PROJECTION_ITEMS: u32 = 128;
pub const MAX_PROJECTION_ITEMS: u32 = 1024;
pub const DEFAULT_LOGICAL_ITEMS: u64 = 65_536;
pub const MAX_LOGICAL_ITEMS: u64 = 1_048_576;
pub const MAX_FRAGMENTS: u32 = 65_536;
pub const DEFAULT_ROUND_REQUESTS: u32 = 1024;
pub const MAX_ROUND_REQUESTS: u32 = 16_384;
pub const DEFAULT_ROUND_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_ROUND_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_DEPTH: u32 = 64;
pub const DEFAULT_ROWS: u64 = 1_000_000;
pub const MAX_ROWS: u64 = 16_000_000;
pub const DEFAULT_MUTATIONS: u64 = 65_536;
pub const MAX_MUTATIONS: u64 = 1_048_576;
pub const DEFAULT_SNAPSHOT_TTL_MS: u64 = 600_000;
pub const MAX_SNAPSHOT_TTL_MS: u64 = 86_400_000;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SyncError {
    Invalid(String),
    IncompatibleModel,
    ClockExhausted,
    LimitTooSmall { required: u64 },
    LogicalLimit,
    StageQuota,
    WorkLimit,
    RetryAt(ReceiveProgress),
    NeedsReconciliation(HistoryStatus),
    SnapshotExpired,
    ConflictingDot,
    ConflictingFragment,
    MissingDependency(#[serde(with = "digest_serde")] Digest),
    IncompleteCoverage,
    StaleWork,
    Cancelled,
    Unauthorized,
    Forbidden,
    NamespaceNotServed,
    StorageBusy,
    Storage(String),
    Transport(String),
}
fn serialize_collection<M: SyncModel, S: serde::Serializer>(
    value: &M::Collection,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    let tag = M::collection_tag(value);
    if tag.is_empty()
        || tag.len() > MAX_TAG_BYTES
        || !M::collections().contains(value)
        || M::collection_from_tag(tag).as_ref() != Some(value)
    {
        return Err(Error::custom("invalid collection tag"));
    }
    serializer.serialize_str(tag)
}
fn deserialize_collection<'de, M: SyncModel, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<M::Collection, D::Error> {
    let tag = <String as serde::Deserialize>::deserialize(deserializer)?;
    if tag.is_empty() || tag.len() > MAX_TAG_BYTES {
        return Err(de::Error::custom("invalid collection tag"));
    }
    let collection =
        M::collection_from_tag(&tag).ok_or_else(|| de::Error::custom("unknown collection tag"))?;
    if M::collection_tag(&collection) != tag || !M::collections().contains(&collection) {
        return Err(de::Error::custom("unregistered collection tag"));
    }
    Ok(collection)
}
fn serialize_collections<M: SyncModel, S: serde::Serializer>(
    values: &BTreeSet<M::Collection>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    if values.is_empty() || values.len() > MAX_COLLECTIONS {
        return Err(Error::custom("invalid collection scope"));
    }
    let mut tags: Vec<_> = values.iter().map(M::collection_tag).collect();
    tags.sort_unstable();
    if tags.windows(2).any(|p| p[0] == p[1])
        || values.iter().any(|v| {
            !M::collections().contains(v)
                || M::collection_from_tag(M::collection_tag(v)).as_ref() != Some(v)
        })
    {
        return Err(Error::custom("invalid collection registry"));
    }
    serde::Serialize::serialize(&tags, serializer)
}
fn deserialize_collections<'de, M: SyncModel, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeSet<M::Collection>, D::Error> {
    let tags = <Vec<String> as serde::Deserialize>::deserialize(deserializer)?;
    if tags.is_empty() || tags.len() > MAX_COLLECTIONS || tags.windows(2).any(|p| p[0] >= p[1]) {
        return Err(de::Error::custom("invalid collection scope order"));
    }
    let mut values = BTreeSet::new();
    for tag in tags {
        let value = M::collection_from_tag(&tag)
            .ok_or_else(|| de::Error::custom("unknown collection tag"))?;
        if tag.is_empty()
            || tag.len() > MAX_TAG_BYTES
            || M::collection_tag(&value) != tag
            || !M::collections().contains(&value)
            || !values.insert(value)
        {
            return Err(de::Error::custom("invalid collection tag"));
        }
    }
    Ok(values)
}
mod bytes_serde {
    use serde::de;
    use serde::ser::Error;
    pub fn serialize<S: serde::Serializer>(
        bytes: &[u8],
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let mut out = String::with_capacity(bytes.len().saturating_mul(2));
        use std::fmt::Write;
        for b in bytes {
            write!(&mut out, "{b:02x}").map_err(Error::custom)?;
        }
        serializer.serialize_str(&out)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Vec<u8>, D::Error> {
        let s = <String as serde::Deserialize>::deserialize(deserializer)?;
        if s.len() % 2 != 0
            || !s
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(de::Error::custom("invalid canonical hex"));
        }
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(de::Error::custom))
            .collect()
    }
}
mod digest_serde {
    use serde::de::Error;
    pub fn serialize<S: serde::Serializer>(
        value: &super::Digest,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        super::bytes_serde::serialize(value, serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<super::Digest, D::Error> {
        let value = super::bytes_serde::deserialize(deserializer)?;
        value
            .try_into()
            .map_err(|_| Error::custom("invalid digest length"))
    }
}

/// Collection is typed in callers; stable tags determine encoding and ordering.
pub trait SyncModel: Clone + Debug + PartialEq + Eq + Send + Sync + 'static {
    type Collection: Clone + Debug + PartialEq + Eq + Ord + Send + Sync + 'static;
    type Payload: Clone
        + Debug
        + PartialEq
        + Eq
        + serde::Serialize
        + serde::de::DeserializeOwned
        + Send
        + Sync
        + 'static;
    fn model_id() -> &'static str;
    fn schema_id() -> &'static str;
    fn collections() -> Vec<Self::Collection>;
    fn collection_tag(collection: &Self::Collection) -> &'static str;
    fn collection_from_tag(tag: &str) -> Option<Self::Collection>;
    fn validate(key: &ObjectKey<Self>, value: &Option<Self::Payload>) -> Result<()>;
    fn validate_revision(
        _key: &ObjectKey<Self>,
        _value: &Option<Self::Payload>,
        _revision: &Revision,
    ) -> Result<()> {
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelId {
    pub model: String,
    pub schema: String,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreIdentity {
    pub namespace: Uuid,
    pub model: ModelId,
    pub replica: Uuid,
    pub generation: Uuid,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalAuthor {
    pub author: Uuid,
    pub writer_incarnation: Uuid,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct ObjectKey<M: SyncModel> {
    #[serde(
        serialize_with = "serialize_collection::<M, _>",
        deserialize_with = "deserialize_collection::<M, _>"
    )]
    pub collection: M::Collection,
    pub id: Uuid,
}
impl<M: SyncModel> Ord for ObjectKey<M> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        M::collection_tag(&self.collection)
            .as_bytes()
            .cmp(M::collection_tag(&other.collection).as_bytes())
            .then_with(|| self.id.cmp(&other.id))
    }
}
impl<M: SyncModel> PartialOrd for ObjectKey<M> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(deny_unknown_fields)]
pub struct Dot {
    pub writer_incarnation: Uuid,
    pub counter: u64,
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(deny_unknown_fields)]
pub struct Revision {
    pub counter: u64,
    pub actor: Uuid,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct Scope<M: SyncModel> {
    #[serde(
        serialize_with = "serialize_collections::<M, _>",
        deserialize_with = "deserialize_collections::<M, _>"
    )]
    pub collections: BTreeSet<M::Collection>,
}
/// Canonical accepted values always have a payload.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct Candidate<M: SyncModel> {
    pub model: ModelId,
    pub namespace: Uuid,
    pub key: ObjectKey<M>,
    pub dot: Dot,
    pub revision: Revision,
    pub payload: M::Payload,
}
/// Convenience projection/command receipt, never a stored removed candidate.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct Record<M: SyncModel> {
    pub key: ObjectKey<M>,
    pub value: Option<M::Payload>,
    pub revision: Revision,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Observed {
    AllCurrent,
    Exact(BTreeSet<Dot>),
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct KeyDelta<M: SyncModel> {
    pub key: ObjectKey<M>,
    pub author: Revision,
    pub removed: Vec<Dot>,
    pub added: Vec<Candidate<M>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CounterInterval {
    pub first: u64,
    pub last_inclusive: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriterContext {
    pub writer_incarnation: Uuid,
    pub prefix: u64,
    pub exceptions: Vec<CounterInterval>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactContext {
    pub writers: Vec<WriterContext>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct KeyRange<M: SyncModel> {
    #[serde(
        serialize_with = "serialize_collection::<M, _>",
        deserialize_with = "deserialize_collection::<M, _>"
    )]
    pub collection: M::Collection,
    pub lower: Option<Uuid>,
    pub upper: Option<Uuid>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct CandidateCursor<M: SyncModel> {
    pub key: ObjectKey<M>,
    pub dot: Dot,
}
impl<M: SyncModel> Ord for CandidateCursor<M> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key
            .cmp(&other.key)
            .then_with(|| self.dot.cmp(&other.dot))
    }
}
impl<M: SyncModel> PartialOrd for CandidateCursor<M> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestKind {
    KeyEvent,
    MergeEvent,
    Context,
    ContextRoot,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct CollectionContext<M: SyncModel> {
    #[serde(
        serialize_with = "serialize_collection::<M, _>",
        deserialize_with = "deserialize_collection::<M, _>"
    )]
    pub collection: M::Collection,
    pub context: ContextRef,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextRoot<M: SyncModel> {
    pub scope: Scope<M>,
    pub contexts: Vec<CollectionContext<M>>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestRef {
    pub kind: ManifestKind,
    #[serde(with = "digest_serde")]
    pub manifest_digest: Digest,
    #[serde(with = "digest_serde")]
    pub content_digest: Digest,
    pub encoded_bytes: u64,
    pub item_count: u64,
    pub fragment_count: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextRef {
    #[serde(with = "digest_serde")]
    pub digest: Digest,
    pub manifest: ManifestRef,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct MergeDelta<M: SyncModel> {
    #[serde(
        serialize_with = "serialize_collection::<M, _>",
        deserialize_with = "deserialize_collection::<M, _>"
    )]
    pub collection: M::Collection,
    pub added: Vec<Candidate<M>>,
    pub observed: ExactContext,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    deny_unknown_fields,
    bound(serialize = "", deserialize = "")
)]
pub enum Event<M: SyncModel> {
    Key(KeyDelta<M>),
    Merge(Box<MergeDelta<M>>),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct ProjectionChange<M: SyncModel> {
    pub key: ObjectKey<M>,
    pub before: Option<Candidate<M>>,
    pub after: Option<Candidate<M>>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct ProjectionBatch<M: SyncModel> {
    pub changes: Vec<ProjectionChange<M>>,
    pub causal_changes: u64,
    pub pending: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiveProgress {
    pub sequence: u64,
    #[serde(with = "digest_serde")]
    pub anchor: Digest,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointKey {
    pub namespace: Uuid,
    pub source_replica: Uuid,
    pub source_generation: Uuid,
    #[serde(with = "digest_serde")]
    pub scope_id: Digest,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum HistoryStatus {
    Uninitialized {
        head: ReceiveProgress,
    },
    Continuous {
        head: ReceiveProgress,
        floor: ReceiveProgress,
    },
    GenerationChanged {
        current: Uuid,
    },
    AnchorMismatch {
        checkpoint: ReceiveProgress,
    },
    HeadBehind {
        head: u64,
    },
    BelowFloor {
        floor: ReceiveProgress,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageLimits {
    pub max_scanned: u32,
    pub max_bytes: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkLimits {
    pub max_requests: u32,
    pub max_bytes: u64,
    pub max_depth: u32,
    pub max_rows_scanned: u64,
    pub max_mutations: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageLimits {
    pub max_total_bytes: u64,
    pub max_stages: u32,
    pub max_event_bytes: u64,
    pub max_event_items: u64,
    pub max_fragment_bytes: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyLimits {
    pub max_projection_items: u32,
    pub max_projection_bytes: u64,
    pub max_mutations: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct PageRequest<M: SyncModel> {
    pub source: StoreIdentity,
    pub scope: Scope<M>,
    pub after: ReceiveProgress,
    pub through: ReceiveProgress,
    pub limits: PageLimits,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields,
    bound(serialize = "", deserialize = "")
)]
pub enum EventRef<M: SyncModel> {
    Inline(Event<M>),
    Fragmented(ManifestRef),
}
/// One commitment for every scanned sequence, even outside the requested scope.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct ScanLink<M: SyncModel> {
    pub sequence: u64,
    #[serde(
        serialize_with = "serialize_collection::<M, _>",
        deserialize_with = "deserialize_collection::<M, _>"
    )]
    pub collection: M::Collection,
    #[serde(with = "digest_serde")]
    pub event_digest: Digest,
    pub event: Option<EventRef<M>>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct ChangePage<M: SyncModel> {
    pub request: PageRequest<M>,
    pub next: ReceiveProgress,
    pub done: bool,
    pub links: Vec<ScanLink<M>>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields,
    bound(serialize = "", deserialize = "")
)]
pub enum PageRead<M: SyncModel> {
    Page(ChangePage<M>),
    FragmentNeeded {
        request: PageRequest<M>,
        sequence: u64,
        #[serde(
            serialize_with = "serialize_collection::<M, _>",
            deserialize_with = "deserialize_collection::<M, _>"
        )]
        collection: M::Collection,
        #[serde(with = "digest_serde")]
        event_digest: Digest,
        manifest: ManifestRef,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fragment {
    pub manifest: ManifestRef,
    pub ordinal: u32,
    pub offset: u64,
    #[serde(with = "bytes_serde")]
    pub bytes: Vec<u8>,
    #[serde(with = "digest_serde")]
    pub digest: Digest,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields,
    bound(serialize = "", deserialize = "")
)]
// Keep the journal request inline in the public admission API.
#[allow(clippy::large_enum_variant)]
pub enum StageOrigin<M: SyncModel> {
    Journal {
        request: PageRequest<M>,
        sequence: u64,
        #[serde(
            serialize_with = "serialize_collection::<M, _>",
            deserialize_with = "deserialize_collection::<M, _>"
        )]
        collection: M::Collection,
        #[serde(with = "digest_serde")]
        event_digest: Digest,
    },
    SnapshotContext {
        session: Uuid,
        snapshot_id: Uuid,
        #[serde(
            serialize_with = "serialize_collection::<M, _>",
            deserialize_with = "deserialize_collection::<M, _>"
        )]
        collection: M::Collection,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct StageDescriptor<M: SyncModel> {
    pub origin: StageOrigin<M>,
    pub manifest: ManifestRef,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FragmentProgress {
    pub stage: Uuid,
    pub next_ordinal: u32,
    pub next_offset: u64,
    pub complete: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct ApplyResult<M: SyncModel> {
    pub progress: ReceiveProgress,
    pub projection: ProjectionBatch<M>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct CandidatePage<M: SyncModel> {
    pub values: Vec<Candidate<M>>,
    pub next: Option<Dot>,
    pub done: bool,
}

/// Minimal receiving capability. It exposes no local authoring or lookup.
pub trait ReceiveStore<M: SyncModel>: Send + Sync {
    fn identity(&self) -> BoxFuture<'_, StoreIdentity>;
    fn checkpoint<'a>(&'a self, key: &'a CheckpointKey) -> BoxFuture<'a, Option<ReceiveProgress>>;
    fn apply_page<'a>(
        &'a self,
        page: &'a ChangePage<M>,
        limits: ApplyLimits,
    ) -> BoxFuture<'a, ApplyResult<M>>;
    fn begin_stage(
        &self,
        descriptor: StageDescriptor<M>,
        limits: StageLimits,
    ) -> BoxFuture<'_, FragmentProgress>;
    fn stage_fragment(&self, stage: Uuid, fragment: Fragment) -> BoxFuture<'_, FragmentProgress>;
    fn stage_status(&self, stage: Uuid) -> BoxFuture<'_, FragmentProgress>;
    fn abandon_stage(&self, stage: Uuid) -> BoxFuture<'_, ()>;
    fn abandon_origin(&self, origin: StageOrigin<M>) -> BoxFuture<'_, ()>;
    fn commit_fragmented_event(
        &self,
        key: CheckpointKey,
        sequence: u64,
        stage: Uuid,
        limits: ApplyLimits,
    ) -> BoxFuture<'_, ApplyResult<M>>;
    fn finish_session(&self, key: CheckpointKey, through: ReceiveProgress) -> BoxFuture<'_, ()>;
}
/// All mutations are atomic. A durable adapter also exposes concrete *_on
/// methods accepting its caller's existing transaction for joined projection.
pub trait SyncStore<M: SyncModel>: ReceiveStore<M> {
    fn local_author(&self) -> BoxFuture<'_, LocalAuthor>;
    fn head(&self) -> BoxFuture<'_, ReceiveProgress>;
    fn history_status<'a>(
        &'a self,
        key: &'a CheckpointKey,
        progress: Option<ReceiveProgress>,
    ) -> BoxFuture<'a, HistoryStatus>;
    fn get<'a>(&'a self, key: &'a ObjectKey<M>) -> BoxFuture<'a, Option<Record<M>>>;
    fn candidates<'a>(
        &'a self,
        key: &'a ObjectKey<M>,
        after: Option<Dot>,
        limits: PageLimits,
    ) -> BoxFuture<'a, CandidatePage<M>>;
    fn write_local(&self, key: ObjectKey<M>, value: Option<M::Payload>)
    -> BoxFuture<'_, Record<M>>;
    fn write_observed(
        &self,
        key: ObjectKey<M>,
        value: Option<M::Payload>,
        observed: Observed,
    ) -> BoxFuture<'_, Record<M>>;
    fn read_page(&self, request: PageRequest<M>) -> BoxFuture<'_, PageRead<M>>;
    fn read_fragment(
        &self,
        manifest: ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    ) -> BoxFuture<'_, Fragment>;
}
pub trait SyncPeer<M: SyncModel>: Send + Sync {
    fn identity(&self) -> BoxFuture<'_, StoreIdentity>;
    fn head(&self) -> BoxFuture<'_, ReceiveProgress>;
    fn history_status<'a>(
        &'a self,
        key: &'a CheckpointKey,
        progress: Option<ReceiveProgress>,
    ) -> BoxFuture<'a, HistoryStatus>;
    fn read_page(&self, request: PageRequest<M>) -> BoxFuture<'_, PageRead<M>>;
    fn read_fragment(
        &self,
        manifest: ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    ) -> BoxFuture<'_, Fragment>;
}
pub struct StorePeer<M: SyncModel>(pub Arc<dyn SyncStore<M>>);
impl<M: SyncModel> SyncPeer<M> for StorePeer<M> {
    fn identity(&self) -> BoxFuture<'_, StoreIdentity> {
        self.0.identity()
    }
    fn head(&self) -> BoxFuture<'_, ReceiveProgress> {
        self.0.head()
    }
    fn history_status<'a>(
        &'a self,
        key: &'a CheckpointKey,
        progress: Option<ReceiveProgress>,
    ) -> BoxFuture<'a, HistoryStatus> {
        self.0.history_status(key, progress)
    }
    fn read_page(&self, request: PageRequest<M>) -> BoxFuture<'_, PageRead<M>> {
        self.0.read_page(request)
    }
    fn read_fragment(
        &self,
        manifest: ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    ) -> BoxFuture<'_, Fragment> {
        self.0.read_fragment(manifest, ordinal, max_bytes)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct RangeSummary<M: SyncModel> {
    pub snapshot_id: Uuid,
    pub range: KeyRange<M>,
    pub candidate_count: u64,
    pub candidate_bytes: u64,
    #[serde(with = "digest_serde")]
    pub aggregate: Digest,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct SnapshotDescriptor<M: SyncModel> {
    pub source: StoreIdentity,
    pub scope: Scope<M>,
    pub snapshot_id: Uuid,
    pub head: ReceiveProgress,
    pub roots: Vec<RangeSummary<M>>,
    pub contexts: Vec<CollectionContext<M>>,
    pub expires_unix_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct LeafPage<M: SyncModel> {
    pub snapshot_id: Uuid,
    pub range: KeyRange<M>,
    pub after: Option<CandidateCursor<M>>,
    pub next: Option<CandidateCursor<M>>,
    pub done: bool,
    pub candidates: Vec<Candidate<M>>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkCursor {
    pub operation_id: Uuid,
    pub generation: Uuid,
    pub position: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum WorkOutcome<T> {
    Ready(T),
    Incomplete { cursor: WorkCursor },
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageProgress {
    pub ledger_revision: u64,
    pub outstanding_ranges: u64,
    pub staged_leaves: u64,
    pub complete: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct CoveragePage<M: SyncModel> {
    pub ledger_revision: u64,
    pub after: u64,
    pub next: u64,
    pub done: bool,
    pub outstanding: Vec<KeyRange<M>>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct LeafProgress<M: SyncModel> {
    pub last: Option<CandidateCursor<M>>,
    pub complete: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromoteProgress {
    pub done: bool,
    pub causal_changes: u64,
}
/// Source leases survive multiple processes; expensive construction/summary are resumable.
pub trait SnapshotSource<M: SyncModel>: Send + Sync {
    fn release_request(&self, request_id: Uuid) -> BoxFuture<'_, ()>;
    fn close_snapshot(&self, snapshot: Uuid) -> BoxFuture<'_, ()>;
    fn cancel_snapshot_build(&self, cursor: WorkCursor) -> BoxFuture<'_, ()>;
    fn open_snapshot(
        &self,
        request_id: Uuid,
        scope: Scope<M>,
        limits: WorkLimits,
        resume: Option<WorkCursor>,
    ) -> BoxFuture<'_, WorkOutcome<SnapshotDescriptor<M>>>;
    fn summarize(
        &self,
        snapshot: Uuid,
        range: KeyRange<M>,
        limits: WorkLimits,
        resume: Option<WorkCursor>,
    ) -> BoxFuture<'_, WorkOutcome<RangeSummary<M>>>;
    fn split(
        &self,
        snapshot: Uuid,
        range: KeyRange<M>,
        limits: WorkLimits,
        resume: Option<WorkCursor>,
    ) -> BoxFuture<'_, WorkOutcome<Vec<KeyRange<M>>>>;
    fn leaf(
        &self,
        snapshot: Uuid,
        range: KeyRange<M>,
        after: Option<CandidateCursor<M>>,
        limits: PageLimits,
    ) -> BoxFuture<'_, LeafPage<M>>;
    fn read_fragment(
        &self,
        snapshot: Uuid,
        manifest: ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    ) -> BoxFuture<'_, Fragment>;
}
/// Uncommitted receive stages may expire on restart; accepted receipts do not.
pub trait SnapshotSink<M: SyncModel>: Send + Sync {
    fn cancel_reconciliation(&self, session: Uuid) -> BoxFuture<'_, ()>;
    fn discard_range(&self, session: Uuid, range: KeyRange<M>) -> BoxFuture<'_, ()>;
    fn begin_reconciliation(
        &self,
        request_id: Uuid,
        remote: SnapshotDescriptor<M>,
        local: SnapshotDescriptor<M>,
    ) -> BoxFuture<'_, Uuid>;
    fn coverage(
        &self,
        session: Uuid,
        ledger_revision: Option<u64>,
        after: u64,
        limits: PageLimits,
    ) -> BoxFuture<'_, CoveragePage<M>>;
    fn split_range(
        &self,
        session: Uuid,
        parent: KeyRange<M>,
        children: Vec<RangeSummary<M>>,
    ) -> BoxFuture<'_, CoverageProgress>;
    fn accept_equal_range(
        &self,
        session: Uuid,
        source: RangeSummary<M>,
    ) -> BoxFuture<'_, CoverageProgress>;
    fn leaf_progress(&self, session: Uuid, range: KeyRange<M>) -> BoxFuture<'_, LeafProgress<M>>;
    fn stage_leaf_page(&self, session: Uuid, page: LeafPage<M>) -> BoxFuture<'_, LeafProgress<M>>;
    fn commit_complete_range(
        &self,
        session: Uuid,
        range: KeyRange<M>,
        limits: ApplyLimits,
    ) -> BoxFuture<'_, (CoverageProgress, ProjectionBatch<M>)>;
    fn promote_step(&self, session: Uuid, limits: ApplyLimits) -> BoxFuture<'_, PromoteProgress>;
    fn complete_reconciliation(&self, session: Uuid) -> BoxFuture<'_, ReceiveProgress>;
}
impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SyncError {}

mod journal;
pub use journal::*;

mod fragments;
pub use fragments::*;
