//! Local binding and pull orchestration for a library's share source.
use crate::sync::model::RuneModel;
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, ExprTrait, TransactionTrait,
    sea_query::{Alias, ColumnDef, Expr, OnConflict, Query, Table},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::Duration,
};
use sync::{
    Coordinator, Event, ReceiveProgress, SessionAdvance, StoreIdentity, SyncError, SyncModel,
    protocol::{CheckpointKey, Scope},
    share::{
        BaseManifest, BaseRef, BaseSize, Checkpoint, DevicePrefix, ExportCursorStatus,
        FORMAT_VERSION, FloorBase, FloorBaseSelection, FormatError, GcInput, GcObservation, Head,
        InProgressCut, JournalEvent, LibraryFile, ManifestStatus, ObjectGcState, ObjectKey,
        ObjectStore, ObjectTag, OwnBase, OwnChunk, OwnSegment, PublishDecision, PublishInput,
        RetainedManifest, RetirementCandidate, RetirementReadStatus, RollupInput,
        SEGMENT_RETENTION_MS, Segment, SegmentLink, SegmentRef, SelectedBase, SharePeer,
        StoreError, SupersededSince, base_key, chunk_key, decode_head, decode_library,
        decode_manifest, decode_segment, encode_head, encode_segment, head_key, library_key,
        parse_key, plan_gc, plan_publish, segment_key,
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

/// Per-library state lives in local SQLite and is never part of the sync model.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShareState {
    pub library_id: Uuid,
    pub stream: Uuid,
    pub generation: Uuid,
    pub writer_session: Uuid,
    pub head_counter: u64,
    pub local_cursor: u64,
    #[serde(with = "observed_object_map")]
    pub observed_objects: BTreeMap<ObjectKey, ObservedObject>,
    pub unavailable_bases: BTreeSet<(DevicePrefix, String)>,
    pub chunk_cache_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedObject {
    pub tag: ObjectTag,
    pub first_seen_ms: i64,
    pub last_change_ms: i64,
    pub gc_state: ObjectGcState,
}

mod observed_object_map {
    use super::{ObjectKey, ObservedObject};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S>(
        objects: &BTreeMap<ObjectKey, ObservedObject>,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        objects.iter().collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> std::result::Result<BTreeMap<ObjectKey, ObservedObject>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = Vec::<(ObjectKey, ObservedObject)>::deserialize(deserializer)?;
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
            unavailable_bases: BTreeSet::new(),
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
    #[error("share pull cancelled")]
    Cancelled,
}

pub type SharePullResult<T> = std::result::Result<T, SharePullError>;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SharePushError {
    #[error("share needs a base cut before incremental publication")]
    NeedsBase,
    #[error("share head changed during publication")]
    StaleHead,
    #[error("share is read-only")]
    ReadOnly,
    #[error("share binding changed")]
    ShareChanged,
    #[error("share push failed: {0}")]
    Failed(String),
}

pub type SharePushResult<T> = std::result::Result<T, SharePushError>;

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
    let mut retained_chunk_names = BTreeSet::new();
    for base in latest.iter().flat_map(|head| head.bases.iter()) {
        let key = base_key(&prefix.to_string(), &base.base_id)
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
        let value = store.get(&key).await.map_err(push_store_error)?;
        let manifest = decode_manifest::<M>(&value.bytes, &base.base_id)
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
        retained_chunk_names.extend(
            manifest.collections.values().flat_map(|collection| {
                collection.buckets.iter().map(|bucket| bucket.chunk.clone())
            }),
        );
        retained_chunk_names.extend(
            manifest
                .collections
                .values()
                .flat_map(|collection| collection.context_fragments.iter().cloned()),
        );
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
        let key = if index + 1 == export.objects.len() {
            base_key(&prefix.to_string(), &object.name)
        } else {
            chunk_key(&prefix.to_string(), &object.name)
        }
        .map_err(|error| SharePushError::Failed(error.to_string()))?;
        match store.create(&key, &object.bytes).await {
            Ok(_) => (),
            Err(StoreError::PreconditionFailed) => {
                if store.get(&key).await.map_err(push_store_error)?.bytes != object.bytes {
                    return Err(SharePushError::Failed(
                        "immutable base object name collision".into(),
                    ));
                }
            }
            Err(error) => return Err(push_store_error(error)),
        }
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
        store.create(&key, &bytes).await.map_err(push_store_error)?;
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
    for key in &listed {
        let parsed = parse_key(key).map_err(|error| SharePushError::Failed(error.to_string()))?;
        let object_prefix = match &parsed {
            ObjectKey::Head { prefix, .. }
            | ObjectKey::Segment { prefix, .. }
            | ObjectKey::Base { prefix, .. }
            | ObjectKey::Chunk { prefix, .. } => prefix,
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
        match store.get(key).await {
            Ok(value) => {
                state.observe_tag(parsed, value.tag.clone(), now_ms as i64);
            }
            Err(StoreError::NotFound) => (),
            Err(error) => return Err(push_store_error(error)),
        }
    }

    let mut manifests = Vec::new();
    let mut own_bases = Vec::new();
    for base in &latest.bases {
        let key = base_key(&prefix.to_string(), &base.base_id)
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
        let object = store.get(&key).await.map_err(push_store_error)?;
        let manifest = decode_manifest::<RuneModel>(&object.bytes, &base.base_id)
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
        let superseded = latest.bases.iter().any(|candidate| {
            candidate.base_id != base.base_id && covers_at_least(&candidate.covers, &base.covers)
        });
        let object_key = ObjectKey::Base {
            prefix: prefix.clone(),
            base_id: base.base_id.clone(),
        };
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
        let referenced_chunks = manifest
            .collections
            .values()
            .flat_map(|collection| {
                collection
                    .buckets
                    .iter()
                    .map(|bucket| bucket.chunk.clone())
                    .chain(collection.context_fragments.iter().cloned())
            })
            .collect();
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
            reference.clone()
        } else {
            let object = match store.get(key).await {
                Ok(object) => object,
                Err(StoreError::NotFound) => continue,
                Err(error) => return Err(push_store_error(error)),
            };
            let segment = decode_segment::<RuneModel>(&object.bytes)
                .map_err(|error| SharePushError::Failed(error.to_string()))?;
            SegmentRef {
                level: 0,
                first,
                last,
                prev_anchor: segment.prev_anchor,
                last_anchor: segment.last_anchor,
                name: hash16,
                bytes: object.bytes.len() as u64,
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
    for precondition in decision.retirement_preconditions {
        let retired = precondition.prefix.to_string();
        if !latest.retired.contains(&retired) {
            continue;
        }
        let listed = store
            .list_prefix(&format!("devices/{retired}/"))
            .await
            .map_err(push_store_error)?;
        let mut verified = false;
        for slot in [0, 1] {
            let key = head_key(&retired, slot)
                .map_err(|error| SharePushError::Failed(error.to_string()))?;
            match store.get(&key).await {
                Ok(value) => {
                    let remote = decode_head(&value.bytes)
                        .map_err(|error| SharePushError::Failed(error.to_string()))?;
                    if remote.stream == precondition.stream_id
                        && remote.generation == precondition.generation
                        && remote.counter == precondition.expected_counter
                        && remote.head.seq == precondition.expected_head.seq
                        && remote.head.anchor == precondition.expected_head.anchor
                    {
                        verified = true;
                    }
                }
                Err(StoreError::NotFound) => (),
                Err(error) => return Err(push_store_error(error)),
            }
        }
        if !verified {
            continue;
        }
        for key in listed {
            let object_prefix =
                match parse_key(&key).map_err(|error| SharePushError::Failed(error.to_string()))? {
                    ObjectKey::Head { prefix, .. }
                    | ObjectKey::Segment { prefix, .. }
                    | ObjectKey::Base { prefix, .. }
                    | ObjectKey::Chunk { prefix, .. } => prefix,
                    _ => continue,
                };
            if object_prefix != precondition.prefix {
                return Err(SharePushError::Failed(
                    "retirement listing returned an object outside its prefix".into(),
                ));
            }
            match store.delete(&key).await {
                Ok(()) | Err(StoreError::NotFound) => (),
                Err(error) => return Err(push_store_error(error)),
            }
        }
    }
    save_state(db, state)
        .await
        .map_err(|error| SharePushError::Failed(error.to_string()))?;
    Ok(final_head)
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
                match store.put(&key, &bytes).await {
                    Ok(_) => (),
                    Err(error) => return Err(store_error(error)),
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
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
    Ok(library)
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
    peers.sort_by_key(|(_, head)| std::cmp::Reverse(head.head.seq));
    let candidates = base_candidates(&peers)?;
    record_library_binding(db, library_id).await?;
    let mut local_state = load_or_create_state(
        db,
        local_identity.replica,
        library_id,
        cache_path.clone(),
        chrono::Utc::now().timestamp_millis(),
    )
    .await
    .map_err(internal_error)?;
    let now_ms = chrono::Utc::now().timestamp_millis();
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
        let applied = local
            .checkpoint(&checkpoint)
            .await
            .map_err(|error| SharePullError::Store(error.to_string()))?;
        let needs_base = applied.is_none_or(|received| received.sequence < head.floor.seq);
        let selected_base = if needs_base {
            Some(
                choose_live_base::<M>(
                    &*store,
                    &candidates,
                    &prefix,
                    &head,
                    &local_state.unavailable_bases,
                )
                .await?,
            )
        } else {
            None
        };
        if head.stream == local_state.stream {
            local_state.generation = head.generation;
            local_state.writer_session = head.writer_session;
            local_state.head_counter = head.counter;
            local_state.local_cursor = head.local_cursor;
        }
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
        let mut result = None;
        for _ in 0..100_000 {
            match coordinator
                .advance(local, sink, local_source, &peer, &peer)
                .await
            {
                Ok(SessionAdvance::Complete(done)) => {
                    result = Some(done);
                    break;
                }
                Ok(SessionAdvance::BudgetExhausted) => coordinator.start_pass(),
                Ok(SessionAdvance::Incomplete) => (),
                Ok(SessionAdvance::Cancelled) => {
                    return Err(SharePullError::Cancelled);
                }
                Err(error) => {
                    if matches!(&error, SyncError::SnapshotSourceUnavailable)
                        && let Some(base) = &selected_base
                    {
                        local_state
                            .unavailable_bases
                            .insert((base.owner_prefix.clone(), base.base_id.clone()));
                        save_state(db, &local_state).await.map_err(internal_error)?;
                    }
                    return Err(sync_error(error));
                }
            }
        }
        let done = result.ok_or_else(|| {
            SharePullError::Store("share coordinator exceeded its work bound".into())
        })?;
        progress.push(done);
        if let Some(base) = selected_base {
            adopt_manifest_covers(db, &local_identity, &base.manifest, &scope).await?;
        }
    }
    save_state(db, &local_state).await.map_err(internal_error)?;
    Ok(progress)
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
        newest_base_gzip_bytes: newest_base,
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
        match store.create(&key, &bytes).await {
            Ok(_) => (),
            Err(StoreError::PreconditionFailed) => {
                let existing = store.get(&key).await.map_err(push_store_error)?;
                if existing.bytes != bytes {
                    return Err(SharePushError::Failed(
                        "immutable segment name contains different bytes".into(),
                    ));
                }
            }
            Err(error) => return Err(push_store_error(error)),
        }
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
        match store.create(&key, &bytes).await {
            Ok(_) => (),
            Err(StoreError::PreconditionFailed) => {
                if store.get(&key).await.map_err(push_store_error)?.bytes != bytes {
                    return Err(SharePushError::Failed(
                        "immutable roll-up name collision".into(),
                    ));
                }
            }
            Err(error) => return Err(push_store_error(error)),
        }
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
        StoreError::Transient(message) => SharePushError::Failed(format!("transient: {message}")),
        StoreError::NotFound => SharePushError::Failed("share object disappeared".into()),
        other => SharePushError::Failed(format!("{other:?}")),
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
        candidate.covers.contains_key(&target_prefix)
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
    ordered.sort_by_key(|(_, coverage)| std::cmp::Reverse(*coverage));
    for (candidate, _) in ordered {
        let key = base_key(&candidate.owner_prefix.to_string(), &candidate.base_id)
            .map_err(|error| SharePullError::Invalid(error.to_string()))?;
        let object = match store.get(&key).await {
            Ok(object) => object,
            Err(StoreError::NotFound) => continue,
            Err(error) => return Err(store_error(error)),
        };
        let manifest = decode_manifest::<M>(&object.bytes, &candidate.base_id)
            .map_err(|error| SharePullError::Invalid(error.to_string()))?;
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
    Err(SharePullError::Invalid(
        "no live base covers the required checkpoint".into(),
    ))
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
    for table in [STATE_TABLE, LIBRARY_BINDINGS_TABLE] {
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
        StoreError::NotFound => SharePullError::Invalid("required share object is missing".into()),
        StoreError::PermissionDenied => SharePullError::ReadOnly,
        other => SharePullError::Store(format!("{other:?}")),
    }
}
fn internal_error(error: impl std::fmt::Display) -> SharePullError {
    SharePullError::Store(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::model::RuneModel;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::Database;
    use sync::share::{MemoryObjectStore, StoreOperation};

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
}
