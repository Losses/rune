//! Local binding and pull orchestration for a library's share source.
use crate::sync::model::RuneModel;
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, ExprTrait, TransactionTrait,
    sea_query::{Alias, ColumnDef, Expr, OnConflict, Query, Table},
};
use serde::{Deserialize, Serialize};
use std::cmp;
use std::fmt;
use std::result;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
};
use sync::{
    Coordinator, Digest, Event, ReceiveProgress, SessionAdvance, StoreIdentity, SyncError,
    SyncModel,
    protocol::{CheckpointKey, Scope, digest_serde},
    share::{
        BaseManifest, BaseRef, BaseSize, Checkpoint, ConditionalGet, DevicePrefix,
        ExportCursorStatus, FORMAT_VERSION, FloorBase, FloorBaseSelection, FormatError, GcInput,
        GcObservation, Head, InProgressCut, JournalEvent, LibraryFile, ManifestStatus,
        ObjectGcState, ObjectKey, ObjectStore, ObjectTag, ObjectValue, OwnBase, OwnChunk,
        OwnSegment, PublishDecision, PublishInput, RetainedManifest, RetirementCandidate,
        RetirementPrecondition, RetirementReadStatus, RollupInput, SEGMENT_RETENTION_MS, Segment,
        SegmentLink, SegmentRef, SelectedBase, SharePeer, StoreError, SupersededSince, base_key,
        chunk_key, decode_head, decode_library, decode_manifest, decode_segment, encode_head,
        encode_segment, head_key, library_key, parse_key, plan_gc, plan_publish, segment_key,
    },
};
use sync_orm::causal_export_cursors::{advance_on, create_on, read_on};
use sync_orm::causal_progress::adopt_checkpoint_on;
use sync_orm::{
    base_exporter::{BaseExportOptions, export_base},
    causal_capabilities::CausalEndpoint,
};
use tokio::sync::{Mutex, MutexGuard};
use uuid::Uuid;

const STATE_TABLE: &str = "device_share_state";
const LIBRARY_BINDINGS_TABLE: &str = "device_share_bindings";
// Libraries whose library.json this device wrote with a plain PUT and has not yet seen hold.
const UNCONFIRMED_CREATIONS_TABLE: &str = "device_share_unconfirmed_creations";

/// Per-library state lives in local SQLite and is never part of the sync model.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShareState {
    pub library_id: Uuid,
    pub stream: Uuid,
    pub generation: Uuid,
    pub writer_session: Uuid,
    pub head_counter: u64,
    pub local_cursor: u64,
    #[serde(with = "object_key_map")]
    pub observed_objects: BTreeMap<ObjectKey, ObservedObject>,
    /// Deletions of retired prefixes that this device has begun and not finished.
    #[serde(default)]
    pub retirement_cleanups: Vec<RetirementCleanup>,
    /// What push and garbage collection use of this device's base manifests and segments, so
    /// that they are not downloaded again. A record belongs to the bytes whose tag it carries:
    /// it is taken only while that tag is the one observed last (`recorded`), and the collection
    /// and the base cut take it only once the store has confirmed that tag (`own_contents`).
    /// Garbage collection drops the records of objects that are no longer in the folder.
    #[serde(default, with = "object_key_map")]
    pub own_objects: BTreeMap<ObjectKey, OwnRecord>,
    pub chunk_cache_path: PathBuf,
}

/// What a keeper found in a retired prefix before its first delete there: the head that the
/// retirement covers and every object listed, all of them covered by the retirement. A later pass
/// finishes the deletion from it when an earlier one stopped half way.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetirementCleanup {
    pub prefix: DevicePrefix,
    pub head_counter: u64,
    pub head: Checkpoint,
    pub objects: BTreeSet<String>,
}

impl RetirementCleanup {
    fn precondition(&self) -> RetirementPrecondition {
        RetirementPrecondition {
            prefix: self.prefix.clone(),
            stream_id: self.prefix.stream,
            generation: self.prefix.generation,
            expected_counter: self.head_counter,
            expected_head: self.head.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedObject {
    pub tag: ObjectTag,
    pub first_seen_ms: i64,
    pub last_change_ms: i64,
    pub gc_state: ObjectGcState,
}

/// What push and garbage collection use of a base manifest or segment in this device's prefix,
/// with the tag of the bytes it was read from, or that this device's write of them returned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnRecord {
    /// The tag of the bytes `contents` describes. For bytes this device wrote, it is the tag the
    /// write returned, taken for the tag of the bytes sent: the device does not read them back.
    pub tag: ObjectTag,
    /// What was read from those bytes, or written as them.
    pub contents: OwnContents,
}

/// What push and garbage collection use of one of this device's base manifests or segments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnContents {
    /// A base manifest.
    Manifest {
        /// Compressed bytes of the chunks and context fragments that the manifest references.
        /// `None` for a manifest recorded before this size was kept.
        content_gzip_bytes: Option<u64>,
        /// The bucket chunks and context fragments the manifest references.
        chunks: BTreeSet<String>,
        /// Whether the manifest references any candidate chunks, which tells a base with no
        /// catalog content from one whose content is only context fragments. `None` for a
        /// manifest recorded before this flag was kept.
        #[serde(default)]
        has_candidates: Option<bool>,
        /// The compressed bytes of the manifest itself, which is all a base without candidate
        /// chunks contributes to the share. State recorded before this name was kept stores it
        /// as `bytes`.
        #[serde(alias = "bytes")]
        manifest_gzip_bytes: u64,
    },
    /// A segment: what a reference to it holds besides its level, range and name.
    Segment {
        /// The anchor of the checkpoint before the segment's first event.
        #[serde(with = "digest_serde")]
        prev_anchor: Digest,
        /// The anchor of the checkpoint at its last event.
        #[serde(with = "digest_serde")]
        last_anchor: Digest,
        /// The size of the stored segment.
        bytes: u64,
    },
}

impl OwnContents {
    fn manifest<M: SyncModel>(
        content_gzip_bytes: u64,
        manifest_gzip_bytes: u64,
        manifest: &BaseManifest<M>,
    ) -> Self {
        Self::Manifest {
            content_gzip_bytes: Some(content_gzip_bytes),
            chunks: manifest_chunk_names(manifest).collect(),
            has_candidates: Some(manifest_has_candidates(manifest)),
            manifest_gzip_bytes,
        }
    }

    /// Decodes a base manifest or segment read from this device's prefix. A manifest is checked
    /// against its base id.
    fn decode<M: SyncModel>(object: &ObjectKey, bytes: &[u8]) -> SharePushResult<Self> {
        let failed = |error: FormatError| SharePushError::Failed(error.to_string());
        match object {
            ObjectKey::Base { base_id, .. } => {
                let manifest = decode_manifest::<M>(bytes, base_id).map_err(failed)?;
                Ok(Self::Manifest {
                    content_gzip_bytes: None,
                    chunks: manifest_chunk_names(&manifest).collect(),
                    has_candidates: Some(manifest_has_candidates(&manifest)),
                    manifest_gzip_bytes: bytes.len() as u64,
                })
            }
            ObjectKey::Segment { .. } => {
                let segment = decode_segment::<M>(bytes).map_err(failed)?;
                Ok(Self::Segment {
                    prev_anchor: segment.prev_anchor,
                    last_anchor: segment.last_anchor,
                    bytes: bytes.len() as u64,
                })
            }
            _ => Err(SharePushError::Failed(
                "only base manifests and segments are recorded".into(),
            )),
        }
    }
}

/// Serializes a map keyed by share object as a list of pairs, since JSON keys are strings.
mod object_key_map {
    use super::ObjectKey;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;
    use std::result;

    pub fn serialize<S, V>(
        objects: &BTreeMap<ObjectKey, V>,
        serializer: S,
    ) -> result::Result<S::Ok, S::Error>
    where
        S: Serializer,
        V: Serialize,
    {
        objects.iter().collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, D, V>(
        deserializer: D,
    ) -> result::Result<BTreeMap<ObjectKey, V>, D::Error>
    where
        D: Deserializer<'de>,
        V: Deserialize<'de>,
    {
        let entries = Vec::<(ObjectKey, V)>::deserialize(deserializer)?;
        Ok(entries.into_iter().collect())
    }
}

#[derive(Clone, Default)]
pub struct ShareWriter(Arc<Mutex<()>>);

impl ShareWriter {
    pub(crate) async fn lock(&self) -> MutexGuard<'_, ()> {
        self.0.lock().await
    }
}

impl ShareState {
    pub fn new(local_replica: Uuid, library_id: Uuid, cache_path: PathBuf) -> Result<Self> {
        ensure!(
            !local_replica.is_nil() && !library_id.is_nil(),
            "share identities cannot be nil"
        );
        Ok(Self {
            library_id,
            stream: Uuid::new_v5(&local_replica, b"rune-share"),
            generation: Uuid::new_v4(),
            writer_session: Uuid::new_v4(),
            head_counter: 0,
            local_cursor: 0,
            observed_objects: BTreeMap::new(),
            retirement_cleanups: Vec::new(),
            own_objects: BTreeMap::new(),
            chunk_cache_path: cache_path,
        })
    }

    /// Starts retention timers from now when the persisted clock began in the future.
    pub fn normalize_future_times(&mut self, now_ms: i64) {
        for observed in self.observed_objects.values_mut() {
            if observed.first_seen_ms > now_ms {
                observed.first_seen_ms = now_ms;
            }
            if observed.last_change_ms > now_ms {
                observed.last_change_ms = now_ms;
            }
            match &mut observed.gc_state {
                ObjectGcState::Current => (),
                ObjectGcState::Unreferenced {
                    first_seen_ms,
                    second_observation,
                } => {
                    if *first_seen_ms > now_ms {
                        *first_seen_ms = now_ms;
                    }
                    let normalized_now = Ord::max(now_ms, 0) as u64;
                    if let GcObservation::ObservedAt(second_seen_ms) = second_observation
                        && *second_seen_ms > normalized_now
                    {
                        *second_observation = GcObservation::ObservedAt(normalized_now);
                    }
                }
                ObjectGcState::Superseded { since_ms } if *since_ms > now_ms => {
                    *since_ms = now_ms;
                }
                ObjectGcState::Superseded { .. } => (),
            }
        }
    }

    pub fn observe_tag(&mut self, object: ObjectKey, tag: ObjectTag, now_ms: i64) {
        match self.observed_objects.entry(object) {
            Entry::Vacant(entry) => {
                entry.insert(ObservedObject {
                    tag,
                    first_seen_ms: now_ms,
                    last_change_ms: now_ms,
                    gc_state: ObjectGcState::Current,
                });
            }
            Entry::Occupied(mut entry) => {
                let observed = entry.get_mut();
                if observed.tag != tag {
                    observed.tag = tag;
                    observed.last_change_ms = now_ms;
                    observed.gc_state = ObjectGcState::Current;
                }
            }
        }
    }

    /// The record of one of this device's objects, if it belongs to the bytes whose tag this device
    /// observed last. Whether the store still holds those bytes is for the caller to confirm.
    pub(crate) fn recorded(&self, object: &ObjectKey) -> Option<&OwnContents> {
        let observed = self.observed_objects.get(object)?;
        self.own_objects
            .get(object)
            .filter(|record| record.tag == observed.tag)
            .map(|record| &record.contents)
    }

    /// Records what was read from, or written as, the bytes of one of this device's objects whose
    /// tag is `tag`.
    fn record(&mut self, object: ObjectKey, tag: ObjectTag, contents: OwnContents) {
        self.own_objects.insert(object, OwnRecord { tag, contents });
    }

    fn rotate_generation(&mut self) {
        self.generation = Uuid::new_v4();
        self.writer_session = Uuid::new_v4();
        self.head_counter = 0;
        self.local_cursor = 0;
        self.observed_objects.clear();
        self.own_objects.clear();
    }
}

/// Load the local share state or create its first generation for this library binding.
pub async fn load_or_create_state(
    db: &DatabaseConnection,
    local_replica: Uuid,
    library_id: Uuid,
    cache_path: PathBuf,
    now_ms: i64,
) -> Result<ShareState> {
    ensure_tables(db).await?;
    let query = Query::select()
        .column(Alias::new("state_json"))
        .from(Alias::new(STATE_TABLE))
        .and_where(Expr::col(Alias::new("library_id")).eq(library_id.to_string()))
        .to_owned();
    if let Some(row) = db.query_one(&query).await? {
        let encoded: String = row.try_get_by_index(0)?;
        let mut state: ShareState =
            serde_json::from_str(&encoded).context("decode local share state")?;
        ensure!(
            state.library_id == library_id,
            "local share binding changed"
        );
        state.normalize_future_times(now_ms);
        save_state(db, &state).await?;
        return Ok(state);
    }
    let state = ShareState::new(local_replica, library_id, cache_path)?;
    save_state(db, &state).await?;
    Ok(state)
}

/// Persist local share metadata, which is excluded from RuneModel and the causal journal.
pub async fn save_state(db: &DatabaseConnection, state: &ShareState) -> Result<()> {
    ensure_tables(db).await?;
    let encoded = serde_json::to_string(state)?;
    let insert = Query::insert()
        .into_table(Alias::new(STATE_TABLE))
        .columns([Alias::new("library_id"), Alias::new("state_json")])
        .values_panic([state.library_id.to_string().into(), encoded.into()])
        .on_conflict(
            OnConflict::column(Alias::new("library_id"))
                .update_column(Alias::new("state_json"))
                .to_owned(),
        )
        .to_owned();
    db.execute(&insert).await?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShareCapabilities {
    /// Set only after the caller's conditional-create probe succeeds.
    pub conditional_create: bool,
    /// Set only after the store's conditional overwrite probe succeeds.
    pub conditional_head_write: bool,
    /// Set only after the store's conditional GET probe succeeds.
    pub conditional_get: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SharePullError {
    #[error("share binding changed or library.json is missing")]
    ShareChanged,
    #[error("share storage temporarily unavailable: {0}")]
    Transient(String),
    #[error("share object is missing or invalid: {0}")]
    Invalid(String),
    #[error("share base object is missing: {0}")]
    MissingObject(String),
    #[error("share store error: {0}")]
    Store(String),
    #[error("share is read-only")]
    ReadOnly,
    #[error("share credentials rejected")]
    CredentialsRejected,
    #[error("share pull cancelled")]
    Cancelled,
}

pub type SharePullResult<T> = result::Result<T, SharePullError>;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SharePushError {
    #[error("share needs a base cut before incremental publication")]
    NeedsBase,
    #[error("share head changed during publication")]
    StaleHead,
    #[error("share is read-only")]
    ReadOnly,
    #[error("share credentials rejected")]
    CredentialsRejected,
    #[error("required share object is missing")]
    MissingObject,
    #[error("share binding changed")]
    ShareChanged,
    #[error("share push failed: {0}")]
    Failed(String),
}

pub type SharePushResult<T> = result::Result<T, SharePushError>;

pub struct PublishJournalRequest<'a, M: SyncModel> {
    pub db: &'a DatabaseConnection,
    pub state: &'a mut ShareState,
    pub writer: &'a ShareWriter,
    pub store: &'a dyn ObjectStore,
    pub identity: &'a StoreIdentity,
    pub current_head: &'a Head,
    pub entries: &'a [JournalEvent<M>],
    pub acknowledgements: &'a BTreeMap<DevicePrefix, u64>,
    pub share_stream_ids: &'a BTreeSet<Uuid>,
    pub share_local_replicas: &'a BTreeSet<Uuid>,
    pub newest_base: BaseSize,
    pub cursor_status: ExportCursorStatus,
    pub capabilities: ShareCapabilities,
    pub max_events_per_segment: usize,
}

pub struct SharePullRequest<'a, M: SyncModel> {
    pub db: &'a DatabaseConnection,
    pub local: &'a dyn sync::ReceiveStore<M>,
    pub sink: &'a dyn sync::SnapshotSink<M>,
    pub local_source: &'a dyn sync::SnapshotSource<M>,
    pub store: Arc<dyn ObjectStore>,
    pub library_id: Uuid,
    pub scope: Scope<M>,
    pub limits: sync::SessionLimits,
    pub cache_path: PathBuf,
    pub capabilities: ShareCapabilities,
}

pub struct BaseCutRequest<'a, M: SyncModel> {
    pub db: &'a DatabaseConnection,
    pub state: &'a mut ShareState,
    pub writer: &'a ShareWriter,
    pub store: &'a dyn ObjectStore,
    pub identity: &'a StoreIdentity,
    pub source: &'a CausalEndpoint<M>,
    pub current_head: Option<&'a Head>,
    pub covers: BTreeMap<String, Checkpoint>,
    pub capabilities: ShareCapabilities,
}

/// Export and commit one base while holding the same prefix writer as journal publication.
pub async fn cut_base<M: SyncModel>(request: BaseCutRequest<'_, M>) -> SharePushResult<Head> {
    let BaseCutRequest {
        db,
        state,
        writer,
        store,
        identity,
        source,
        current_head,
        mut covers,
        capabilities,
    } = request;
    if identity.namespace != state.library_id {
        return Err(SharePushError::ShareChanged);
    }
    let guard = writer.lock().await;
    let prefix = DevicePrefix {
        stream: state.stream,
        generation: state.generation,
    };
    validate_share_library_inner::<M>(db, store, state.library_id, capabilities, true)
        .await
        .map_err(|error| SharePushError::Failed(error.to_string()))?;
    let latest = match current_head {
        Some(expected) => {
            let latest = refresh_own_head(store, &prefix).await?;
            if latest.counter != expected.counter
                || latest.writer_session != expected.writer_session
            {
                return Err(SharePushError::StaleHead);
            }
            covers.insert(
                prefix.to_string(),
                Checkpoint {
                    seq: latest.head.seq,
                    anchor: latest.head.anchor,
                },
            );
            Some(latest)
        }
        None => {
            for slot in [0, 1] {
                let key = head_key(&prefix.to_string(), slot)
                    .map_err(|error| SharePushError::Failed(error.to_string()))?;
                match store.get(&key).await {
                    Err(StoreError::NotFound) => (),
                    Ok(_) => return Err(SharePushError::StaleHead),
                    Err(error) => return Err(push_store_error(error)),
                }
            }
            None
        }
    };
    // A retained manifest must still be there and decode before anything of the new base is
    // written, as the chunks it names are not uploaded again.
    let now_ms = Utc::now().timestamp_millis();
    let mut retained_chunk_names = BTreeSet::new();
    for base in latest.iter().flat_map(|head| head.bases.iter()) {
        let object = ObjectKey::Base {
            prefix: prefix.clone(),
            base_id: base.base_id.clone(),
        };
        let key = base_key(&prefix.to_string(), &base.base_id)
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
        match own_contents::<M>(store, state, &object, &key, None, capabilities, now_ms).await? {
            Some(OwnContents::Manifest { chunks, .. }) => retained_chunk_names.extend(chunks),
            _ => return Err(SharePushError::MissingObject),
        }
    }
    let export = export_base(
        source,
        BaseExportOptions {
            own_prefix: prefix.to_string(),
            covers,
            retained_chunk_names,
            ..BaseExportOptions::default()
        },
    )
    .await
    .map_err(|error| SharePushError::Failed(error.to_string()))?;
    for (index, object) in export.objects.iter().enumerate() {
        let is_manifest = index + 1 == export.objects.len();
        let key = if is_manifest {
            base_key(&prefix.to_string(), &object.name)
        } else {
            chunk_key(&prefix.to_string(), &object.name)
        }
        .map_err(|error| SharePushError::Failed(error.to_string()))?;
        write_immutable_object(
            store,
            state,
            &key,
            &object.bytes,
            capabilities,
            "immutable base object name collision",
            is_manifest.then(|| {
                OwnContents::manifest(
                    export.content_gzip_bytes,
                    object.bytes.len() as u64,
                    &export.manifest,
                )
            }),
        )
        .await?;
    }
    let cover = export
        .manifest
        .covers
        .get(&prefix.to_string())
        .cloned()
        .ok_or_else(|| {
            SharePushError::Failed("base manifest omitted its owner checkpoint".into())
        })?;
    let mut next = latest.clone().unwrap_or_else(|| Head {
        format: FORMAT_VERSION,
        library_id: state.library_id,
        stream: state.stream,
        generation: state.generation,
        local_replica: identity.replica,
        writer_session: state.writer_session,
        counter: 0,
        head: cover.clone(),
        floor: cover.clone(),
        floor_base: FloorBase {
            owner_prefix: prefix.to_string(),
            base_id: export.base_id.clone(),
            covers_seq: cover.seq,
            covers_anchor: cover.anchor,
        },
        segments: Vec::new(),
        bases: Vec::new(),
        acks: BTreeMap::new(),
        local_cursor: state.local_cursor,
        retired: Vec::new(),
        updated_ms: 0,
    });
    next.counter = next
        .counter
        .checked_add(1)
        .ok_or_else(|| SharePushError::Failed("share head counter exhausted".into()))?;
    next.floor = cover.clone();
    next.head = cover.clone();
    next.floor_base = FloorBase {
        owner_prefix: prefix.to_string(),
        base_id: export.base_id.clone(),
        covers_seq: cover.seq,
        covers_anchor: cover.anchor,
    };
    next.bases.push(BaseRef {
        base_id: export.base_id,
        covers: export.manifest.covers,
    });
    next.segments.retain(|segment| segment.last > cover.seq);
    next.local_cursor = cover.seq;
    next.updated_ms = Ord::max(Utc::now().timestamp_millis(), 0) as u64;
    let written = if let Some(latest) = latest.as_ref() {
        write_head_retry(
            store,
            &prefix.to_string(),
            latest,
            &next,
            &guard,
            capabilities,
        )
        .await?
    } else {
        let key = head_key(&prefix.to_string(), 0)
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
        let bytes =
            encode_head(&next).map_err(|error| SharePushError::Failed(error.to_string()))?;
        let result = if capabilities.conditional_head_write {
            store.create(&key, &bytes).await
        } else {
            store.put(&key, &bytes).await
        };
        result.map_err(push_store_error)?;
        next
    };
    advance_export_cursor(
        db,
        identity.namespace,
        state.stream.as_bytes(),
        state.local_cursor,
        written.local_cursor,
    )
    .await?;
    state.head_counter = written.counter;
    state.local_cursor = written.local_cursor;
    save_state(db, state)
        .await
        .map_err(|error| SharePushError::Failed(error.to_string()))?;
    Ok(written)
}

pub(crate) async fn collect_own_prefix(
    db: &DatabaseConnection,
    state: &mut ShareState,
    writer: &ShareWriter,
    store: &dyn ObjectStore,
    head: &Head,
    other_heads: &BTreeMap<DevicePrefix, Head>,
    capabilities: ShareCapabilities,
) -> SharePushResult<Head> {
    let guard = writer.lock().await;
    let prefix = DevicePrefix {
        stream: state.stream,
        generation: state.generation,
    };
    let latest = refresh_own_head(store, &prefix).await?;
    if latest.counter != head.counter || latest.writer_session != head.writer_session {
        return Err(SharePushError::StaleHead);
    }
    let now_ms = Ord::max(Utc::now().timestamp_millis(), 0) as u64;
    state.normalize_future_times(now_ms as i64);
    let listed = store
        .list_prefix(&format!("devices/{prefix}/"))
        .await
        .map_err(push_store_error)?;
    // What observing each listed manifest and segment found, for reading them below.
    let mut listed_reads = BTreeMap::new();
    for key in &listed {
        let parsed = parse_key(key).map_err(|error| SharePushError::Failed(error.to_string()))?;
        let object_prefix = match &parsed {
            ObjectKey::Head { prefix, .. }
            | ObjectKey::Segment { prefix, .. }
            | ObjectKey::Base { prefix, .. }
            | ObjectKey::Chunk { prefix, .. } => prefix,
            ObjectKey::Probe => continue,
            _ => {
                return Err(SharePushError::Failed(
                    "invalid object in own prefix listing".into(),
                ));
            }
        };
        if object_prefix != &prefix {
            return Err(SharePushError::Failed(
                "own-prefix listing returned a foreign object".into(),
            ));
        }
        let read =
            observe_own_object(store, state, &parsed, key, capabilities, now_ms as i64).await?;
        if matches!(parsed, ObjectKey::Base { .. } | ObjectKey::Segment { .. }) {
            listed_reads.insert(parsed, read);
        }
    }
    // Records are kept only for the objects in the folder.
    state
        .own_objects
        .retain(|object, _| listed_reads.contains_key(object));

    let mut manifests = Vec::new();
    let mut own_bases = Vec::new();
    for base in &latest.bases {
        let key = base_key(&prefix.to_string(), &base.base_id)
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
        let object_key = ObjectKey::Base {
            prefix: prefix.clone(),
            base_id: base.base_id.clone(),
        };
        // A base of the head must be there and decode, or the collection plans nothing.
        let referenced_chunks = match own_contents::<RuneModel>(
            store,
            state,
            &object_key,
            &key,
            listed_reads.remove(&object_key),
            capabilities,
            now_ms as i64,
        )
        .await?
        {
            Some(OwnContents::Manifest { chunks, .. }) => chunks,
            _ => return Err(SharePushError::MissingObject),
        };
        let superseded = latest.bases.iter().any(|candidate| {
            candidate.base_id != base.base_id && covers_at_least(&candidate.covers, &base.covers)
        });
        let observed = state.observed_objects.get_mut(&object_key).ok_or_else(|| {
            SharePushError::Failed("base observation disappeared during collection".into())
        })?;
        let superseded_since = if superseded {
            let since = match observed.gc_state {
                ObjectGcState::Superseded { since_ms } => since_ms,
                _ => {
                    observed.gc_state = ObjectGcState::Superseded {
                        since_ms: now_ms as i64,
                    };
                    now_ms as i64
                }
            };
            SupersededSince::Since(Ord::max(since, 0) as u64)
        } else {
            observed.gc_state = ObjectGcState::Current;
            SupersededSince::Current
        };
        manifests.push(RetainedManifest {
            base: base.clone(),
            status: if superseded {
                ManifestStatus::Superseded
            } else {
                ManifestStatus::Live
            },
            referenced_chunks,
        });
        own_bases.push(OwnBase {
            base_id: base.base_id.clone(),
            key,
            superseded_since,
        });
    }

    let mut segments = Vec::new();
    for key in &listed {
        let ObjectKey::Segment {
            prefix: object_prefix,
            first,
            last,
            hash16,
        } = parse_key(key).map_err(|error| SharePushError::Failed(error.to_string()))?
        else {
            continue;
        };
        if object_prefix != prefix {
            continue;
        }
        let parsed = ObjectKey::Segment {
            prefix: prefix.clone(),
            first,
            last,
            hash16: hash16.clone(),
        };
        let last_changed_ms = state
            .observed_objects
            .get(&parsed)
            .map(|observed| Ord::max(observed.last_change_ms, 0) as u64)
            .unwrap_or(now_ms);
        let reference = if let Some(reference) = latest.segments.iter().find(|reference| {
            reference.first == first && reference.last == last && reference.name == hash16
        }) {
            // A segment of the head, whose reference the head holds. Bytes the listing downloaded
            // are recorded all the same, so that the segment is not downloaded again once it
            // leaves the head. Bytes that do not decode get no record: that later read reports
            // them, as before.
            if let Some(OwnObjectRead::Downloaded(value)) = listed_reads.remove(&parsed)
                && let Ok(contents) = OwnContents::decode::<RuneModel>(&parsed, &value.bytes)
            {
                state.record(parsed.clone(), value.tag, contents);
            }
            reference.clone()
        } else {
            // A segment that left the head, after a roll-up or a base cut.
            match own_contents::<RuneModel>(
                store,
                state,
                &parsed,
                key,
                listed_reads.remove(&parsed),
                capabilities,
                now_ms as i64,
            )
            .await?
            {
                Some(OwnContents::Segment {
                    prev_anchor,
                    last_anchor,
                    bytes,
                }) => SegmentRef {
                    level: 0,
                    first,
                    last,
                    prev_anchor,
                    last_anchor,
                    name: hash16,
                    bytes,
                },
                // Gone since the listing.
                _ => continue,
            }
        };
        segments.push(OwnSegment {
            reference,
            last_changed_ms,
            key: key.clone(),
        });
    }
    let referenced: BTreeSet<String> = manifests
        .iter()
        .flat_map(|manifest| manifest.referenced_chunks.iter().cloned())
        .collect();
    let mut chunks = Vec::new();
    for key in &listed {
        let ObjectKey::Chunk {
            prefix: object_prefix,
            digest,
        } = parse_key(key).map_err(|error| SharePushError::Failed(error.to_string()))?
        else {
            continue;
        };
        if object_prefix != prefix {
            continue;
        }
        let observation = state
            .observed_objects
            .get_mut(&ObjectKey::Chunk {
                prefix: prefix.clone(),
                digest: digest.clone(),
            })
            .ok_or_else(|| {
                SharePushError::Failed("chunk observation disappeared during collection".into())
            })?;
        if referenced.contains(&digest) {
            observation.gc_state = ObjectGcState::Current;
            continue;
        }
        let (first, second_observation) = match observation.gc_state {
            ObjectGcState::Unreferenced {
                first_seen_ms,
                second_observation,
            } => {
                let second_observation = match second_observation {
                    GcObservation::ObservedAt(_) => second_observation,
                    GcObservation::NotObserved if now_ms as i64 > first_seen_ms => {
                        GcObservation::ObservedAt(now_ms)
                    }
                    GcObservation::NotObserved => GcObservation::NotObserved,
                };
                (first_seen_ms, second_observation)
            }
            _ => (now_ms as i64, GcObservation::NotObserved),
        };
        observation.gc_state = ObjectGcState::Unreferenced {
            first_seen_ms: first,
            second_observation,
        };
        chunks.push(OwnChunk {
            name: digest,
            key: key.clone(),
            first_unreferenced_ms: Ord::max(first, 0) as u64,
            second_unreferenced_observation: second_observation,
        });
    }
    let active_acks = other_heads
        .iter()
        .filter(|(device, _)| {
            **device != prefix
                && observed_head_last_change(state, device)
                    .is_some_and(|changed| now_ms.saturating_sub(changed) <= SEGMENT_RETENTION_MS)
        })
        .map(|(device, _)| {
            (
                device.stream,
                latest.acks.get(&device.to_string()).copied().unwrap_or(0),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let retirements = other_heads
        .iter()
        .filter(|(device, _)| **device != prefix)
        .filter_map(|(device, other)| {
            let unchanged_since_ms = observed_head_last_change(state, device)?;
            Some(RetirementCandidate {
                prefix: device.clone(),
                stream_id: other.stream,
                generation: other.generation,
                head_counter: other.counter,
                head: Checkpoint {
                    seq: other.head.seq,
                    anchor: other.head.anchor,
                },
                unchanged_since_ms,
                read_status: RetirementReadStatus::ConsistentlyReadable,
            })
        })
        .collect::<Vec<_>>();
    let decision = plan_gc(GcInput {
        own_prefix: &prefix,
        now_ms,
        head: Checkpoint {
            seq: latest.head.seq,
            anchor: latest.head.anchor,
        },
        segments: &segments,
        bases: &own_bases,
        manifests: &manifests,
        active_acks: &active_acks,
        in_progress: &InProgressCut {
            pinned_chunks: BTreeSet::new(),
            pinned_bases: BTreeSet::new(),
        },
        chunks: &chunks,
        retirements: &retirements,
    })
    .map_err(|error| SharePushError::Failed(error.to_string()))?;
    let mut next = latest.clone();
    next.floor = decision.floor.clone();
    if let FloorBaseSelection::Selected { base_id, covers } = decision.floor_base {
        next.floor_base = FloorBase {
            owner_prefix: prefix.to_string(),
            base_id,
            covers_seq: covers.seq,
            covers_anchor: covers.anchor,
        };
    }
    next.bases.retain(|base| {
        !decision.delete_bases.iter().any(|key| {
        matches!(parse_key(key), Ok(ObjectKey::Base { base_id, .. }) if base_id == base.base_id)
    })
    });
    for precondition in &decision.retirement_preconditions {
        let retired = precondition.prefix.to_string();
        if !next.retired.contains(&retired) {
            next.retired.push(retired);
        }
    }
    let head_changed = next.floor != latest.floor
        || next.floor_base != latest.floor_base
        || next.bases != latest.bases
        || next.retired != latest.retired;
    if head_changed {
        next.counter = latest
            .counter
            .checked_add(1)
            .ok_or_else(|| SharePushError::Failed("share head counter exhausted".into()))?;
        next.updated_ms = now_ms;
        write_head_retry(
            store,
            &prefix.to_string(),
            &latest,
            &next,
            &guard,
            capabilities,
        )
        .await?;
    }
    let final_head = if head_changed { next } else { latest.clone() };
    state.head_counter = final_head.counter;
    state.local_cursor = final_head.local_cursor;
    for key in decision
        .delete_segments
        .iter()
        .chain(decision.delete_chunks.iter())
        .chain(decision.delete_bases.iter())
    {
        let object_prefix =
            match parse_key(key).map_err(|error| SharePushError::Failed(error.to_string()))? {
                ObjectKey::Segment { prefix, .. }
                | ObjectKey::Base { prefix, .. }
                | ObjectKey::Chunk { prefix, .. } => prefix,
                _ => return Err(SharePushError::Failed("GC planned a non-object key".into())),
            };
        if object_prefix != prefix {
            return Err(SharePushError::Failed(
                "GC planned deletion outside own prefix".into(),
            ));
        }
        match store.delete(key).await {
            Ok(()) | Err(StoreError::NotFound) => (),
            Err(error) => return Err(push_store_error(error)),
        }
    }
    // A deletion that an earlier pass began goes on from what that pass found, even when the heads
    // it deleted no longer make the prefix due for retirement.
    let mut retirements = state
        .retirement_cleanups
        .iter()
        .map(|cleanup| (cleanup.precondition(), Some(cleanup.objects.clone())))
        .collect::<Vec<_>>();
    for precondition in decision.retirement_preconditions {
        if latest.retired.contains(&precondition.prefix.to_string())
            && !retirements
                .iter()
                .any(|(begun, _)| begun.prefix == precondition.prefix)
        {
            retirements.push((precondition, None));
        }
    }
    for (retirement, begun) in retirements {
        delete_retired_prefix(db, state, store, &retirement, begun.as_ref()).await?;
    }
    save_state(db, state)
        .await
        .map_err(|error| SharePushError::Failed(error.to_string()))?;
    Ok(final_head)
}

/// Deletes a retired prefix when everything listed in it is covered by the retirement. `begun`
/// holds the objects that an earlier pass found covered when it began this deletion.
///
/// The covered objects are saved in the state before the first delete, so a pass that stops half
/// way leaves what a later pass needs to finish: the manifests and heads that the first check
/// relies on can be gone by then. The record goes once the deletion is done or no longer applies.
async fn delete_retired_prefix(
    db: &DatabaseConnection,
    state: &mut ShareState,
    store: &dyn ObjectStore,
    retirement: &RetirementPrecondition,
    begun: Option<&BTreeSet<String>>,
) -> SharePushResult<()> {
    let listed = store
        .list_prefix(&format!("devices/{}/", retirement.prefix))
        .await
        .map_err(push_store_error)?;
    // The slots and manifests are read after the listing, so a head published before these
    // reads keeps everything that was listed.
    let covered = retired_listing_is_covered(store, &listed, retirement, begun).await?;
    if covered && begun.is_none() {
        state.retirement_cleanups.push(RetirementCleanup {
            prefix: retirement.prefix.clone(),
            head_counter: retirement.expected_counter,
            head: retirement.expected_head.clone(),
            objects: listed.iter().cloned().collect(),
        });
        save_state(db, state)
            .await
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
    }
    if covered {
        for key in &listed {
            let object_prefix =
                match parse_key(key).map_err(|error| SharePushError::Failed(error.to_string()))? {
                    ObjectKey::Head { prefix, .. }
                    | ObjectKey::Segment { prefix, .. }
                    | ObjectKey::Base { prefix, .. }
                    | ObjectKey::Chunk { prefix, .. } => prefix,
                    _ => continue,
                };
            if object_prefix != retirement.prefix {
                return Err(SharePushError::Failed(
                    "retirement listing returned an object outside its prefix".into(),
                ));
            }
            match store.delete(key).await {
                Ok(()) | Err(StoreError::NotFound) => (),
                Err(error) => return Err(push_store_error(error)),
            }
        }
    }
    state
        .retirement_cleanups
        .retain(|cleanup| cleanup.prefix != retirement.prefix);
    Ok(())
}

/// Whether everything in a listing of a retired prefix is covered by the retirement, so that
/// deleting it loses nothing the keeper base does not hold.
///
/// The head that the retirement covers must still be the newest head of the prefix, as
/// `retired_heads` checks. A device uploads a publication's segments, and a base cut's chunks and
/// manifest, before the head that names them, so any other object can belong to a head that is
/// still to come: a segment must end at or before the covered head, a base must be named by a
/// head in the slots, and a chunk must be referenced by the manifest of such a base. A manifest
/// that the covered head names must be there and verify. One that only an older head names can be
/// gone, because a device deletes a base it has dropped once the head without it is written.
///
/// When this device has begun the deletion already, `begun` holds what it found covered then,
/// and some of those objects, the covered head and its manifests among them, can be gone. Every
/// head left must still be the covered head or older, and every listed object must be one that
/// was found covered then; anything that came since keeps the prefix.
async fn retired_listing_is_covered(
    store: &dyn ObjectStore,
    listed: &[String],
    retirement: &RetirementPrecondition,
    begun: Option<&BTreeSet<String>>,
) -> SharePushResult<bool> {
    let Some(heads) = retired_heads(store, retirement, begun.is_none()).await? else {
        return Ok(false);
    };
    if let Some(begun) = begun {
        return Ok(listed.iter().all(|key| begun.contains(key)));
    }
    let prefix = retirement.prefix.to_string();
    // Each base that a head in the slots names, and whether the covered head is among them.
    let mut bases = BTreeMap::<String, bool>::new();
    for head in &heads {
        let covered = head.counter == retirement.expected_counter;
        for base in &head.bases {
            *bases.entry(base.base_id.clone()).or_default() |= covered;
        }
    }
    let mut chunks = BTreeSet::new();
    for (base_id, named_by_covered_head) in &bases {
        let key = base_key(&prefix, base_id)
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
        let manifest = match store.get(&key).await {
            Ok(value) => decode_manifest::<RuneModel>(&value.bytes, base_id).ok(),
            Err(StoreError::NotFound) => None,
            Err(error) => return Err(push_store_error(error)),
        };
        match manifest {
            Some(manifest) => chunks.extend(manifest_chunk_names(&manifest)),
            None if *named_by_covered_head => return Ok(false),
            None => (),
        }
    }
    for key in listed {
        let covered =
            match parse_key(key).map_err(|error| SharePushError::Failed(error.to_string()))? {
                ObjectKey::Segment { last, .. } => last <= retirement.expected_head.seq,
                ObjectKey::Base { base_id, .. } => bases.contains_key(&base_id),
                ObjectKey::Chunk { digest, .. } => chunks.contains(&digest),
                // The slots were checked above, and nothing else in the listing is deleted.
                _ => true,
            };
        if !covered {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The heads in the slots of a retired prefix, if the head that the retirement covers is still
/// the newest head there.
///
/// Like `refresh_own_head`, this reads both slots and goes by the newest head: each slot must be
/// empty, hold the covered head or hold an older head of the same stream, and with `covered_head`
/// one of them must hold the covered head. Unlike there, a slot that fails its checksum is not
/// skipped, because it can be a newer head that the retired device is still writing.
async fn retired_heads(
    store: &dyn ObjectStore,
    retirement: &RetirementPrecondition,
    covered_head: bool,
) -> SharePushResult<Option<Vec<Head>>> {
    let prefix = retirement.prefix.to_string();
    let mut heads = Vec::new();
    let mut covered = false;
    for slot in 0..=1 {
        let key =
            head_key(&prefix, slot).map_err(|error| SharePushError::Failed(error.to_string()))?;
        let remote = match store.get(&key).await {
            Ok(value) => match decode_head(&value.bytes) {
                Ok(head) => head,
                Err(FormatError::Checksum) => return Ok(None),
                Err(error) => return Err(SharePushError::Failed(error.to_string())),
            },
            Err(StoreError::NotFound) => continue,
            Err(error) => return Err(push_store_error(error)),
        };
        if remote.stream != retirement.stream_id || remote.generation != retirement.generation {
            return Ok(None);
        }
        match remote.counter.cmp(&retirement.expected_counter) {
            Ordering::Less => (),
            Ordering::Equal if remote.head == retirement.expected_head => covered = true,
            Ordering::Equal | Ordering::Greater => return Ok(None),
        }
        heads.push(remote);
    }
    Ok((covered || !covered_head).then_some(heads))
}

/// The chunks that a base manifest references: its buckets' chunks and its context fragments.
fn manifest_chunk_names<M: SyncModel>(
    manifest: &BaseManifest<M>,
) -> impl Iterator<Item = String> + '_ {
    manifest.collections.values().flat_map(|collection| {
        collection
            .buckets
            .iter()
            .map(|bucket| bucket.chunk.clone())
            .chain(collection.context_fragments.iter().cloned())
    })
}

/// Whether a base manifest references any candidate chunks.
fn manifest_has_candidates<M: SyncModel>(manifest: &BaseManifest<M>) -> bool {
    manifest
        .collections
        .values()
        .any(|collection| !collection.buckets.is_empty())
}

fn observed_head_last_change(state: &ShareState, prefix: &DevicePrefix) -> Option<u64> {
    [0, 1]
        .into_iter()
        .filter_map(|slot| {
            state
                .observed_objects
                .get(&ObjectKey::Head {
                    prefix: prefix.clone(),
                    slot,
                })
                .map(|observed| Ord::max(observed.last_change_ms, 0) as u64)
        })
        .max()
}

fn covers_at_least(a: &BTreeMap<String, Checkpoint>, b: &BTreeMap<String, Checkpoint>) -> bool {
    b.iter().all(|(stream, checkpoint)| {
        a.get(stream).is_some_and(|candidate| {
            candidate.seq > checkpoint.seq
                || (candidate.seq == checkpoint.seq && candidate.anchor == checkpoint.anchor)
        })
    })
}

/// Ensure the caller's library binding against library.json, creating it only on first contact.
pub async fn validate_share_library<M: SyncModel>(
    db: &DatabaseConnection,
    store: &dyn ObjectStore,
    library_id: Uuid,
    capabilities: ShareCapabilities,
) -> SharePullResult<LibraryFile> {
    validate_share_library_inner::<M>(db, store, library_id, capabilities, true).await
}

async fn validate_share_library_inner<M: SyncModel>(
    db: &DatabaseConnection,
    store: &dyn ObjectStore,
    library_id: Uuid,
    capabilities: ShareCapabilities,
    record_binding: bool,
) -> SharePullResult<LibraryFile> {
    ensure_tables(db).await.map_err(internal_error)?;
    let bound = binding_exists(db, library_id)
        .await
        .map_err(internal_error)?;
    let key = library_key();
    let value = match store.get(&key).await {
        Ok(value) => value,
        Err(StoreError::NotFound) if bound => return Err(SharePullError::ShareChanged),
        Err(StoreError::NotFound) => {
            let library = LibraryFile {
                format: FORMAT_VERSION,
                library_id,
                model: M::model_id().to_owned(),
                schema: M::schema_id().to_owned(),
            };
            let bytes = serde_json::to_vec(&library).map_err(internal_error)?;
            if capabilities.conditional_create {
                match store.create(&key, &bytes).await {
                    Ok(_) | Err(StoreError::PreconditionFailed) => (),
                    Err(error) => return Err(store_error(error)),
                }
            } else {
                // Without create-if-absent, a library that writes library.json at the same
                // moment cannot be told apart: the last write owns the folder, and the other
                // device meets ShareChanged on its next validation.
                record_unconfirmed_creation(db, library_id)
                    .await
                    .map_err(internal_error)?;
                match store.put(&key, &bytes).await {
                    Ok(_) => (),
                    Err(error) => return Err(store_error(error)),
                }
            }
            store.get(&key).await.map_err(store_error)?
        }
        Err(error) => return Err(store_error(error)),
    };
    let library =
        decode_library(&value.bytes).map_err(|error| SharePullError::Invalid(error.to_string()))?;
    if library.library_id != library_id
        || library.model != M::model_id()
        || library.schema != M::schema_id()
    {
        return Err(SharePullError::ShareChanged);
    }
    if record_binding {
        record_library_binding(db, library_id).await?;
    }
    confirm_creation_after_publication(db, library_id)
        .await
        .map_err(internal_error)?;
    Ok(library)
}

async fn record_unconfirmed_creation(db: &DatabaseConnection, library_id: Uuid) -> Result<()> {
    let statement = Query::insert()
        .into_table(Alias::new(UNCONFIRMED_CREATIONS_TABLE))
        .columns([Alias::new("library_id")])
        .values_panic([library_id.to_string().into()])
        .on_conflict(
            OnConflict::column(Alias::new("library_id"))
                .do_nothing()
                .to_owned(),
        )
        .to_owned();
    db.execute(&statement).await?;
    Ok(())
}

async fn creation_unconfirmed(db: &DatabaseConnection, library_id: Uuid) -> Result<bool> {
    let query = Query::select()
        .column(Alias::new("library_id"))
        .from(Alias::new(UNCONFIRMED_CREATIONS_TABLE))
        .and_where(Expr::col(Alias::new("library_id")).eq(library_id.to_string()))
        .to_owned();
    Ok(db.query_one(&query).await?.is_some())
}

async fn forget_unconfirmed_creation(db: &DatabaseConnection, library_id: Uuid) -> Result<()> {
    let statement = Query::delete()
        .from_table(Alias::new(UNCONFIRMED_CREATIONS_TABLE))
        .and_where(Expr::col(Alias::new("library_id")).eq(library_id.to_string()))
        .to_owned();
    db.execute(&statement).await?;
    Ok(())
}

// A creation holds once library.json still names this library on a validation after this device
// published its first head, which is the next pass after that publication. Only until then does
// this device remove its own prefix when another library wrote library.json last.
async fn confirm_creation_after_publication(
    db: &DatabaseConnection,
    library_id: Uuid,
) -> Result<()> {
    if !creation_unconfirmed(db, library_id).await? {
        return Ok(());
    }
    if load_state(db, library_id)
        .await?
        .is_some_and(|state| state.head_counter > 0)
    {
        forget_unconfirmed_creation(db, library_id).await?;
    }
    Ok(())
}

async fn load_state(db: &DatabaseConnection, library_id: Uuid) -> Result<Option<ShareState>> {
    let query = Query::select()
        .column(Alias::new("state_json"))
        .from(Alias::new(STATE_TABLE))
        .and_where(Expr::col(Alias::new("library_id")).eq(library_id.to_string()))
        .to_owned();
    let Some(row) = db.query_one(&query).await? else {
        return Ok(None);
    };
    let encoded: String = row.try_get_by_index(0)?;
    let state: ShareState = serde_json::from_str(&encoded).context("decode local share state")?;
    Ok(Some(state))
}

/// What became of this device's own prefix after a share validation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LostCreation {
    /// This device wrote library.json, another library wrote it later, and everything this
    /// device had written to the share is gone.
    PrefixRemoved,
    /// Nothing was removed: this device did not lose a creation race, or what is in its prefix
    /// was not written by this device.
    Kept,
}

/// After the share named another library, removes this device's own prefix when this device
/// created library.json and the creation had not yet held: another library wrote the file last
/// and owns the folder. Only keys of this device's current prefix are deleted, only when every
/// head there was written by this device's writer session, and nothing at all when library.json
/// is missing or names this library.
pub async fn discard_lost_creation(
    db: &DatabaseConnection,
    store: &dyn ObjectStore,
    writer: &ShareWriter,
    library_id: Uuid,
) -> Result<LostCreation> {
    ensure_tables(db).await?;
    if !creation_unconfirmed(db, library_id).await? {
        return Ok(LostCreation::Kept);
    }
    let owner = match store.get(&library_key()).await {
        Ok(value) => decode_library(&value.bytes)?,
        Err(StoreError::NotFound) => return Ok(LostCreation::Kept),
        Err(error) => anyhow::bail!("read library.json: {error:?}"),
    };
    if owner.library_id == library_id {
        return Ok(LostCreation::Kept);
    }
    let _guard = writer.lock().await;
    let Some(state) = load_state(db, library_id).await? else {
        forget_unconfirmed_creation(db, library_id).await?;
        return Ok(LostCreation::Kept);
    };
    let prefix = DevicePrefix {
        stream: state.stream,
        generation: state.generation,
    };
    let listed = store
        .list_prefix(&format!("devices/{prefix}/"))
        .await
        .map_err(|error| anyhow::anyhow!("list own prefix: {error:?}"))?;
    let mut heads = Vec::new();
    let mut objects = Vec::new();
    for key in listed {
        let object_prefix = match parse_key(&key)? {
            ObjectKey::Head { prefix, .. } => {
                heads.push(key.clone());
                prefix
            }
            ObjectKey::Segment { prefix, .. }
            | ObjectKey::Base { prefix, .. }
            | ObjectKey::Chunk { prefix, .. } => {
                objects.push(key.clone());
                prefix
            }
            ObjectKey::Probe => continue,
            _ => anyhow::bail!("own-prefix listing returned {key}"),
        };
        ensure!(
            object_prefix == prefix,
            "own-prefix listing returned a foreign object {key}"
        );
    }
    for key in &heads {
        let head = match store.get(key).await {
            Ok(value) => decode_head(&value.bytes)?,
            Err(StoreError::NotFound) => continue,
            Err(error) => anyhow::bail!("read own head {key}: {error:?}"),
        };
        if head.writer_session != state.writer_session || head.library_id != library_id {
            return Ok(LostCreation::Kept);
        }
    }
    // Heads go first, so no head is left naming objects that are already gone.
    for key in heads.iter().chain(&objects) {
        match store.delete(key).await {
            Ok(()) | Err(StoreError::NotFound) => (),
            Err(error) => anyhow::bail!("delete own object {key}: {error:?}"),
        }
    }
    forget_unconfirmed_creation(db, library_id).await?;
    Ok(LostCreation::PrefixRemoved)
}

async fn record_library_binding(db: &DatabaseConnection, library_id: Uuid) -> SharePullResult<()> {
    let statement = Query::insert()
        .into_table(Alias::new(LIBRARY_BINDINGS_TABLE))
        .columns([Alias::new("library_id")])
        .values_panic([library_id.to_string().into()])
        .on_conflict(
            OnConflict::column(Alias::new("library_id"))
                .do_nothing()
                .to_owned(),
        )
        .to_owned();
    db.execute(&statement).await.map_err(internal_error)?;
    Ok(())
}

/// Pull each stream through the normal Coordinator, reading journal segments on demand.
pub async fn pull<M: SyncModel>(
    request: SharePullRequest<'_, M>,
) -> SharePullResult<Vec<ReceiveProgress>> {
    let SharePullRequest {
        db,
        local,
        sink,
        local_source,
        store,
        library_id,
        scope,
        limits,
        cache_path,
        capabilities,
    } = request;
    validate_share_library_inner::<M>(db, &*store, library_id, capabilities, false).await?;
    let local_identity = local
        .identity()
        .await
        .map_err(|error| SharePullError::Store(error.to_string()))?;
    let scope_id =
        sync::scope_id(&scope).map_err(|error| SharePullError::Invalid(error.to_string()))?;
    let listed = store.list_prefix("devices/").await.map_err(store_error)?;
    let mut head_keys = BTreeMap::<DevicePrefix, BTreeSet<u8>>::new();
    for key in listed {
        match parse_key(&key) {
            Ok(ObjectKey::Head { prefix, slot }) => {
                head_keys.entry(prefix).or_default().insert(slot);
            }
            Ok(_) => (),
            Err(error) => return Err(SharePullError::Invalid(error.to_string())),
        }
    }

    let mut newest = BTreeMap::<Uuid, (DevicePrefix, Head)>::new();
    let mut observed_tags = Vec::new();
    for (prefix, slots) in head_keys {
        let mut valid = Vec::new();
        let mut checksum_failure = false;
        for slot in slots {
            let key = head_key(&prefix.to_string(), slot)
                .map_err(|error| SharePullError::Invalid(error.to_string()))?;
            match store.get(&key).await {
                Ok(value) => match decode_head(&value.bytes) {
                    Ok(head) => {
                        if head.stream != prefix.stream || head.generation != prefix.generation {
                            return Err(SharePullError::Invalid(
                                "head identity does not match its object key".into(),
                            ));
                        }
                        validate_head_covers(&head)?;
                        observed_tags.push((
                            ObjectKey::Head {
                                prefix: prefix.clone(),
                                slot,
                            },
                            value.tag,
                        ));
                        valid.push(head);
                    }
                    Err(FormatError::Checksum) => checksum_failure = true,
                    Err(error) => return Err(SharePullError::Invalid(error.to_string())),
                },
                Err(StoreError::NotFound) => checksum_failure = true,
                Err(error) => return Err(store_error(error)),
            }
        }
        let head = valid
            .into_iter()
            .max_by_key(|head| head.counter)
            .ok_or_else(|| {
                SharePullError::Invalid(if checksum_failure {
                    "both listed share head slots are invalid".into()
                } else {
                    "listed share head is missing".into()
                })
            })?;
        if head.library_id != library_id {
            continue;
        }
        if newest
            .get(&head.stream)
            .is_none_or(|(_, current)| head.updated_ms > current.updated_ms)
        {
            newest.insert(head.stream, (prefix, head));
        }
    }

    let mut peers = newest.into_values().collect::<Vec<_>>();
    peers.sort_by_key(|(_, head)| cmp::Reverse(head.head.seq));
    let candidates = base_candidates(&peers)?;
    record_library_binding(db, library_id).await?;
    let mut local_state = load_or_create_state(
        db,
        local_identity.replica,
        library_id,
        cache_path.clone(),
        Utc::now().timestamp_millis(),
    )
    .await
    .map_err(internal_error)?;
    let own_prefix = DevicePrefix {
        stream: local_state.stream,
        generation: local_state.generation,
    };
    if peers
        .iter()
        .any(|(_, head)| head.retired.contains(&own_prefix.to_string()))
    {
        local_state.rotate_generation();
    }
    let now_ms = Utc::now().timestamp_millis();
    for (key, tag) in observed_tags {
        local_state.observe_tag(key, tag, now_ms);
    }

    let mut progress = Vec::new();
    for (prefix, head) in peers {
        let checkpoint = CheckpointKey {
            namespace: local_identity.namespace,
            source_replica: head.stream,
            source_generation: head.generation,
            scope_id,
        };
        if head.stream == local_state.stream {
            if head.generation == local_state.generation {
                if head.writer_session != local_state.writer_session {
                    local_state.rotate_generation();
                }
            } else if head.writer_session != local_state.writer_session {
                local_state.rotate_generation();
            }
            continue;
        }
        // A base that cannot be read is skipped for this stream in this pull only: an object can
        // be missing just while the share recovers, so the next pull tries the base again. Each
        // attempt rereads the checkpoint, which a failed attempt may have advanced.
        let mut unavailable_bases = BTreeSet::new();
        let (done, selected_base) = 'selection: loop {
            let applied = local
                .checkpoint(&checkpoint)
                .await
                .map_err(|error| SharePullError::Store(error.to_string()))?;
            // With no checkpoint a new reader can still start at share genesis
            // if the whole history is retained. Older empty bases recorded the
            // local database's genesis instead, so must not be required here.
            let needs_base = applied.map_or(0, |received| received.sequence) < head.floor.seq;
            let selected_base = if needs_base {
                Some(
                    choose_live_base::<M>(&*store, &candidates, &prefix, &head, &unavailable_bases)
                        .await?,
                )
            } else {
                None
            };
            let peer = match selected_base.clone() {
                Some(base) => SharePeer::<M>::new_with_base(
                    store.clone(),
                    library_id,
                    prefix.to_string(),
                    cache_path.clone(),
                    base,
                ),
                None => SharePeer::<M>::new(
                    store.clone(),
                    library_id,
                    prefix.to_string(),
                    cache_path.clone(),
                ),
            }
            .map_err(|error| SharePullError::Invalid(error.to_string()))?;
            if let Some(genesis) = peer.genesis_checkpoint().await.map_err(sync_error)? {
                let tx = db.begin().await.map_err(internal_error)?;
                adopt_checkpoint_on(&tx, &checkpoint, &genesis)
                    .await
                    .map_err(internal_error)?;
                tx.commit().await.map_err(internal_error)?;
            }
            let mut coordinator = Coordinator::<M>::new(scope.clone(), limits)
                .map_err(|error| SharePullError::Invalid(error.to_string()))?;
            for _ in 0..100_000 {
                match coordinator
                    .advance(local, sink, local_source, &peer, &peer)
                    .await
                {
                    Ok(SessionAdvance::Complete(done)) => break 'selection (done, selected_base),
                    Ok(SessionAdvance::BudgetExhausted) => coordinator.start_pass(),
                    Ok(SessionAdvance::Incomplete) => (),
                    Ok(SessionAdvance::Cancelled) => {
                        return Err(SharePullError::Cancelled);
                    }
                    Err(error) => {
                        if matches!(&error, SyncError::SnapshotSourceUnavailable)
                            && let Some(base) = &selected_base
                        {
                            // An attempt that began reconciling holds a receiver session, and
                            // the next base needs room for its own.
                            abort_coordinator(&mut coordinator, local, sink, local_source, &peer)
                                .await?;
                            unavailable_bases
                                .insert((base.owner_prefix.clone(), base.base_id.clone()));
                            continue 'selection;
                        }
                        return Err(sync_error(error));
                    }
                }
            }
            return Err(SharePullError::Store(
                "share coordinator exceeded its work bound".into(),
            ));
        };
        progress.push(done);
        if let Some(base) = selected_base {
            adopt_manifest_covers(db, &local_identity, &base.manifest, &scope).await?;
        }
    }
    save_state(db, &local_state).await.map_err(internal_error)?;
    Ok(progress)
}

/// Drives a coordinator that the pull gives up on through its abort path. That cancels its
/// receiver session and releases both snapshot requests; what it already committed stays.
async fn abort_coordinator<M: SyncModel>(
    coordinator: &mut Coordinator<M>,
    local: &dyn sync::ReceiveStore<M>,
    sink: &dyn sync::SnapshotSink<M>,
    local_source: &dyn sync::SnapshotSource<M>,
    peer: &SharePeer<M>,
) -> SharePullResult<()> {
    coordinator.start_abort();
    for _ in 0..100_000 {
        match coordinator
            .advance(local, sink, local_source, peer, peer)
            .await
            .map_err(sync_error)?
        {
            SessionAdvance::Cancelled => return Ok(()),
            SessionAdvance::BudgetExhausted => coordinator.start_pass(),
            SessionAdvance::Incomplete | SessionAdvance::Complete(_) => (),
        }
    }
    Err(SharePullError::Store(
        "share coordinator exceeded its work bound".into(),
    ))
}

/// Publish a prepared local journal range after the caller has completed pull.
/// The required per-library writer covers prefix writes through head commit and cursor update.
pub async fn publish_journal<M: SyncModel>(
    request: PublishJournalRequest<'_, M>,
) -> SharePushResult<Option<Head>> {
    let PublishJournalRequest {
        db,
        state,
        writer,
        store,
        identity,
        current_head,
        entries,
        acknowledgements,
        share_stream_ids,
        share_local_replicas,
        newest_base,
        cursor_status,
        capabilities,
        max_events_per_segment,
    } = request;
    let expected_counter = current_head.counter;
    let expected_writer_session = current_head.writer_session;
    let writer_guard = writer.lock().await;
    validate_share_library_inner::<M>(db, store, state.library_id, capabilities, true)
        .await
        .map_err(|error| match error {
            SharePullError::ShareChanged => SharePushError::ShareChanged,
            SharePullError::ReadOnly => SharePushError::ReadOnly,
            other => SharePushError::Failed(other.to_string()),
        })?;
    let own_prefix = DevicePrefix {
        stream: state.stream,
        generation: state.generation,
    };
    let current_head = refresh_own_head(store, &own_prefix).await?;
    if current_head.counter < expected_counter
        || current_head.writer_session != expected_writer_session
    {
        return Err(SharePushError::StaleHead);
    }
    state.head_counter = current_head.counter;
    state.local_cursor = current_head.local_cursor;
    let share_identity = StoreIdentity {
        namespace: identity.namespace,
        model: identity.model.clone(),
        replica: own_prefix.stream,
        generation: own_prefix.generation,
    };
    let decision = plan_publish(PublishInput {
        identity: &share_identity,
        after_cursor: state.local_cursor,
        entries,
        share_stream_ids,
        share_local_replicas,
        head: Checkpoint {
            seq: current_head.head.seq,
            anchor: current_head.head.anchor,
        },
        existing_segments: &current_head.segments,
        newest_base,
        export_cursor: cursor_status,
        max_events_per_segment,
    })
    .map_err(|error| SharePushError::Failed(error.to_string()))?;
    if let PublishDecision::NothingToPublish { export_cursor } = &decision {
        let mut ack_head = current_head.clone();
        for (prefix, sequence) in acknowledgements {
            let text = prefix.to_string();
            ack_head
                .acks
                .entry(text)
                .and_modify(|current| *current = Ord::max(*current, *sequence))
                .or_insert(*sequence);
        }
        let cursor_advanced = *export_cursor > current_head.local_cursor;
        if ack_head.acks == current_head.acks && !cursor_advanced {
            return Ok(None);
        }
        ack_head.local_cursor = Ord::max(current_head.local_cursor, *export_cursor);
        ack_head.counter = current_head
            .counter
            .checked_add(1)
            .ok_or_else(|| SharePushError::Failed("share head counter exhausted".into()))?;
        ack_head.updated_ms = Ord::max(Utc::now().timestamp_millis(), 0) as u64;
        let written = write_head_retry(
            store,
            &own_prefix.to_string(),
            &current_head,
            &ack_head,
            &writer_guard,
            capabilities,
        )
        .await?;
        advance_export_cursor(
            db,
            identity.namespace,
            state.stream.as_bytes(),
            state.local_cursor,
            written.local_cursor,
        )
        .await?;
        state.head_counter = written.counter;
        state.local_cursor = written.local_cursor;
        save_state(db, state)
            .await
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
        return Ok(Some(written));
    }
    let (events, segments, rollups, export_cursor) = match decision {
        PublishDecision::NothingToPublish { .. } => return Ok(None),
        PublishDecision::CutBase { .. } => return Err(SharePushError::NeedsBase),
        PublishDecision::PublishSegments {
            events,
            segments,
            rollups,
            export_cursor,
            ..
        } => (events, segments, rollups, export_cursor),
    };
    let prefix = own_prefix.to_string();
    let mut next_head = current_head.clone();
    for (device, sequence) in acknowledgements {
        let text = device.to_string();
        next_head
            .acks
            .entry(text)
            .and_modify(|current| *current = Ord::max(*current, *sequence))
            .or_insert(*sequence);
    }
    let mut written_segments = BTreeMap::new();
    for group in segments {
        let numbered = events
            .iter()
            .filter(|event| (group.first..=group.last).contains(&event.share_sequence))
            .collect::<Vec<_>>();
        let links = numbered
            .iter()
            .map(|event| {
                let collection = match &event.event {
                    Event::Key(key) => key.key.collection.clone(),
                    Event::Merge(merge) => merge.collection.clone(),
                };
                SegmentLink {
                    seq: event.share_sequence,
                    collection,
                    event: event.event.clone(),
                }
            })
            .collect::<Vec<_>>();
        let segment = Segment {
            format: FORMAT_VERSION,
            prev_anchor: group.previous_anchor,
            links,
            last_anchor: group.last_anchor,
        };
        let bytes =
            encode_segment(&segment).map_err(|error| SharePushError::Failed(error.to_string()))?;
        let hash = blake3::hash(&bytes).to_hex().to_string();
        let hash16 = &hash[..16];
        let key = segment_key(&prefix, group.first, group.last, hash16)
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
        write_immutable_object(
            store,
            state,
            &key,
            &bytes,
            capabilities,
            "immutable segment name contains different bytes",
            Some(OwnContents::Segment {
                prev_anchor: group.previous_anchor,
                last_anchor: group.last_anchor,
                bytes: bytes.len() as u64,
            }),
        )
        .await?;
        let reference = SegmentRef {
            level: 0,
            first: group.first,
            last: group.last,
            prev_anchor: group.previous_anchor,
            last_anchor: group.last_anchor,
            name: hash16.to_owned(),
            bytes: bytes.len() as u64,
        };
        next_head.segments.push(reference.clone());
        written_segments.insert(
            (reference.level, reference.first, reference.last),
            (segment, reference),
        );
    }
    for rollup in rollups {
        let mut links = Vec::new();
        for input in &rollup.inputs {
            let source = match input {
                RollupInput::Existing(reference) => {
                    let key =
                        segment_key(&prefix, reference.first, reference.last, &reference.name)
                            .map_err(|error| SharePushError::Failed(error.to_string()))?;
                    let object = store.get(&key).await.map_err(push_store_error)?;
                    let digest = blake3::hash(&object.bytes).to_hex().to_string();
                    if object.bytes.len() as u64 != reference.bytes
                        || !digest.starts_with(&reference.name)
                    {
                        return Err(SharePushError::Failed(
                            "existing segment digest mismatch".into(),
                        ));
                    }
                    decode_segment::<M>(&object.bytes)
                        .map_err(|error| SharePushError::Failed(error.to_string()))?
                }
                RollupInput::Planned {
                    level, first, last, ..
                } => written_segments
                    .get(&(*level, *first, *last))
                    .map(|(segment, _)| segment.clone())
                    .ok_or_else(|| {
                        SharePushError::Failed("planned roll-up input is missing".into())
                    })?,
            };
            links.extend(source.links);
        }
        let rolled = Segment {
            format: FORMAT_VERSION,
            prev_anchor: rollup.previous_anchor,
            links,
            last_anchor: rollup.last_anchor,
        };
        let bytes =
            encode_segment(&rolled).map_err(|error| SharePushError::Failed(error.to_string()))?;
        let hash = blake3::hash(&bytes).to_hex().to_string();
        let hash16 = &hash[..16];
        let key = segment_key(&prefix, rollup.first, rollup.last, hash16)
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
        write_immutable_object(
            store,
            state,
            &key,
            &bytes,
            capabilities,
            "immutable roll-up name collision",
            Some(OwnContents::Segment {
                prev_anchor: rollup.previous_anchor,
                last_anchor: rollup.last_anchor,
                bytes: bytes.len() as u64,
            }),
        )
        .await?;
        next_head.segments.retain(|reference| {
            !(reference.level < rollup.level
                && reference.first >= rollup.first
                && reference.last <= rollup.last)
        });
        let reference = SegmentRef {
            level: rollup.level,
            first: rollup.first,
            last: rollup.last,
            prev_anchor: rollup.previous_anchor,
            last_anchor: rollup.last_anchor,
            name: hash16.to_owned(),
            bytes: bytes.len() as u64,
        };
        next_head.segments.push(reference.clone());
        written_segments.insert(
            (reference.level, reference.first, reference.last),
            (rolled, reference),
        );
    }
    next_head.counter = current_head
        .counter
        .checked_add(1)
        .ok_or_else(|| SharePushError::Failed("share head counter exhausted".into()))?;
    next_head.head = Checkpoint {
        seq: events
            .last()
            .map_or(current_head.head.seq, |event| event.share_sequence),
        anchor: events
            .last()
            .map_or(current_head.head.anchor, |event| event.anchor),
    };
    next_head.local_cursor = export_cursor;
    next_head.updated_ms = Ord::max(Utc::now().timestamp_millis(), 0) as u64;

    let written_head = write_head_retry(
        store,
        &prefix,
        &current_head,
        &next_head,
        &writer_guard,
        capabilities,
    )
    .await?;
    advance_export_cursor(
        db,
        identity.namespace,
        state.stream.as_bytes(),
        state.local_cursor,
        export_cursor,
    )
    .await?;
    state.local_cursor = export_cursor;
    state.head_counter = written_head.counter;
    save_state(db, state)
        .await
        .map_err(|error| SharePushError::Failed(error.to_string()))?;
    Ok(Some(written_head))
}

async fn advance_export_cursor(
    db: &DatabaseConnection,
    namespace: Uuid,
    export_id: &[u8],
    current_cursor: u64,
    export_cursor: u64,
) -> SharePushResult<()> {
    let tx = db
        .begin()
        .await
        .map_err(|error| SharePushError::Failed(error.to_string()))?;
    if read_on(&tx, namespace, export_id)
        .await
        .map_err(|error| SharePushError::Failed(error.to_string()))?
        .is_none()
    {
        create_on(
            &tx,
            namespace,
            export_id,
            i64::try_from(current_cursor)
                .map_err(|error| SharePushError::Failed(error.to_string()))?,
            Ord::max(Utc::now().timestamp_millis(), 0),
        )
        .await
        .map_err(|error| SharePushError::Failed(error.to_string()))?;
    }
    advance_on(
        &tx,
        namespace,
        export_id,
        i64::try_from(export_cursor).map_err(|error| SharePushError::Failed(error.to_string()))?,
        Ord::max(Utc::now().timestamp_millis(), 0),
    )
    .await
    .map_err(|error| SharePushError::Failed(error.to_string()))?;
    tx.commit()
        .await
        .map_err(|error| SharePushError::Failed(error.to_string()))?;
    Ok(())
}

async fn write_head_retry(
    store: &dyn ObjectStore,
    prefix: &str,
    expected: &Head,
    next: &Head,
    _writer: &MutexGuard<'_, ()>,
    capabilities: ShareCapabilities,
) -> SharePushResult<Head> {
    let mut current_slot = None;
    for slot in 0..=1 {
        let key =
            head_key(prefix, slot).map_err(|error| SharePushError::Failed(error.to_string()))?;
        match store.get(&key).await {
            Ok(value) => match decode_head(&value.bytes) {
                Ok(head) if head == *expected => current_slot = Some(slot),
                Ok(_) | Err(FormatError::Checksum) => (),
                Err(error) => return Err(SharePushError::Failed(error.to_string())),
            },
            Err(StoreError::NotFound) => (),
            Err(error) => return Err(push_store_error(error)),
        }
    }
    let slot = current_slot
        .map(|slot| 1 - slot)
        .ok_or(SharePushError::StaleHead)?;
    let key = head_key(prefix, slot).map_err(|error| SharePushError::Failed(error.to_string()))?;
    let bytes = encode_head(next).map_err(|error| SharePushError::Failed(error.to_string()))?;
    for _ in 0..3 {
        let current = match store.get(&key).await {
            Ok(value) => Some(value),
            Err(StoreError::NotFound) => None,
            Err(error) => return Err(push_store_error(error)),
        };
        if let Some(value) = &current {
            match decode_head(&value.bytes) {
                Ok(head) if head == *next => return Ok(head),
                Ok(_) | Err(FormatError::Checksum) => (),
                Err(error) => return Err(SharePushError::Failed(error.to_string())),
            }
        }
        let mut observed = Vec::new();
        for candidate_slot in 0..=1 {
            let candidate_key = head_key(prefix, candidate_slot)
                .map_err(|error| SharePushError::Failed(error.to_string()))?;
            match store.get(&candidate_key).await {
                Ok(value) => match decode_head(&value.bytes) {
                    Ok(head) => observed.push(head),
                    Err(FormatError::Checksum) => (),
                    Err(error) => return Err(SharePushError::Failed(error.to_string())),
                },
                Err(StoreError::NotFound) => (),
                Err(error) => return Err(push_store_error(error)),
            }
        }
        let latest = observed.into_iter().max_by_key(|head| head.counter);
        if latest.as_ref().is_none_or(|head| {
            head.counter != expected.counter || head.writer_session != expected.writer_session
        }) {
            return Err(SharePushError::StaleHead);
        }
        let result = match current {
            Some(value) if capabilities.conditional_head_write => {
                store.put_if_match(&key, &value.tag, &bytes).await
            }
            None if capabilities.conditional_head_write => store.create(&key, &bytes).await,
            _ => store.put(&key, &bytes).await,
        };
        match result {
            Ok(_) => return Ok(next.clone()),
            Err(StoreError::PreconditionFailed) => continue,
            Err(error) => return Err(push_store_error(error)),
        }
    }
    Err(SharePushError::StaleHead)
}

async fn refresh_own_head(store: &dyn ObjectStore, prefix: &DevicePrefix) -> SharePushResult<Head> {
    let mut latest = None;
    for slot in 0..=1 {
        let key = head_key(&prefix.to_string(), slot)
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
        match store.get(&key).await {
            Ok(value) => match decode_head(&value.bytes) {
                Ok(head) => {
                    if head.stream != prefix.stream || head.generation != prefix.generation {
                        return Err(SharePushError::ShareChanged);
                    }
                    if latest
                        .as_ref()
                        .is_none_or(|current: &Head| head.counter > current.counter)
                    {
                        latest = Some(head);
                    }
                }
                Err(FormatError::Checksum) => (),
                Err(error) => return Err(SharePushError::Failed(error.to_string())),
            },
            Err(StoreError::NotFound) => (),
            Err(error) => return Err(push_store_error(error)),
        }
    }
    latest.ok_or(SharePushError::StaleHead)
}

pub(super) fn push_store_error(error: StoreError) -> SharePushError {
    match error {
        StoreError::PermissionDenied => SharePushError::ReadOnly,
        StoreError::Unauthorized => SharePushError::CredentialsRejected,
        StoreError::Transient(message) => SharePushError::Failed(format!("transient: {message}")),
        StoreError::NotFound => SharePushError::MissingObject,
        other => SharePushError::Failed(format!("{other:?}")),
    }
}

/// Writes an immutable object into this device's prefix. It records the tag the write returned,
/// or the tag of the identical object that was already there, and with that tag the `contents` of
/// a manifest or segment, so that garbage collection confirms the object instead of downloading
/// it. The tag of a successful write is taken for the tag of the bytes sent; they are not read
/// back.
async fn write_immutable_object(
    store: &dyn ObjectStore,
    state: &mut ShareState,
    key: &str,
    bytes: &[u8],
    capabilities: ShareCapabilities,
    collision_message: &'static str,
    contents: Option<OwnContents>,
) -> SharePushResult<()> {
    let tag = if capabilities.conditional_create {
        match store.create(key, bytes).await {
            Ok(tag) => tag,
            Err(StoreError::PreconditionFailed) => {
                let existing = store.get(key).await.map_err(push_store_error)?;
                if existing.bytes != bytes {
                    return Err(SharePushError::Failed(collision_message.into()));
                }
                existing.tag
            }
            Err(error) => return Err(push_store_error(error)),
        }
    } else {
        match store.get(key).await {
            Ok(existing) if existing.bytes == bytes => existing.tag,
            Ok(_) => return Err(SharePushError::Failed(collision_message.into())),
            Err(StoreError::NotFound) => store.put(key, bytes).await.map_err(push_store_error)?,
            Err(error) => return Err(push_store_error(error)),
        }
    };
    let object = parse_key(key).map_err(|error| SharePushError::Failed(error.to_string()))?;
    state.observe_tag(object.clone(), tag.clone(), Utc::now().timestamp_millis());
    if let Some(contents) = contents {
        state.record(object, tag, contents);
    }
    Ok(())
}

/// What observing one of this device's objects found.
enum OwnObjectRead {
    /// The store confirmed the tag recorded for the object, so nothing was downloaded.
    Unchanged,
    /// The object was downloaded: no tag was recorded for it, its tag changed, or the store has
    /// no conditional GET.
    Downloaded(ObjectValue),
    /// The object is no longer in the store.
    Missing,
}

/// Observes the tag of an object in this device's prefix. A tag recorded earlier, from this
/// device's own write or an earlier observation, is confirmed with a conditional GET where the
/// store supports one, so an object that has not changed is not downloaded again. A different tag
/// is recorded like any other observation, which restarts the object's retention timers.
async fn observe_own_object(
    store: &dyn ObjectStore,
    state: &mut ShareState,
    object: &ObjectKey,
    key: &str,
    capabilities: ShareCapabilities,
    now_ms: i64,
) -> SharePushResult<OwnObjectRead> {
    let recorded = state
        .observed_objects
        .get(object)
        .filter(|_| capabilities.conditional_get)
        .map(|observed| observed.tag.clone());
    let read = match &recorded {
        Some(tag) => store
            .get_if_none_match(key, tag)
            .await
            .map(|result| match result {
                ConditionalGet::NotModified => None,
                ConditionalGet::Modified(value) => Some(value),
            }),
        None => store.get(key).await.map(Some),
    };
    match read {
        Ok(None) => Ok(OwnObjectRead::Unchanged),
        Ok(Some(value)) => {
            state.observe_tag(object.clone(), value.tag.clone(), now_ms);
            Ok(OwnObjectRead::Downloaded(value))
        }
        Err(StoreError::NotFound) => Ok(OwnObjectRead::Missing),
        Err(error) => Err(push_store_error(error)),
    }
}

/// What this device knows of one of its base manifests or segments, for the bytes the store holds
/// now, or `None` when the object is gone. A record is taken only when the store has confirmed the
/// tag it belongs to; other bytes are downloaded, unless `observed` holds them already, decoded,
/// and recorded in its place. A manifest whose candidate flag is unknown, recorded by an earlier
/// build, is read and decoded too, so it always returns one that knows. `observed` is what
/// observing the object found in this pass; without it the object is observed here.
async fn own_contents<M: SyncModel>(
    store: &dyn ObjectStore,
    state: &mut ShareState,
    object: &ObjectKey,
    key: &str,
    observed: Option<OwnObjectRead>,
    capabilities: ShareCapabilities,
    now_ms: i64,
) -> SharePushResult<Option<OwnContents>> {
    let observed = match observed {
        Some(observed) => observed,
        None => observe_own_object(store, state, object, key, capabilities, now_ms).await?,
    };
    let value = match observed {
        OwnObjectRead::Missing => return Ok(None),
        OwnObjectRead::Downloaded(value) => value,
        OwnObjectRead::Unchanged => {
            if let Some(contents) = state.recorded(object) {
                let resolved = !matches!(
                    contents,
                    OwnContents::Manifest {
                        has_candidates: None,
                        ..
                    }
                );
                if resolved {
                    return Ok(Some(contents.clone()));
                }
            }
            // Confirmed, but nothing is recorded for these bytes, as after an earlier build, or
            // the recorded manifest does not yet say whether it holds candidates. Read the bytes
            // so the flag is resolved before anything decides with the contents.
            match store.get(key).await {
                Ok(value) => {
                    state.observe_tag(object.clone(), value.tag.clone(), now_ms);
                    value
                }
                Err(StoreError::NotFound) => return Ok(None),
                Err(error) => return Err(push_store_error(error)),
            }
        }
    };
    let contents = match OwnContents::decode::<M>(object, &value.bytes)? {
        OwnContents::Manifest {
            content_gzip_bytes: None,
            chunks,
            has_candidates,
            manifest_gzip_bytes,
        } => {
            // The manifest names its content by hash, so a recorded size for the same chunks
            // still describes these bytes after the manifest was read again, even when the store
            // reports another tag for the bytes it wrote.
            let content_gzip_bytes = match state.own_objects.get(object) {
                Some(OwnRecord {
                    contents:
                        OwnContents::Manifest {
                            content_gzip_bytes: Some(bytes),
                            chunks: recorded_chunks,
                            ..
                        },
                    ..
                }) if recorded_chunks == &chunks => Some(*bytes),
                _ => None,
            };
            OwnContents::Manifest {
                content_gzip_bytes,
                chunks,
                has_candidates,
                manifest_gzip_bytes,
            }
        }
        other => other,
    };
    state.record(object.clone(), value.tag, contents.clone());
    Ok(Some(contents))
}

/// The compressed sizes a base contributes to the share: its content, the chunks and context
/// fragments that its manifest references, and the manifest itself.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BaseGzipSizes {
    /// The content bytes, always the full chunk and context fragment total.
    pub content: u64,
    /// The manifest bytes.
    pub manifest: u64,
    /// Whether the manifest references any candidate chunks. A base without them holds no
    /// catalog content; the planners decide what that means for a change.
    pub has_candidates: bool,
}

/// The compressed sizes of a base and whether it holds candidate chunks: the chunks and context
/// fragments that its manifest references, and the manifest itself. A base cut records both sizes
/// with the manifest; a manifest from an earlier build resolves its candidate flag and measures
/// its content once, by reading its chunks, then records them. `None` means the manifest or one
/// of its chunks is gone.
pub(crate) async fn base_gzip_sizes<M: SyncModel>(
    store: &dyn ObjectStore,
    state: &mut ShareState,
    prefix: &DevicePrefix,
    base_id: &str,
    capabilities: ShareCapabilities,
    now_ms: i64,
) -> SharePushResult<Option<BaseGzipSizes>> {
    let object = ObjectKey::Base {
        prefix: prefix.clone(),
        base_id: base_id.to_owned(),
    };
    if let Some(OwnContents::Manifest {
        content_gzip_bytes: Some(content_gzip_bytes),
        has_candidates: Some(has_candidates),
        manifest_gzip_bytes,
        ..
    }) = state.recorded(&object)
    {
        return Ok(Some(BaseGzipSizes {
            content: *content_gzip_bytes,
            manifest: *manifest_gzip_bytes,
            has_candidates: *has_candidates,
        }));
    }
    let key = base_key(&prefix.to_string(), base_id)
        .map_err(|error| SharePushError::Failed(error.to_string()))?;
    let Some(OwnContents::Manifest {
        content_gzip_bytes,
        has_candidates,
        chunks,
        manifest_gzip_bytes,
    }) = own_contents::<M>(store, state, &object, &key, None, capabilities, now_ms).await?
    else {
        return Ok(None);
    };
    // `own_contents` reads a manifest whose flag is unknown, so a manifest it returns always
    // knows whether it holds candidates.
    let Some(has_candidates) = has_candidates else {
        return Err(SharePushError::Failed(
            "the base manifest left its candidate flag unresolved".into(),
        ));
    };
    if let Some(content_gzip_bytes) = content_gzip_bytes {
        return Ok(Some(BaseGzipSizes {
            content: content_gzip_bytes,
            manifest: manifest_gzip_bytes,
            has_candidates,
        }));
    }
    // A manifest recorded by an earlier build names no content size yet; count every chunk it
    // references, context fragments included.
    let mut measured = 0_u64;
    for chunk in &chunks {
        let key = chunk_key(&prefix.to_string(), chunk)
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
        match store.get(&key).await {
            Ok(value) => {
                measured = measured
                    .checked_add(value.bytes.len() as u64)
                    .ok_or_else(|| SharePushError::Failed("base content size overflow".into()))?;
            }
            Err(StoreError::NotFound) => return Ok(None),
            Err(error) => return Err(push_store_error(error)),
        }
    }
    record_base_gzip_sizes(state, &object, Some(measured), manifest_gzip_bytes);
    Ok(Some(BaseGzipSizes {
        content: measured,
        manifest: manifest_gzip_bytes,
        has_candidates,
    }))
}

/// Replaces the recorded content and manifest sizes of one base manifest, keeping the candidate
/// flag recorded with them.
fn record_base_gzip_sizes(
    state: &mut ShareState,
    object: &ObjectKey,
    content_gzip_bytes: Option<u64>,
    manifest_gzip_bytes: u64,
) {
    if let Some(OwnRecord {
        contents:
            OwnContents::Manifest {
                content_gzip_bytes: recorded_content,
                manifest_gzip_bytes: recorded_manifest,
                ..
            },
        ..
    }) = state.own_objects.get_mut(object)
    {
        *recorded_content = content_gzip_bytes;
        *recorded_manifest = manifest_gzip_bytes;
    }
}

struct BaseCandidate {
    owner_prefix: DevicePrefix,
    base_id: String,
    covers: BTreeMap<String, Checkpoint>,
}

fn validate_head_covers(head: &Head) -> SharePullResult<()> {
    DevicePrefix::from_str(&head.floor_base.owner_prefix)
        .map_err(|error| SharePullError::Invalid(error.to_string()))?;
    for base in &head.bases {
        for prefix in base.covers.keys() {
            DevicePrefix::from_str(prefix)
                .map_err(|error| SharePullError::Invalid(error.to_string()))?;
        }
    }
    Ok(())
}

fn base_candidates(peers: &[(DevicePrefix, Head)]) -> SharePullResult<Vec<BaseCandidate>> {
    let mut candidates = BTreeMap::<(DevicePrefix, String), BaseCandidate>::new();
    for (device, head) in peers {
        let owner = device.clone();
        for reference in &head.bases {
            let candidate = candidates
                .entry((owner.clone(), reference.base_id.clone()))
                .or_insert_with(|| BaseCandidate {
                    owner_prefix: owner.clone(),
                    base_id: reference.base_id.clone(),
                    covers: BTreeMap::new(),
                });
            for (prefix, cover) in &reference.covers {
                if candidate
                    .covers
                    .insert(prefix.clone(), cover.clone())
                    .is_some_and(|existing| existing != *cover)
                {
                    return Err(SharePullError::Invalid(
                        "conflicting base cover declarations".into(),
                    ));
                }
            }
        }
        DevicePrefix::from_str(&head.floor_base.owner_prefix)
            .map_err(|error| SharePullError::Invalid(error.to_string()))?;
        let floor_owner = DevicePrefix::from_str(&head.floor_base.owner_prefix)
            .map_err(|error| SharePullError::Invalid(error.to_string()))?;
        let candidate = candidates
            .entry((floor_owner.clone(), head.floor_base.base_id.clone()))
            .or_insert_with(|| BaseCandidate {
                owner_prefix: floor_owner,
                base_id: head.floor_base.base_id.clone(),
                covers: BTreeMap::new(),
            });
        let floor_cover = Checkpoint {
            seq: head.floor_base.covers_seq,
            anchor: head.floor_base.covers_anchor,
        };
        if candidate
            .covers
            .insert(device.to_string(), floor_cover.clone())
            .is_some_and(|existing| existing != floor_cover)
        {
            return Err(SharePullError::Invalid(
                "floor base cover conflicts with its base reference".into(),
            ));
        }
    }
    Ok(candidates.into_values().collect())
}

/// Selects the live base covering `target` with the largest total coverage. A base whose
/// manifest is missing, does not decode or fails its digest is skipped for this selection, as
/// are the `unavailable_bases` this pull could not read.
async fn choose_live_base<M: SyncModel>(
    store: &dyn ObjectStore,
    candidates: &[BaseCandidate],
    target: &DevicePrefix,
    head: &Head,
    unavailable_bases: &BTreeSet<(DevicePrefix, String)>,
) -> SharePullResult<SelectedBase<M>> {
    let target_prefix = target.to_string();
    let mut ordered = Vec::new();
    for candidate in candidates.iter().filter(|candidate| {
        candidate
            .covers
            .get(&target_prefix)
            .is_some_and(|cover| cover.seq > head.floor.seq || cover == &head.floor)
            && !unavailable_bases
                .contains(&(candidate.owner_prefix.clone(), candidate.base_id.clone()))
    }) {
        let coverage = candidate
            .covers
            .values()
            .try_fold(0_u64, |sum, cover| sum.checked_add(cover.seq))
            .ok_or_else(|| SharePullError::Invalid("base coverage sum overflows".into()))?;
        ordered.push((candidate, coverage));
    }
    ordered.sort_by_key(|(_, coverage)| cmp::Reverse(*coverage));
    for (candidate, _) in ordered {
        let key = base_key(&candidate.owner_prefix.to_string(), &candidate.base_id)
            .map_err(|error| SharePullError::Invalid(error.to_string()))?;
        let object = match store.get(&key).await {
            Ok(object) => object,
            Err(StoreError::NotFound) => continue,
            Err(error) => return Err(store_error(error)),
        };
        let Ok(manifest) = decode_manifest::<M>(&object.bytes, &candidate.base_id) else {
            continue;
        };
        validate_manifest_covers(&manifest)?;
        for (prefix, cover) in &candidate.covers {
            if manifest.covers.get(prefix) != Some(cover) {
                return Err(SharePullError::Invalid(
                    "base manifest disagrees with its head cover".into(),
                ));
            }
        }
        let cover = manifest
            .covers
            .get(&target_prefix)
            .ok_or_else(|| SharePullError::Invalid("selected base omits target cover".into()))?;
        if candidate.owner_prefix.to_string() == head.floor_base.owner_prefix
            && candidate.base_id == head.floor_base.base_id
            && (cover.seq != head.floor_base.covers_seq
                || cover.anchor != head.floor_base.covers_anchor)
        {
            return Err(SharePullError::Invalid(
                "floor base reference disagrees with its manifest".into(),
            ));
        }
        return Ok(SelectedBase {
            owner_prefix: candidate.owner_prefix.clone(),
            base_id: candidate.base_id.clone(),
            manifest,
        });
    }
    if unavailable_bases.is_empty() {
        Err(SharePullError::Invalid(
            "no live base covers the required checkpoint".into(),
        ))
    } else {
        Err(sync_error(SyncError::SnapshotSourceUnavailable))
    }
}

fn validate_manifest_covers<M: SyncModel>(manifest: &BaseManifest<M>) -> SharePullResult<()> {
    for prefix in manifest.covers.keys() {
        DevicePrefix::from_str(prefix)
            .map_err(|error| SharePullError::Invalid(error.to_string()))?;
    }
    Ok(())
}

fn sync_error(error: sync::SyncError) -> SharePullError {
    match error {
        SyncError::Transport(message) => SharePullError::Transient(message),
        SyncError::SnapshotSourceUnavailable => {
            SharePullError::MissingObject("referenced share base or chunk is missing".into())
        }
        other => SharePullError::Store(other.to_string()),
    }
}

async fn adopt_manifest_covers<M: SyncModel>(
    db: &DatabaseConnection,
    identity: &StoreIdentity,
    manifest: &BaseManifest<M>,
    scope: &Scope<M>,
) -> SharePullResult<()> {
    let scope_id =
        sync::scope_id(scope).map_err(|error| SharePullError::Invalid(error.to_string()))?;
    let tx = db.begin().await.map_err(internal_error)?;
    for (prefix, cover) in &manifest.covers {
        let device = DevicePrefix::from_str(prefix)
            .map_err(|error| SharePullError::Invalid(error.to_string()))?;
        let key = CheckpointKey {
            namespace: identity.namespace,
            source_replica: device.stream,
            source_generation: device.generation,
            scope_id,
        };
        let progress = ReceiveProgress {
            sequence: cover.seq,
            anchor: cover.anchor,
        };
        adopt_checkpoint_on(&tx, &key, &progress)
            .await
            .map_err(internal_error)?;
    }
    tx.commit().await.map_err(internal_error)?;
    Ok(())
}

async fn ensure_tables(db: &DatabaseConnection) -> Result<()> {
    for table in [
        STATE_TABLE,
        LIBRARY_BINDINGS_TABLE,
        UNCONFIRMED_CREATIONS_TABLE,
    ] {
        let mut create = Table::create();
        create.if_not_exists().table(Alias::new(table));
        if table == STATE_TABLE {
            create.col(
                ColumnDef::new(Alias::new("library_id"))
                    .string()
                    .not_null()
                    .primary_key(),
            );
            create.col(ColumnDef::new(Alias::new("state_json")).text().not_null());
        } else {
            create.col(
                ColumnDef::new(Alias::new("library_id"))
                    .string()
                    .not_null()
                    .primary_key(),
            );
        }
        db.execute(&create.to_owned()).await?;
    }
    Ok(())
}

async fn binding_exists(db: &DatabaseConnection, library_id: Uuid) -> Result<bool> {
    let query = Query::select()
        .column(Alias::new("library_id"))
        .from(Alias::new(LIBRARY_BINDINGS_TABLE))
        .and_where(Expr::col(Alias::new("library_id")).eq(library_id.to_string()))
        .to_owned();
    Ok(db.query_one(&query).await?.is_some())
}

fn store_error(error: StoreError) -> SharePullError {
    match error {
        StoreError::Transient(message) => SharePullError::Transient(message),
        StoreError::NotFound => {
            SharePullError::MissingObject("required share object is missing".into())
        }
        StoreError::PermissionDenied => SharePullError::ReadOnly,
        StoreError::Unauthorized => SharePullError::CredentialsRejected,
        other => SharePullError::Store(format!("{other:?}")),
    }
}
fn internal_error(error: impl fmt::Display) -> SharePullError {
    SharePullError::Store(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::model::RuneModel;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::Database;
    use std::sync as std_sync;
    use std::sync::Mutex as StdMutex;
    use std::time as std_time;
    use std::time::Duration;
    use std::time::Instant;
    use sync::protocol;
    use sync::share::{ConditionalGet, MemoryObjectStore, ObjectValue, StoreOperation};
    use tokio::sync as tokio_sync;
    use tokio::sync::Notify;

    #[tokio::test]
    async fn bound_library_removed_reports_changed_without_recreating() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        Migrator::up(&db, None).await?;
        let store = MemoryObjectStore::default();
        let id = Uuid::new_v4();
        validate_share_library::<RuneModel>(
            &db,
            &store,
            id,
            ShareCapabilities {
                conditional_create: true,
                conditional_head_write: false,
                conditional_get: false,
            },
        )
        .await
        .unwrap();
        store.delete(&library_key()).await.unwrap();
        assert_eq!(
            validate_share_library::<RuneModel>(&db, &store, id, ShareCapabilities::default())
                .await
                .unwrap_err(),
            SharePullError::ShareChanged
        );
        assert_eq!(
            store.get(&library_key()).await.unwrap_err(),
            StoreError::NotFound
        );
        Ok(())
    }

    #[tokio::test]
    async fn transient_library_read_is_typed_and_does_not_create_binding() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        Migrator::up(&db, None).await?;
        let store = MemoryObjectStore::default();
        store.inject_error(StoreOperation::Get, StoreError::Transient("offline".into()));
        let result = validate_share_library::<RuneModel>(
            &db,
            &store,
            Uuid::new_v4(),
            ShareCapabilities::default(),
        )
        .await;
        assert!(matches!(result, Err(SharePullError::Transient(_))));
        assert!(store.calls().contains(&StoreOperation::Get));
        Ok(())
    }

    #[tokio::test]
    async fn permission_denied_library_create_is_typed_as_read_only() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        Migrator::up(&db, None).await?;
        let store = MemoryObjectStore::default();
        store.inject_error(StoreOperation::Create, StoreError::PermissionDenied);
        let result = validate_share_library::<RuneModel>(
            &db,
            &store,
            Uuid::new_v4(),
            ShareCapabilities {
                conditional_create: true,
                conditional_head_write: false,
                conditional_get: false,
            },
        )
        .await;
        assert_eq!(result.unwrap_err(), SharePullError::ReadOnly);
        assert_eq!(
            store.list_prefix("devices/").await.unwrap(),
            Vec::<String>::new()
        );
        Ok(())
    }

    #[tokio::test]
    async fn local_share_state_is_durable_and_future_retention_starts_reset() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        Migrator::up(&db, None).await?;
        let library = Uuid::new_v4();
        let replica = Uuid::new_v4();
        let cache_path = PathBuf::from("cache/share");
        let mut state =
            load_or_create_state(&db, replica, library, cache_path.clone(), 100).await?;
        assert_eq!(state.stream, Uuid::new_v5(&replica, b"rune-share"));
        let prefix = DevicePrefix {
            stream: Uuid::from_u128(1),
            generation: Uuid::from_u128(2),
        };
        let key = ObjectKey::Head {
            prefix: prefix.clone(),
            slot: 0,
        };
        state.observe_tag(key.clone(), ObjectTag::new("etag-1"), 800);
        state.observe_tag(key.clone(), ObjectTag::new("etag-2"), 900);
        let observed = state.observed_objects.get_mut(&key).unwrap();
        assert_eq!(observed.tag, ObjectTag::new("etag-2"));
        assert_eq!(observed.first_seen_ms, 800);
        assert_eq!(observed.last_change_ms, 900);
        observed.first_seen_ms = 500;
        observed.last_change_ms = 600;
        observed.gc_state = ObjectGcState::Unreferenced {
            first_seen_ms: 700,
            second_observation: GcObservation::ObservedAt(800),
        };
        let base_key = ObjectKey::Base {
            prefix,
            base_id: "base".into(),
        };
        state.observe_tag(base_key.clone(), ObjectTag::new("base-etag"), 900);
        state.observed_objects.get_mut(&base_key).unwrap().gc_state =
            ObjectGcState::Superseded { since_ms: 900 };
        save_state(&db, &state).await?;
        let reopened = load_or_create_state(&db, replica, library, PathBuf::new(), 200).await?;
        assert_eq!(reopened.chunk_cache_path, cache_path);
        let observed = &reopened.observed_objects[&key];
        assert_eq!(observed.first_seen_ms, 200);
        assert_eq!(observed.last_change_ms, 200);
        assert_eq!(
            observed.gc_state,
            ObjectGcState::Unreferenced {
                first_seen_ms: 200,
                second_observation: GcObservation::ObservedAt(200),
            }
        );
        assert_eq!(
            reopened.observed_objects[&base_key].gc_state,
            ObjectGcState::Superseded { since_ms: 200 }
        );
        assert_eq!(observed.tag, ObjectTag::new("etag-2"));
        Ok(())
    }

    #[tokio::test]
    async fn changed_library_id_under_existing_binding_is_typed() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        Migrator::up(&db, None).await?;
        let store = MemoryObjectStore::default();
        let id = Uuid::new_v4();
        validate_share_library::<RuneModel>(
            &db,
            &store,
            id,
            ShareCapabilities {
                conditional_create: true,
                conditional_head_write: false,
                conditional_get: false,
            },
        )
        .await?;
        let other = LibraryFile {
            format: FORMAT_VERSION,
            library_id: Uuid::new_v4(),
            model: RuneModel::model_id().to_owned(),
            schema: RuneModel::schema_id().to_owned(),
        };
        store
            .put(&library_key(), &serde_json::to_vec(&other)?)
            .await
            .unwrap();
        assert_eq!(
            validate_share_library::<RuneModel>(&db, &store, id, ShareCapabilities::default())
                .await
                .unwrap_err(),
            SharePullError::ShareChanged
        );
        Ok(())
    }

    /// The shared store as one device sees it. Its upload of library.json can be held until the
    /// test lets it land, as a slow upload is, and the view notes how long after the upload landed
    /// the device read library.json back.
    struct DeviceStoreView {
        store: Arc<MemoryObjectStore>,
        held_upload: Option<(Arc<tokio_sync::Notify>, Arc<tokio_sync::Notify>)>,
        upload_landed: std_sync::Mutex<Option<std_time::Instant>>,
        read_back_after: std_sync::Mutex<Option<Duration>>,
    }

    impl DeviceStoreView {
        fn new(
            store: Arc<MemoryObjectStore>,
            held_upload: Option<(Arc<tokio_sync::Notify>, Arc<tokio_sync::Notify>)>,
        ) -> Self {
            Self {
                store,
                held_upload,
                upload_landed: StdMutex::new(None),
                read_back_after: StdMutex::new(None),
            }
        }

        fn read_back_after(&self) -> Option<Duration> {
            *self.read_back_after.lock().unwrap()
        }
    }

    impl ObjectStore for DeviceStoreView {
        fn get<'a>(&'a self, key: &'a str) -> protocol::BoxFuture<'a, ObjectValue, StoreError> {
            Box::pin(async move {
                if key == library_key()
                    && let Some(landed) = *self.upload_landed.lock().unwrap()
                {
                    self.read_back_after
                        .lock()
                        .unwrap()
                        .get_or_insert(landed.elapsed());
                }
                self.store.get(key).await
            })
        }

        fn get_if_none_match<'a>(
            &'a self,
            key: &'a str,
            tag: &'a ObjectTag,
        ) -> protocol::BoxFuture<'a, ConditionalGet, StoreError> {
            self.store.get_if_none_match(key, tag)
        }

        fn put<'a>(
            &'a self,
            key: &'a str,
            bytes: &'a [u8],
        ) -> protocol::BoxFuture<'a, ObjectTag, StoreError> {
            Box::pin(async move {
                if key != library_key() {
                    return self.store.put(key, bytes).await;
                }
                if let Some((started, lands)) = &self.held_upload {
                    started.notify_one();
                    lands.notified().await;
                }
                let tag = self.store.put(key, bytes).await;
                *self.upload_landed.lock().unwrap() = Some(Instant::now());
                tag
            })
        }

        fn create<'a>(
            &'a self,
            key: &'a str,
            bytes: &'a [u8],
        ) -> protocol::BoxFuture<'a, ObjectTag, StoreError> {
            self.store.create(key, bytes)
        }

        fn put_if_match<'a>(
            &'a self,
            key: &'a str,
            tag: &'a ObjectTag,
            bytes: &'a [u8],
        ) -> protocol::BoxFuture<'a, ObjectTag, StoreError> {
            self.store.put_if_match(key, tag, bytes)
        }

        fn delete<'a>(&'a self, key: &'a str) -> protocol::BoxFuture<'a, (), StoreError> {
            self.store.delete(key)
        }

        fn list_prefix<'a>(
            &'a self,
            prefix: &'a str,
        ) -> protocol::BoxFuture<'a, Vec<String>, StoreError> {
            self.store.list_prefix(prefix)
        }
    }

    #[tokio::test]
    async fn concurrent_creators_without_conditional_create_leave_the_share_to_the_last_writer()
    -> Result<()> {
        let store = Arc::new(MemoryObjectStore::default());
        let first_db = Database::connect("sqlite::memory:").await?;
        Migrator::up(&first_db, None).await?;
        let second_db = Database::connect("sqlite::memory:").await?;
        Migrator::up(&second_db, None).await?;
        let first_library = Uuid::new_v4();
        let second_library = Uuid::new_v4();
        let started = Arc::new(Notify::new());
        let lands = Arc::new(Notify::new());
        let first_view = DeviceStoreView::new(store.clone(), None);
        let second_view = Arc::new(DeviceStoreView::new(
            store.clone(),
            Some((started.clone(), lands.clone())),
        ));

        // Both devices find no library.json on a store that cannot create only if absent, and
        // the second device's upload lands after the first device has read the file back.
        let second = tokio::spawn({
            let second_view = second_view.clone();
            async move {
                let result = validate_share_library::<RuneModel>(
                    &second_db,
                    &*second_view,
                    second_library,
                    ShareCapabilities::default(),
                )
                .await;
                (second_db, result)
            }
        });
        started.notified().await;
        validate_share_library::<RuneModel>(
            &first_db,
            &first_view,
            first_library,
            ShareCapabilities::default(),
        )
        .await
        .unwrap();
        lands.notify_one();
        let (second_db, second) = second.await?;
        second.unwrap();

        // A device reads library.json back as soon as its upload lands, with no wait that could
        // only guess whether another upload is still on its way.
        for view in [&first_view, &*second_view] {
            assert!(
                view.read_back_after()
                    .is_some_and(|after| after < Duration::from_secs(5)),
                "library.json was read back {:?} after the upload",
                view.read_back_after()
            );
        }

        // The library that wrote library.json last owns the share. The other device is told on
        // its next validation.
        assert_eq!(
            decode_library(&store.get(&library_key()).await.unwrap().bytes)?.library_id,
            second_library
        );
        assert_eq!(
            validate_share_library::<RuneModel>(
                &first_db,
                &*store,
                first_library,
                ShareCapabilities::default(),
            )
            .await
            .unwrap_err(),
            SharePullError::ShareChanged
        );
        validate_share_library::<RuneModel>(
            &second_db,
            &*store,
            second_library,
            ShareCapabilities::default(),
        )
        .await
        .unwrap();
        Ok(())
    }
}
