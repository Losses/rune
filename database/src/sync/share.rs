//! Local binding and pull orchestration for a library's share source.
use anyhow::{Context, Result, ensure};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, ExprTrait, TransactionTrait,
    sea_query::{Alias, ColumnDef, Expr, OnConflict, Query, Table},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::Duration,
};
use sync::{
    Coordinator, ReceiveProgress, SessionAdvance, StoreIdentity, SyncError, SyncModel,
    protocol::{CheckpointKey, Scope},
    share::{
        BaseManifest, Checkpoint, DevicePrefix, FORMAT_VERSION, FormatError, Head, LibraryFile,
        ObjectKey, ObjectStore, SelectedBase, SharePeer, StoreError, base_key, decode_head,
        decode_library, decode_manifest, head_key, library_key, parse_key,
    },
};
use sync_orm::causal_progress::adopt_checkpoint_on;
use uuid::Uuid;

const STATE_TABLE: &str = "device_share_state";
const LIBRARY_BINDINGS_TABLE: &str = "device_share_bindings";

/// Per-library state lives in local SQLite and is never part of the sync model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareState {
    pub library_id: Uuid,
    pub stream: Uuid,
    pub generation: Uuid,
    pub writer_session: Uuid,
    pub head_counter: u64,
    pub local_cursor: u64,
    pub etags: BTreeMap<String, String>,
    pub last_seen_tags: BTreeMap<String, String>,
    pub first_seen_ms: BTreeMap<String, i64>,
    pub last_change_ms: BTreeMap<String, i64>,
    pub chunk_cache_path: PathBuf,
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
            etags: BTreeMap::new(),
            last_seen_tags: BTreeMap::new(),
            first_seen_ms: BTreeMap::new(),
            last_change_ms: BTreeMap::new(),
            chunk_cache_path: cache_path,
        })
    }

    /// Starts retention timers from now when the persisted clock began in the future.
    pub fn normalize_future_times(&mut self, now_ms: i64) {
        for time in self
            .first_seen_ms
            .values_mut()
            .chain(self.last_change_ms.values_mut())
        {
            if *time > now_ms {
                *time = now_ms;
            }
        }
    }

    pub fn observe_tag(&mut self, object: &str, tag: &str, now_ms: i64) {
        self.first_seen_ms
            .entry(object.to_owned())
            .or_insert(now_ms);
        if self
            .last_seen_tags
            .get(object)
            .is_some_and(|previous| previous != tag)
        {
            self.last_change_ms.insert(object.to_owned(), now_ms);
        } else {
            self.last_change_ms
                .entry(object.to_owned())
                .or_insert(now_ms);
        }
        self.last_seen_tags
            .insert(object.to_owned(), tag.to_owned());
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
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SharePullError {
    #[error("share binding changed or library.json is missing")]
    ShareChanged,
    #[error("share storage temporarily unavailable: {0}")]
    Transient(String),
    #[error("share object is missing or invalid: {0}")]
    Invalid(String),
    #[error("share store error: {0}")]
    Store(String),
}

pub type SharePullResult<T> = std::result::Result<T, SharePullError>;

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
                        observed_tags.push((key, value.tag.as_str().to_owned()));
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
        local_state.etags.insert(key.clone(), tag.clone());
        local_state.observe_tag(&key, &tag, now_ms);
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
            Some(choose_live_base::<M>(&*store, &candidates, &prefix, &head).await?)
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
                    return Err(SharePullError::Store("share pull cancelled".into()));
                }
                Err(error) => return Err(sync_error(error)),
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

struct BaseCandidate {
    owner_prefix: String,
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
    let mut candidates = BTreeMap::<(String, String), BaseCandidate>::new();
    for (device, head) in peers {
        let owner = device.to_string();
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
        let candidate = candidates
            .entry((
                head.floor_base.owner_prefix.clone(),
                head.floor_base.base_id.clone(),
            ))
            .or_insert_with(|| BaseCandidate {
                owner_prefix: head.floor_base.owner_prefix.clone(),
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
) -> SharePullResult<SelectedBase<M>> {
    let target_prefix = target.to_string();
    let mut ordered = Vec::new();
    for candidate in candidates
        .iter()
        .filter(|candidate| candidate.covers.contains_key(&target_prefix))
    {
        let coverage = candidate
            .covers
            .values()
            .try_fold(0_u64, |sum, cover| sum.checked_add(cover.seq))
            .ok_or_else(|| SharePullError::Invalid("base coverage sum overflows".into()))?;
        ordered.push((candidate, coverage));
    }
    ordered.sort_by_key(|(_, coverage)| std::cmp::Reverse(*coverage));
    for (candidate, _) in ordered {
        let key = base_key(&candidate.owner_prefix, &candidate.base_id)
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
        if candidate.owner_prefix == head.floor_base.owner_prefix
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
            SharePullError::Invalid("referenced share base or chunk is missing".into())
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
    async fn local_share_state_is_durable_and_future_retention_starts_reset() -> Result<()> {
        let db = Database::connect("sqlite::memory:").await?;
        Migrator::up(&db, None).await?;
        let library = Uuid::new_v4();
        let replica = Uuid::new_v4();
        let cache_path = PathBuf::from("cache/share");
        let mut state =
            load_or_create_state(&db, replica, library, cache_path.clone(), 100).await?;
        assert_eq!(state.stream, Uuid::new_v5(&replica, b"rune-share"));
        state
            .first_seen_ms
            .insert("devices/x/head.0.json".into(), 500);
        state
            .last_change_ms
            .insert("devices/x/head.0.json".into(), 600);
        state.observe_tag("devices/x/head.0.json", "etag-1", 800);
        save_state(&db, &state).await?;
        let reopened = load_or_create_state(&db, replica, library, PathBuf::new(), 200).await?;
        assert_eq!(reopened.chunk_cache_path, cache_path);
        assert_eq!(reopened.first_seen_ms["devices/x/head.0.json"], 200);
        assert_eq!(reopened.last_change_ms["devices/x/head.0.json"], 200);
        assert_eq!(reopened.last_seen_tags["devices/x/head.0.json"], "etag-1");
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
