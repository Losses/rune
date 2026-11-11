//! Pure planning for share publication, base cuts, and own-prefix collection.
use super::{BaseRef, Checkpoint, DevicePrefix, ObjectKey, SegmentRef, parse_key};
use crate::protocol::{
    Event, MAX_NUMBER, StoreIdentity, SyncModel, event_digest, initial_anchor, next_anchor,
};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlannerError {
    Invalid(String),
}

impl std::fmt::Display for PlannerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "share planning failed: {message}"),
        }
    }
}

impl std::error::Error for PlannerError {}

impl From<String> for PlannerError {
    fn from(value: String) -> Self {
        Self::Invalid(value)
    }
}

impl From<&str> for PlannerError {
    fn from(value: &str) -> Self {
        Self::Invalid(value.to_owned())
    }
}

pub const BULK_GZIP_LIMIT: u64 = 16 * 1024 * 1024;
pub const KEEPER_ACTIVE_MS: u64 = 7 * 24 * 60 * 60 * 1000;
pub const BASE_CADENCE_MS: u64 = 30 * 24 * 60 * 60 * 1000;
pub const BASE_MAX_AGE_MS: u64 = 45 * 24 * 60 * 60 * 1000;
pub const SEGMENT_RETENTION_MS: u64 = 30 * 24 * 60 * 60 * 1000;
pub const OBJECT_UNREFERENCED_MS: u64 = 24 * 60 * 60 * 1000;
pub const SUPERSEDED_BASE_RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1000;
pub const RETIREMENT_MS: u64 = 180 * 24 * 60 * 60 * 1000;

#[derive(Clone, Debug)]
/// A local journal assignment and its SH4 relay origin, if it has one.
pub struct JournalEvent<M: SyncModel> {
    /// The local journal sequence, used to advance the export pin.
    pub local_sequence: u64,
    /// The immutable assignment to publish.
    pub event: Event<M>,
    /// The peer that relayed the assignment into this journal; `None` means local authoring.
    pub origin: Option<StoreIdentity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Whether a newest base exists and, if it does, its compressed size.
pub enum BaseSize {
    Absent,
    GzipBytes(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// State of the local journal export pin.
pub enum ExportCursorStatus {
    /// The incremental journal range is still pinned.
    Current,
    /// The pin was dropped; publish a base instead of segments.
    NeedsBase,
}

#[derive(Clone, Debug)]
/// Caller-supplied state needed to plan one immutable share publication.
pub struct PublishInput<'a, M: SyncModel> {
    /// Identity of the local share stream.
    pub identity: &'a StoreIdentity,
    /// Last local journal sequence committed by an earlier share head.
    pub after_cursor: u64,
    /// Journal entries read after `after_cursor`, including origin metadata.
    pub entries: &'a [JournalEvent<M>],
    /// Replica ids used by known share streams.
    pub share_stream_ids: &'a BTreeSet<Uuid>,
    /// Local replica ids advertised by known share heads.
    pub share_local_replicas: &'a BTreeSet<Uuid>,
    /// Current share-stream head checkpoint.
    pub head: Checkpoint,
    /// Segment references currently named by the head.
    pub existing_segments: &'a [SegmentRef],
    /// Size of the newest live base, when present.
    pub newest_base_gzip_bytes: BaseSize,
    /// Whether the local export pin was dropped and requires a base.
    pub export_cursor: ExportCursorStatus,
    /// Maximum number of selected events to place in one L0 segment.
    pub max_events_per_segment: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// An event assigned its share sequence and anchor-chain value.
pub struct NumberedEvent<M: SyncModel> {
    pub local_sequence: u64,
    pub share_sequence: u64,
    pub event: Event<M>,
    pub digest: crate::protocol::Digest,
    pub anchor: crate::protocol::Digest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// One planned L0 segment's range and exact gzip estimate.
pub struct SegmentGroup {
    pub first: u64,
    pub last: u64,
    pub previous_anchor: crate::protocol::Digest,
    pub last_anchor: crate::protocol::Digest,
    pub compressed_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A roll-up input is either an uploaded segment or a prior planned segment.
pub enum RollupInput {
    /// A segment already present in the current head.
    Existing(SegmentRef),
    /// A segment created earlier in this publication, identified by its level and range.
    Planned {
        level: u8,
        first: u64,
        last: u64,
        previous_anchor: crate::protocol::Digest,
        last_anchor: crate::protocol::Digest,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// One immutable 16-to-1 segment replacement.
pub struct RollupPlan {
    pub level: u8,
    pub first: u64,
    pub last: u64,
    pub previous_anchor: crate::protocol::Digest,
    pub last_anchor: crate::protocol::Digest,
    pub inputs: Vec<RollupInput>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Why publication must replace the incremental tail with a base.
pub enum BulkReason {
    ExportCursorNeedsBase,
    GzipLimitExceeded,
    BaseFractionExceeded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Complete result of evaluating the next share publication.
pub enum PublishDecision<M: SyncModel> {
    NothingToPublish {
        export_cursor: u64,
    },
    CutBase {
        reason: BulkReason,
        covers_seq: u64,
    },
    PublishSegments {
        events: Vec<NumberedEvent<M>>,
        segments: Vec<SegmentGroup>,
        rollups: Vec<RollupPlan>,
        compressed_bytes: u64,
        export_cursor: u64,
    },
}

/// Select journal events and calculate the complete immutable publication plan.
///
/// The caller performs storage operations and advances its export cursor only after
/// the planned head update has committed.
pub fn plan_publish<M: SyncModel>(
    input: PublishInput<'_, M>,
) -> Result<PublishDecision<M>, PlannerError> {
    if input.max_events_per_segment == 0 {
        return Err("segment event limit must be positive".into());
    }
    let mut selected = Vec::new();
    let mut export_cursor = input.after_cursor;
    for entry in input
        .entries
        .iter()
        .filter(|entry| entry.local_sequence > input.after_cursor)
    {
        if entry.local_sequence <= export_cursor {
            return Err("journal entries are not strictly increasing".into());
        }
        export_cursor = entry.local_sequence;
        let include = entry.origin.as_ref().is_none_or(|origin| {
            !input.share_stream_ids.contains(&origin.replica)
                && !input.share_local_replicas.contains(&origin.replica)
        });
        if include {
            selected.push((entry.local_sequence, entry.event.clone()));
        }
    }
    if selected.is_empty() {
        return if input.export_cursor == ExportCursorStatus::NeedsBase {
            Ok(PublishDecision::CutBase {
                reason: BulkReason::ExportCursorNeedsBase,
                covers_seq: next_base_sequence(input.head.seq)?,
            })
        } else {
            Ok(PublishDecision::NothingToPublish { export_cursor })
        };
    }

    let mut seq = input.head.seq;
    let start_anchor = if seq == 0 {
        initial_anchor(input.identity).map_err(|error| error.to_string())?
    } else {
        input.head.anchor
    };
    let mut anchor = start_anchor;
    let mut numbered = Vec::with_capacity(selected.len());
    for (local_sequence, event) in selected {
        seq = seq
            .checked_add(1)
            .ok_or_else(|| "share sequence exhausted".to_owned())?;
        let digest = event_digest(&event).map_err(|error| error.to_string())?;
        let collection = match &event {
            Event::Key(key) => &key.key.collection,
            Event::Merge(merge) => &merge.collection,
        };
        anchor =
            next_anchor::<M>(anchor, seq, collection, digest).map_err(|error| error.to_string())?;
        numbered.push(NumberedEvent {
            local_sequence,
            share_sequence: seq,
            event,
            digest,
            anchor,
        });
    }

    let mut groups = Vec::new();
    for batch in numbered.chunks(input.max_events_per_segment) {
        let first_event = batch
            .first()
            .ok_or_else(|| "empty segment group".to_owned())?;
        let last_event = batch
            .last()
            .ok_or_else(|| "empty segment group".to_owned())?;
        let previous_anchor = if first_event.share_sequence == input.head.seq + 1 {
            start_anchor
        } else {
            numbered
                .iter()
                .find(|event| event.share_sequence + 1 == first_event.share_sequence)
                .map(|event| event.anchor)
                .ok_or_else(|| "missing preceding anchor".to_owned())?
        };
        let compressed_bytes = estimate_segment_gzip::<M>(batch, previous_anchor)?;
        groups.push(SegmentGroup {
            first: first_event.share_sequence,
            last: last_event.share_sequence,
            previous_anchor,
            last_anchor: last_event.anchor,
            compressed_bytes,
        });
    }
    let compressed_bytes: u64 = groups.iter().map(|group| group.compressed_bytes).sum();
    let bulk_result = bulk_assessment(
        compressed_bytes,
        input.newest_base_gzip_bytes,
        input.export_cursor,
    );
    if let BulkAssessment::Bulk(reason) = bulk_result {
        return Ok(PublishDecision::CutBase {
            reason,
            covers_seq: next_base_sequence(input.head.seq)?,
        });
    }

    let rollups = plan_rollups(&groups, input.existing_segments);
    Ok(PublishDecision::PublishSegments {
        events: numbered,
        segments: groups,
        rollups,
        compressed_bytes,
        export_cursor,
    })
}

fn next_base_sequence(head: u64) -> Result<u64, String> {
    head.checked_add(1)
        .filter(|sequence| *sequence <= MAX_NUMBER)
        .ok_or_else(|| "share sequence exhausted".into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BulkAssessment {
    Incremental,
    Bulk(BulkReason),
}

fn bulk_assessment(bytes: u64, base: BaseSize, cursor: ExportCursorStatus) -> BulkAssessment {
    if cursor == ExportCursorStatus::NeedsBase {
        BulkAssessment::Bulk(BulkReason::ExportCursorNeedsBase)
    } else if bytes > BULK_GZIP_LIMIT {
        BulkAssessment::Bulk(BulkReason::GzipLimitExceeded)
    } else if let BaseSize::GzipBytes(base_bytes) = base
        && bytes.saturating_mul(4) > base_bytes
    {
        BulkAssessment::Bulk(BulkReason::BaseFractionExceeded)
    } else {
        BulkAssessment::Incremental
    }
}

fn estimate_segment_gzip<M: SyncModel>(
    events: &[NumberedEvent<M>],
    previous: crate::protocol::Digest,
) -> Result<u64, String> {
    let links = events
        .iter()
        .map(|item| {
            let collection = match &item.event {
                Event::Key(key) => key.key.collection.clone(),
                Event::Merge(merge) => merge.collection.clone(),
            };
            super::SegmentLink {
                seq: item.share_sequence,
                collection,
                event: item.event.clone(),
            }
        })
        .collect();
    let segment = super::Segment {
        format: super::FORMAT_VERSION,
        prev_anchor: previous,
        links,
        last_anchor: events.last().map(|item| item.anchor).unwrap_or(previous),
    };
    super::encode_segment(&segment)
        .map(|bytes| bytes.len() as u64)
        .map_err(|error| error.to_string())
}

fn plan_rollups(groups: &[SegmentGroup], existing: &[SegmentRef]) -> Vec<RollupPlan> {
    #[derive(Clone)]
    struct Candidate {
        reference: SegmentRef,
        source: RollupInput,
    }

    let mut generated = Vec::new();
    let mut level_zero = Vec::new();
    for group in groups {
        let reference = SegmentRef {
            level: 0,
            first: group.first,
            last: group.last,
            prev_anchor: group.previous_anchor,
            last_anchor: group.last_anchor,
            name: String::new(),
            bytes: group.compressed_bytes,
        };
        level_zero.push(Candidate {
            source: RollupInput::Planned {
                level: 0,
                first: group.first,
                last: group.last,
                previous_anchor: group.previous_anchor,
                last_anchor: group.last_anchor,
            },
            reference,
        });
    }
    let mut plans = Vec::new();
    let mut level = 0u8;
    while level < 2 {
        let mut level_inputs: Vec<_> = existing
            .iter()
            .filter(|segment| segment.level == level)
            .cloned()
            .map(|reference| Candidate {
                source: RollupInput::Existing(reference.clone()),
                reference,
            })
            .collect();
        level_inputs.extend(
            generated
                .iter()
                .filter(|candidate: &&Candidate| candidate.reference.level == level)
                .cloned(),
        );
        if level == 0 {
            level_inputs.append(&mut level_zero);
        }
        level_inputs.sort_by_key(|item| item.reference.first);
        if level_inputs.len() < 16 {
            break;
        }
        let mut next = Vec::new();
        let mut run_start = 0;
        for run_end in 1..=level_inputs.len() {
            let contiguous = run_end < level_inputs.len()
                && level_inputs[run_end - 1].reference.last.checked_add(1)
                    == Some(level_inputs[run_end].reference.first);
            if contiguous {
                continue;
            }
            let run = &level_inputs[run_start..run_end];
            for batch in run.as_chunks::<16>().0 {
                let plan = RollupPlan {
                    level: level + 1,
                    first: batch[0].reference.first,
                    last: batch[15].reference.last,
                    previous_anchor: batch[0].reference.prev_anchor,
                    last_anchor: batch[15].reference.last_anchor,
                    inputs: batch.iter().map(|item| item.source.clone()).collect(),
                };
                let reference = SegmentRef {
                    level: level + 1,
                    first: plan.first,
                    last: plan.last,
                    prev_anchor: plan.previous_anchor,
                    last_anchor: plan.last_anchor,
                    name: String::new(),
                    bytes: batch.iter().map(|item| item.reference.bytes).sum(),
                };
                next.push(Candidate {
                    source: RollupInput::Planned {
                        level: level + 1,
                        first: plan.first,
                        last: plan.last,
                        previous_anchor: plan.previous_anchor,
                        last_anchor: plan.last_anchor,
                    },
                    reference,
                });
                plans.push(plan);
            }
            run_start = run_end;
        }
        if next.is_empty() {
            break;
        }
        generated.extend(next);
        level += 1;
    }
    plans
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A stream and the caller's last observed head-change time.
pub struct StreamActivity {
    pub stream_id: Uuid,
    pub last_head_change_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Reason a caller should schedule a base cut.
pub enum BaseCutReason {
    Cadence,
    TailFraction,
    MaximumAge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Newest base state supplied to the keeper planner.
pub enum NewestBase {
    /// No share base exists.
    Absent,
    /// A base exists and was created at this local time.
    Present { created_ms: u64, gzip_bytes: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The elected keeper, including the case where no active stream exists.
pub enum Keeper {
    /// There is no stream with a head change observed in the active window.
    None,
    /// The smallest active stream id.
    Stream(Uuid),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Keeper election and base-cut outcome for the supplied local observations.
pub enum KeeperDecision {
    NotKeeper { keeper: Keeper },
    Wait,
    CutBase(BaseCutReason),
}

/// Elect the smallest active stream and apply the base age and retained-tail rules.
pub fn plan_keeper(
    me: Uuid,
    now_ms: u64,
    activities: &[StreamActivity],
    newest_base: NewestBase,
    retained_tail_gzip_bytes: u64,
) -> KeeperDecision {
    let active: Vec<_> = activities
        .iter()
        .filter(|item| now_ms.saturating_sub(item.last_head_change_ms) <= KEEPER_ACTIVE_MS)
        .map(|item| item.stream_id)
        .collect();
    let keeper = active
        .iter()
        .copied()
        .min()
        .map_or(Keeper::None, Keeper::Stream);
    let (age, base_bytes) = match newest_base {
        NewestBase::Absent => {
            return if keeper == Keeper::Stream(me) {
                KeeperDecision::CutBase(BaseCutReason::Cadence)
            } else {
                KeeperDecision::NotKeeper { keeper }
            };
        }
        NewestBase::Present {
            created_ms,
            gzip_bytes,
        } => (now_ms.saturating_sub(created_ms), gzip_bytes),
    };
    if age > BASE_MAX_AGE_MS {
        return KeeperDecision::CutBase(BaseCutReason::MaximumAge);
    }
    if keeper != Keeper::Stream(me) {
        return KeeperDecision::NotKeeper { keeper };
    }
    if age >= BASE_CADENCE_MS {
        return KeeperDecision::CutBase(BaseCutReason::Cadence);
    }
    if retained_tail_gzip_bytes.saturating_mul(4) > base_bytes {
        return KeeperDecision::CutBase(BaseCutReason::TailFraction);
    }
    KeeperDecision::Wait
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Whether a manifest remains reachable by the head or retained history.
pub enum ManifestStatus {
    /// The base is live and available to readers.
    Live,
    /// The base has been superseded but is still retained.
    Superseded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// One retained manifest and the object names it owns by reference.
pub struct RetainedManifest {
    pub base: BaseRef,
    pub status: ManifestStatus,
    pub referenced_chunks: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// An own-prefix segment and its local retention timestamp.
pub struct OwnSegment {
    pub reference: SegmentRef,
    pub last_changed_ms: u64,
    pub key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Supersession state persisted for an own-prefix base.
pub enum SupersededSince {
    /// No covering live base has superseded this base.
    Current,
    /// Local time when a covering base was first observed.
    Since(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// An own-prefix base and the first local observation that superseded it.
pub struct OwnBase {
    pub base_id: String,
    pub key: String,
    pub superseded_since: SupersededSince,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Whether an orphan was not yet observed or was observed during a GC pass.
pub enum GcObservation {
    /// No GC pass has observed this object as unreferenced.
    NotObserved,
    /// Local time when a GC pass observed this object as unreferenced.
    ObservedAt(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// An own-prefix chunk and observations from two separate GC passes.
pub struct OwnChunk {
    pub name: String,
    pub key: String,
    pub first_unreferenced_ms: u64,
    pub second_unreferenced_observation: GcObservation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Objects protected by a base cut currently in progress.
pub struct InProgressCut {
    pub pinned_chunks: BTreeSet<String>,
    pub pinned_bases: BTreeSet<String>,
}

/// Whether a candidate head was successfully observed on every retirement poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetirementReadStatus {
    /// Every observation succeeded, so the unchanged interval is trustworthy.
    ConsistentlyReadable,
    /// A missing or unreadable observation broke the interval.
    Incomplete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A foreign stream considered for the design's narrow retirement rule.
pub struct RetirementCandidate {
    pub prefix: DevicePrefix,
    pub stream_id: Uuid,
    pub generation: Uuid,
    pub head_counter: u64,
    pub head: Checkpoint,
    /// Local time when this exact head was first observed unchanged.
    pub unchanged_since_ms: u64,
    /// Whether every intervening poll read this head successfully.
    pub read_status: RetirementReadStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Exact remote-head state the deleter must re-read before deleting a retired prefix.
pub struct RetirementPrecondition {
    pub prefix: DevicePrefix,
    pub stream_id: Uuid,
    pub generation: Uuid,
    pub expected_counter: u64,
    pub expected_head: Checkpoint,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Read-only state required to plan own-prefix collection.
pub struct GcInput<'a> {
    pub own_prefix: &'a DevicePrefix,
    pub now_ms: u64,
    pub head: Checkpoint,
    pub segments: &'a [OwnSegment],
    pub bases: &'a [OwnBase],
    pub manifests: &'a [RetainedManifest],
    pub active_acks: &'a BTreeMap<Uuid, u64>,
    pub in_progress: &'a InProgressCut,
    pub chunks: &'a [OwnChunk],
    pub retirements: &'a [RetirementCandidate],
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Planned own-prefix deletions, floor, and conditional retirement requests.
pub struct GcDecision {
    pub delete_segments: Vec<String>,
    pub delete_bases: Vec<String>,
    pub delete_chunks: Vec<String>,
    pub floor_base: FloorBaseSelection,
    pub floor: Checkpoint,
    pub retirement_preconditions: Vec<RetirementPrecondition>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Whether GC found a live base suitable for the own-stream floor.
pub enum FloorBaseSelection {
    /// No live retained base covers the local stream.
    None,
    /// Selected live base and its own-stream checkpoint.
    Selected { base_id: String, covers: Checkpoint },
}

/// Plan reclamation without performing any storage operation.
///
/// Every returned object deletion is checked against `own_prefix`. A foreign prefix
/// can only appear as a retirement precondition for a separate re-read step.
pub fn plan_gc(input: GcInput<'_>) -> Result<GcDecision, PlannerError> {
    let own_stream = input.own_prefix.to_string();
    let floor_manifest = input
        .manifests
        .iter()
        .filter(|manifest| manifest.status == ManifestStatus::Live)
        .filter_map(|manifest| {
            manifest
                .base
                .covers
                .get(&own_stream)
                .map(|cover| (manifest, cover))
        })
        .max_by_key(|(_, cover)| cover.seq);
    let floor_base = floor_manifest
        .map(|(manifest, cover)| FloorBaseSelection::Selected {
            base_id: manifest.base.base_id.clone(),
            covers: cover.clone(),
        })
        .unwrap_or(FloorBaseSelection::None);
    let floor_cover = floor_manifest.map(|(_, cover)| cover.seq).unwrap_or(0);

    let mut delete_segments = Vec::new();
    let mut retained = Vec::new();
    for segment in input.segments {
        if !matches!(parse_key(&segment.key), Ok(ObjectKey::Segment { prefix, .. }) if prefix == *input.own_prefix)
        {
            return Err("segment outside own prefix".into());
        }
        let covered = segment.reference.last <= floor_cover;
        let acknowledged = input
            .active_acks
            .values()
            .all(|ack| *ack >= segment.reference.last);
        let expired = input.now_ms.saturating_sub(segment.last_changed_ms) >= SEGMENT_RETENTION_MS;
        if covered && (acknowledged || expired) {
            delete_segments.push(segment.key.clone());
        } else {
            retained.push(segment);
        }
    }
    let floor_seq = retained
        .iter()
        .map(|segment| segment.reference.first)
        .min()
        .unwrap_or(input.head.seq);
    let floor = if let Some(segment) = retained
        .iter()
        .find(|segment| segment.reference.first == floor_seq)
    {
        Checkpoint {
            seq: floor_seq.saturating_sub(1),
            anchor: segment.reference.prev_anchor,
        }
    } else {
        input.head.clone()
    };

    let live_bases: Vec<_> = input
        .manifests
        .iter()
        .filter(|manifest| manifest.status == ManifestStatus::Live)
        .collect();
    let mut delete_bases = Vec::new();
    for base in input.bases {
        if !matches!(parse_key(&base.key), Ok(ObjectKey::Base { prefix, .. }) if prefix == *input.own_prefix)
        {
            return Err("base outside own prefix".into());
        }
        let superseded = live_bases.iter().any(|manifest| {
            manifest.base.base_id != base.base_id
                && input
                    .manifests
                    .iter()
                    .find(|candidate| candidate.base.base_id == base.base_id)
                    .is_some_and(|old| covers_at_least(&manifest.base.covers, &old.base.covers))
        });
        let pinned = input.in_progress.pinned_bases.contains(&base.base_id);
        if superseded
            && !pinned
            && matches!(base.superseded_since, SupersededSince::Since(since)
                if input.now_ms.saturating_sub(since) >= SUPERSEDED_BASE_RETENTION_MS)
        {
            delete_bases.push(base.key.clone());
        }
    }
    let referenced: BTreeSet<_> = input
        .manifests
        .iter()
        .flat_map(|manifest| manifest.referenced_chunks.iter().cloned())
        .collect();
    let mut delete_chunks = Vec::new();
    for chunk in input.chunks {
        if !matches!(parse_key(&chunk.key), Ok(ObjectKey::Chunk { prefix, .. }) if prefix == *input.own_prefix)
        {
            return Err("chunk outside own prefix".into());
        }
        if !referenced.contains(&chunk.name)
            && !input.in_progress.pinned_chunks.contains(&chunk.name)
            && matches!(chunk.second_unreferenced_observation, GcObservation::ObservedAt(second)
                if second <= input.now_ms
                    && second.saturating_sub(chunk.first_unreferenced_ms) >= OBJECT_UNREFERENCED_MS)
        {
            delete_chunks.push(chunk.key.clone());
        }
    }
    let mut retirement_preconditions = Vec::new();
    for retired in input.retirements {
        if retired.read_status != RetirementReadStatus::ConsistentlyReadable
            || input.now_ms.saturating_sub(retired.unchanged_since_ms) < RETIREMENT_MS
        {
            continue;
        }
        if retired.prefix != *input.own_prefix
            && live_bases.iter().any(|manifest| {
                manifest
                    .base
                    .covers
                    .get(&retired.prefix.to_string())
                    .is_some_and(|cover| cover.seq >= retired.head.seq)
            })
        {
            retirement_preconditions.push(RetirementPrecondition {
                prefix: retired.prefix.clone(),
                stream_id: retired.stream_id,
                generation: retired.generation,
                expected_counter: retired.head_counter,
                expected_head: retired.head.clone(),
            });
        }
    }
    Ok(GcDecision {
        delete_segments,
        delete_bases,
        delete_chunks,
        floor_base,
        floor,
        retirement_preconditions,
    })
}

fn covers_at_least(a: &BTreeMap<String, Checkpoint>, b: &BTreeMap<String, Checkpoint>) -> bool {
    b.iter().all(|(stream, checkpoint)| {
        a.get(stream).is_some_and(|candidate| {
            candidate.seq > checkpoint.seq
                || (candidate.seq == checkpoint.seq && candidate.anchor == checkpoint.anchor)
        })
    })
}

#[cfg(test)]
mod tests;
