//! Data structures for the share object formats.
use crate::protocol::{Digest, Event, KeyRange, RangeSummary, SyncModel};
use serde::{Deserialize, Serialize, de::Error as DeError};
use std::{collections::BTreeMap, result::Result as StdResult};
use uuid::Uuid;

pub const FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryFile {
    pub format: u32,
    pub library_id: Uuid,
    pub model: String,
    pub schema: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub seq: u64,
    pub anchor: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FloorBase {
    pub owner_prefix: String,
    pub base_id: String,
    pub covers_seq: u64,
    pub covers_anchor: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SegmentRef {
    pub level: u8,
    pub first: u64,
    pub last: u64,
    pub prev_anchor: Digest,
    pub last_anchor: Digest,
    pub name: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaseRef {
    pub base_id: String,
    pub covers: BTreeMap<String, Checkpoint>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Head {
    pub format: u32,
    pub library_id: Uuid,
    pub stream: Uuid,
    pub generation: Uuid,
    pub local_replica: Uuid,
    pub writer_session: Uuid,
    pub counter: u64,
    pub head: Checkpoint,
    pub floor: Checkpoint,
    pub floor_base: FloorBase,
    pub segments: Vec<SegmentRef>,
    pub bases: Vec<BaseRef>,
    pub acks: BTreeMap<String, u64>,
    pub local_cursor: u64,
    pub retired: Vec<String>,
    pub updated_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct SegmentLink<M: SyncModel> {
    pub seq: u64,
    #[serde(
        serialize_with = "serialize_collection::<M, _>",
        deserialize_with = "deserialize_collection::<M, _>"
    )]
    pub collection: M::Collection,
    pub event: Event<M>,
}

fn serialize_collection<M: SyncModel, S: serde::Serializer>(
    collection: &M::Collection,
    serializer: S,
) -> StdResult<S::Ok, S::Error> {
    serializer.serialize_str(M::collection_tag(collection))
}

fn deserialize_collection<'de, M: SyncModel, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> StdResult<M::Collection, D::Error> {
    let tag = <String as Deserialize>::deserialize(deserializer)?;
    M::collection_from_tag(&tag)
        .filter(|collection| M::collection_tag(collection) == tag)
        .ok_or_else(|| DeError::custom("unknown collection tag"))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct Segment<M: SyncModel> {
    pub format: u32,
    pub prev_anchor: Digest,
    pub links: Vec<SegmentLink<M>>,
    pub last_anchor: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct BucketSummary<M: SyncModel> {
    pub range: KeyRange<M>,
    pub candidate_count: u64,
    pub candidate_bytes: u64,
    pub aggregate: Digest,
    pub chunk: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct CollectionSummary<M: SyncModel> {
    pub root: RangeSummary<M>,
    pub bucket_count: u32,
    pub buckets: Vec<BucketSummary<M>>,
    pub context_fragments: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(serialize = "", deserialize = ""))]
pub struct BaseManifest<M: SyncModel> {
    pub format: u32,
    pub covers: BTreeMap<String, Checkpoint>,
    pub collections: BTreeMap<String, CollectionSummary<M>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub format: u32,
    pub name: String,
    pub canonical_bytes: Vec<u8>,
}
