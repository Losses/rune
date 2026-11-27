//! Rune library composition and transactional receive.
use super::SeaOrmSyncStore;
use super::model::Collection;
use super::model::{ObjectKey, Payload, Record, RuneModel};
use super::references::collect;
use super::share::{
    BaseCutRequest, LostCreation, PublishJournalRequest, ShareCapabilities, SharePullError,
    SharePullRequest, SharePullResult, SharePushError, SharePushResult, ShareWriter,
    base_gzip_sizes, collect_own_prefix, cut_base, discard_lost_creation, load_or_create_state,
    publish_journal, pull, push_store_error,
};
use crate::sync::{import, legacy};
use anyhow::{Result, ensure};
use chrono::Utc;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, SqlxSqliteConnector, Statement,
    TransactionTrait,
    sea_query::{Alias, Expr, Query},
    sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use sync::{
    Candidate, EventRef, ExactContext, MAX_MUTATIONS, MergeDelta, ModelId, PageRead, ReceiveStore,
    Revision, StoreIdentity, SyncModel, SyncPeer, SyncStore, candidate_digest, candidate_rank,
    decode_candidate,
    share::{
        BaseSize, Checkpoint, DevicePrefix, ExportCursorStatus, FormatError, Head, JournalEvent,
        KeeperDecision, NewestBase, ObjectKey as ShareObjectKey, ObjectStore, PublishDecision,
        PublishInput, StoreError, StreamActivity, decode_head, head_key, parse_key, plan_keeper,
        plan_publish,
    },
};
use sync_orm::causal_progress;
use sync_orm::causal_store;
use sync_orm::maintenance;
use sync_orm::{
    causal_candidates::{CandidatePageRequest, CandidateRow, page_on as candidate_page_on},
    causal_capabilities::CausalEndpoint,
    causal_export_cursors::read_on,
    causal_leases::lock_on,
    causal_schema::clear_namespace_on,
    causal_store::namespace_model_on,
    registry::{ProjectionBudget, RegistryProjection},
};
use uuid::Uuid;

/// Read the catalog identity without initializing or migrating the retained database.
pub(crate) async fn retained_catalog_id(path: &Path) -> Result<Option<Uuid>> {
    if !path.try_exists()? {
        return Ok(None);
    }
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(path).read_only(true))
        .await?;
    let db = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);
    let table = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'sync_meta'",
        ))
        .await?;
    let namespaces = if table.is_some() {
        causal_store::list_namespace_ids_on(&db, 2).await?
    } else {
        Vec::new()
    };
    db.close().await?;
    ensure!(
        namespaces.len() <= 1,
        "multiple namespaces require explicit selection"
    );
    Ok(namespaces
        .first()
        .copied()
        .filter(|id| *id != legacy::namespace()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Schema1PlaybackStatistics;

impl SyncModel for Schema1PlaybackStatistics {
    type Collection = Collection;
    type Payload = Payload;

    fn model_id() -> &'static str {
        RuneModel::model_id()
    }

    fn schema_id() -> &'static str {
        "1"
    }

    fn collections() -> Vec<Collection> {
        RuneModel::collections()
    }

    fn collection_tag(collection: &Collection) -> &'static str {
        RuneModel::collection_tag(collection)
    }

    fn collection_from_tag(tag: &str) -> Option<Collection> {
        RuneModel::collection_from_tag(tag)
    }

    fn validate(key: &sync::ObjectKey<Self>, value: &Option<Payload>) -> sync::Result<()> {
        RuneModel::validate(
            &ObjectKey {
                collection: key.collection,
                id: key.id,
            },
            value,
        )
    }

    fn validate_revision(
        key: &sync::ObjectKey<Self>,
        value: &Option<Payload>,
        revision: &Revision,
    ) -> sync::Result<()> {
        RuneModel::validate_revision(
            &ObjectKey {
                collection: key.collection,
                id: key.id,
            },
            value,
            revision,
        )
    }
}

fn decode_schema_1_statistics_candidate(
    namespace: Uuid,
    row: &CandidateRow,
) -> Result<Candidate<Schema1PlaybackStatistics>> {
    let candidate = decode_candidate::<Schema1PlaybackStatistics>(&row.body)?;
    ensure!(
        candidate.namespace == namespace
            && candidate.key.collection == Collection::PlaybackStatistics
            && candidate.key.id.as_bytes() == row.object.as_slice()
            && candidate.dot.writer_incarnation.as_bytes() == row.writer.as_slice()
            && i64::try_from(candidate.dot.counter)? == row.counter
            && candidate.revision.actor.as_bytes() == row.author.as_slice()
            && i64::try_from(candidate.revision.counter)? == row.revision
            && candidate_digest(&candidate)?.as_slice() == row.digest.as_slice(),
        "stored schema 1 statistics candidate identity mismatch"
    );
    Ok(candidate)
}

fn select_schema_1_winner(
    candidates: &[Candidate<Schema1PlaybackStatistics>],
) -> Result<Option<Candidate<RuneModel>>> {
    type CandidateRank = (Revision, sync::Digest, sync::Dot);
    let mut best: Option<(CandidateRank, &Candidate<Schema1PlaybackStatistics>)> = None;
    for candidate in candidates {
        let rank = candidate_rank(candidate)?;
        if best.as_ref().is_none_or(|(previous, _)| rank > *previous) {
            best = Some((rank, candidate));
        }
    }
    Ok(best.map(|(_, winner)| Candidate {
        model: ModelId {
            model: RuneModel::model_id().into(),
            schema: RuneModel::schema_id().into(),
        },
        namespace: winner.namespace,
        key: ObjectKey {
            collection: winner.key.collection,
            id: winner.key.id,
        },
        dot: winner.dot,
        revision: winner.revision,
        payload: winner.payload.clone(),
    }))
}

async fn collect_schema_1_accepted_statistics(
    tx: &DatabaseTransaction,
    namespace: Uuid,
    state_seq: i64,
) -> Result<Vec<Candidate<RuneModel>>> {
    let tag = RuneModel::collection_tag(&Collection::PlaybackStatistics).as_bytes();
    let mut after = None;
    let mut current_key: Option<Uuid> = None;
    let mut current_group = Vec::new();
    let mut winners = Vec::new();
    loop {
        let page = candidate_page_on(
            tx,
            CandidatePageRequest {
                namespace,
                collection: tag,
                state_seq,
                lower: None,
                upper: None,
                after: after.clone(),
                max_items: 128,
                max_bytes: 4 * 1024 * 1024,
            },
        )
        .await?;
        for row in &page.rows {
            let candidate = decode_schema_1_statistics_candidate(namespace, row)?;
            if current_key != Some(candidate.key.id) {
                if let Some(winner) = select_schema_1_winner(&current_group)? {
                    winners.push(winner);
                }
                current_group.clear();
                current_key = Some(candidate.key.id);
            }
            current_group.push(candidate);
        }
        if page.done {
            break;
        }
        ensure!(
            page.next != after,
            "schema 1 statistics traversal made no progress"
        );
        after = page.next;
    }
    if let Some(winner) = select_schema_1_winner(&current_group)? {
        winners.push(winner);
    }
    Ok(winners)
}

async fn rebuild_schema_1_encoding(
    db: &DatabaseConnection,
    namespace: Uuid,
    node_id: Uuid,
    file_token: Option<Uuid>,
) -> Result<Option<SeaOrmSyncStore>> {
    use sea_orm::ExprTrait as _;
    let tx = db.begin().await?;
    lock_on(&tx, namespace).await?;
    let meta_query = Query::select()
        .columns(
            [
                "model",
                "schema",
                "codec",
                "replica",
                "generation",
                "state_seq",
                "revision_counter",
            ]
            .map(Alias::new),
        )
        .from(Alias::new("sync_meta"))
        .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
        .to_owned();
    let Some(row) = tx.query_one(&meta_query).await? else {
        tx.commit().await?;
        return Ok(None);
    };
    let model: String = row.try_get_by_index(0)?;
    let schema: String = row.try_get_by_index(1)?;
    if model != RuneModel::model_id() || schema != "1" {
        tx.commit().await?;
        return Ok(None);
    }
    let codec: String = row.try_get_by_index(2)?;
    ensure!(codec == "canonical-json-1", "incompatible schema 1 codec");
    let replica = Uuid::from_slice(&row.try_get_by_index::<Vec<u8>>(3)?)?;
    let generation = Uuid::from_slice(&row.try_get_by_index::<Vec<u8>>(4)?)?;
    let state_seq: i64 = row.try_get_by_index(5)?;
    let revision_counter: i64 = row.try_get_by_index(6)?;
    ensure!(
        !replica.is_nil() && !generation.is_nil() && state_seq >= 0 && revision_counter >= 0,
        "invalid schema 1 replica metadata"
    );
    let old_identity = StoreIdentity {
        namespace,
        model: ModelId { model, schema },
        replica,
        generation,
    };

    // Preserve accepted playback statistics across the schema 1 -> 2 encoding rebuild so
    // records owned by other devices remain forwardable without authoring them as local.
    let preserved_stats = collect_schema_1_accepted_statistics(&tx, namespace, state_seq).await?;

    // Candidate digests bind the old JSON arrays; rebuild causal history from source rows
    // while restoring previously accepted statistics atomically in the same transaction.
    clear_namespace_on(&tx, namespace).await?;
    let inner =
        SeaOrmSyncStore::create_empty_on(&tx, db.clone(), namespace, node_id, file_token).await?;
    if revision_counter > 0 {
        let update = Query::update()
            .table(Alias::new("sync_meta"))
            .value(Alias::new("revision_counter"), revision_counter)
            .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
            .to_owned();
        ensure!(
            tx.execute(&update).await?.rows_affected() == 1,
            "missing causal namespace after schema 2 creation"
        );
    }
    let apply_limits = super::session_limits().apply;
    let chunk_size = apply_limits.max_projection_items as usize;
    for chunk in preserved_stats.chunks(chunk_size) {
        let delta = MergeDelta {
            collection: Collection::PlaybackStatistics,
            added: chunk.to_vec(),
            observed: ExactContext {
                writers: Vec::new(),
            },
        };
        let mut budget = MAX_MUTATIONS;
        let (_, batch) = inner
            .apply_merge_on(&tx, &delta, &old_identity, apply_limits, &mut budget)
            .await?;
        for change in &batch.changes {
            let Some(winner) = change.after.as_ref() else {
                continue;
            };
            if winner.revision.actor == node_id {
                match RuneModel::encode_on(&tx, &change.key).await? {
                    None => {
                        RuneModel::project_on(&tx, &change.key, Some(&winner.payload)).await?;
                    }
                    Some(local_payload) if local_payload != winner.payload => {
                        inner
                            .author_on(&tx, change.key.clone(), Some(local_payload.clone()))
                            .await?;
                        RuneModel::project_on(&tx, &change.key, Some(&local_payload)).await?;
                    }
                    Some(_) => {}
                }
            } else {
                RuneModel::project_on(&tx, &change.key, Some(&winner.payload)).await?;
            }
        }
    }
    tx.commit().await?;
    Ok(Some(inner))
}

#[derive(Clone)]
pub struct LibrarySyncStore {
    db: DatabaseConnection,
    inner: SeaOrmSyncStore,
    endpoint: CausalEndpoint<RuneModel>,
    share_writer: ShareWriter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingShareChange {
    Content,
    StatisticsOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SharePendingChanges {
    pub journal_head: u64,
    pub change: PendingShareChange,
}

impl LibrarySyncStore {
    /// Return this library's persisted share prefix, creating local share state if needed.
    pub async fn share_prefix(&self, cache_path: PathBuf) -> Result<String> {
        let identity = SyncPeer::identity(&self.endpoint).await?;
        let state = load_or_create_state(
            &self.db,
            identity.replica,
            identity.namespace,
            cache_path,
            Utc::now().timestamp_millis(),
        )
        .await?;
        Ok(format!("{}.{}", state.stream, state.generation))
    }
    /// Classify the local journal tail after the share export cursor for timer selection.
    /// A tail that compaction has already passed counts as content: it is published as a base.
    pub async fn share_pending_changes(
        &self,
        cache_path: PathBuf,
    ) -> Result<Option<SharePendingChanges>> {
        let identity = SyncPeer::identity(&self.endpoint).await?;
        let state = load_or_create_state(
            &self.db,
            identity.replica,
            identity.namespace,
            cache_path,
            Utc::now().timestamp_millis(),
        )
        .await?;
        let journal = self.inner.clone();
        let through = SyncPeer::head(&self.endpoint).await?;
        if state.local_cursor >= through.sequence {
            return Ok(None);
        }
        let scope = sync::Scope {
            collections: RuneModel::collections().into_iter().collect(),
        };
        let Some(mut after) = journal.progress_at(state.local_cursor).await? else {
            return Ok(Some(SharePendingChanges {
                journal_head: through.sequence,
                change: PendingShareChange::Content,
            }));
        };
        let mut change = PendingShareChange::StatisticsOnly;
        while after.sequence < through.sequence {
            let page_read = journal
                .read_page_with_origins(sync::PageRequest {
                    source: identity.clone(),
                    scope: scope.clone(),
                    after: after.clone(),
                    through: through.clone(),
                    limits: sync::PageLimits {
                        max_scanned: sync::MAX_PAGE_ITEMS,
                        max_bytes: sync::MAX_PAGE_BYTES,
                    },
                })
                .await?;
            match page_read.read {
                PageRead::Page(page) => {
                    if page
                        .links
                        .iter()
                        .any(|link| link.collection != Collection::PlaybackStatistics)
                    {
                        change = PendingShareChange::Content;
                    }
                    after = page.next;
                    if page.done {
                        break;
                    }
                }
                PageRead::FragmentNeeded { collection, .. } => {
                    if collection != Collection::PlaybackStatistics {
                        change = PendingShareChange::Content;
                    }
                    break;
                }
            }
        }
        Ok(Some(SharePendingChanges {
            journal_head: through.sequence,
            change,
        }))
    }
    pub fn record_store(&self) -> SeaOrmSyncStore {
        self.inner.clone()
    }
    /// This store for writes that need `permit`: each write through [`Self::record_store`]
    /// confirms it once it holds the library's write lock.
    pub fn with_commit_permit(&self, permit: Arc<dyn causal_store::CommitPermit>) -> Self {
        Self {
            inner: self.inner.with_commit_permit(permit),
            ..self.clone()
        }
    }
    /// Persist the writer counter before the owning library database is closed.
    pub async fn clean_close(&self) -> Result<()> {
        self.inner.clean_close().await
    }
    /// Explicitly traverse the registry; reads never initiate source import.
    async fn import_collections(&self, collections: &[Collection]) -> Result<()> {
        if causal_progress::initialization_complete_on(&self.db, self.inner.namespace()).await? {
            return Ok(());
        }
        for (index, collection) in collections.iter().enumerate() {
            let mut after = None;
            loop {
                let tx = self.db.begin().await?;
                let page = import::batch(&tx, &self.inner, collection, after).await?;
                if page.done && index + 1 == collections.len() {
                    causal_progress::complete_initialization_on(&tx, self.inner.namespace())
                        .await?;
                }
                tx.commit().await?;
                if page.done {
                    break;
                }
                ensure!(page.next != after, "source traversal made no progress");
                after = page.next;
            }
        }
        Ok(())
    }
    pub async fn import_basic(&self) -> Result<()> {
        self.import_collections(&<RuneModel as SyncModel>::collections())
            .await
    }
    pub async fn import_existing(&self, device_id: Uuid) -> Result<()> {
        ensure!(
            device_id == self.inner.author(),
            "import actor differs from local device"
        );
        self.import_basic().await
    }
    pub async fn open(
        db: DatabaseConnection,
        node_id: Uuid,
        library_id: Option<Uuid>,
    ) -> Result<Self> {
        Self::open_with_file_token(db, node_id, library_id, None).await
    }
    pub async fn open_with_file_token(
        db: DatabaseConnection,
        node_id: Uuid,
        library_id: Option<Uuid>,
        file_token: Option<Uuid>,
    ) -> Result<Self> {
        ensure!(!node_id.is_nil(), "actor identity cannot be nil");
        ensure!(
            library_id.is_none_or(|id| !id.is_nil()),
            "namespace identity cannot be nil"
        );
        RuneModel::validate_database_on(&db).await?;
        let mut existing = causal_store::list_namespace_ids_on(&db, 2).await?;
        ensure!(
            existing.len() <= 1,
            "multiple namespaces require explicit selection"
        );
        let reset = existing.as_slice() == [legacy::namespace()]
            && library_id.is_some_and(|id| id != legacy::namespace());
        if reset {
            legacy::reset(&db, node_id).await?;
            existing.clear();
        }
        let actual = existing
            .first()
            .copied()
            .or(library_id)
            .unwrap_or_else(Uuid::new_v4);
        ensure!(
            library_id.is_none_or(|id| id == actual),
            "namespace mismatch"
        );
        let rebuilt = if !existing.is_empty()
            && let Some(model) = namespace_model_on(&db, actual).await?
            && model.model == RuneModel::model_id()
            && model.schema == "1"
        {
            rebuild_schema_1_encoding(&db, actual, node_id, file_token).await?
        } else {
            None
        };
        let rebuild_encoding = rebuilt.is_some();
        let inner = if let Some(inner) = rebuilt {
            inner
        } else if existing.is_empty() {
            SeaOrmSyncStore::create_empty_with_file_token(db.clone(), actual, node_id, file_token)
                .await?
        } else {
            SeaOrmSyncStore::open_with_file_token(db.clone(), actual, node_id, file_token).await?
        };
        let endpoint = CausalEndpoint::with_projection(
            inner.clone(),
            Arc::new(RegistryProjection::<RuneModel>::new(ProjectionBudget {
                max_reads: 65536,
                max_mutations: 65536,
            })?),
        );
        let library = Self {
            db,
            inner,
            endpoint,
            share_writer: ShareWriter::default(),
        };
        if reset || rebuild_encoding {
            library.import_existing(node_id).await?;
        }
        Ok(library)
    }
    pub async fn maintain(&self, state: &mut maintenance::Maintenance) -> Result<()> {
        state
            .advance::<RuneModel>(
                &self.db,
                self.identity().await?,
                Utc::now().timestamp_millis(),
            )
            .await?;
        Ok(())
    }
    pub fn endpoint(&self) -> CausalEndpoint<RuneModel> {
        self.endpoint.clone()
    }
    /// Pull every discovered share stream into this library through the normal causal endpoint.
    pub async fn pull_share(
        &self,
        store: Arc<dyn ObjectStore>,
        library_id: Uuid,
        cache_path: PathBuf,
        capabilities: ShareCapabilities,
    ) -> SharePullResult<Vec<sync::ReceiveProgress>> {
        let result = pull::<RuneModel>(SharePullRequest {
            db: &self.db,
            local: &self.endpoint,
            sink: &self.endpoint,
            local_source: &self.endpoint,
            store: store.clone(),
            library_id,
            scope: sync::Scope {
                collections: RuneModel::collections().into_iter().collect(),
            },
            limits: super::session_limits(),
            cache_path,
            capabilities,
        })
        .await;
        if matches!(result, Err(SharePullError::ShareChanged)) {
            self.discard_lost_share_creation(&*store, library_id).await;
        }
        result
    }
    /// Pull every stream through Continuous, then publish this device's local journal tail.
    pub async fn push_share(
        &self,
        store: Arc<dyn ObjectStore>,
        library_id: Uuid,
        cache_path: PathBuf,
        capabilities: ShareCapabilities,
    ) -> SharePushResult<Option<Head>> {
        let result = self
            .push_share_pass(store.clone(), library_id, cache_path, capabilities)
            .await;
        if matches!(result, Err(SharePushError::ShareChanged)) {
            self.discard_lost_share_creation(&*store, library_id).await;
        }
        result
    }
    /// Removes what this device wrote to a share whose library.json another library wrote after
    /// this device created it. The share sync failure itself is reported by the caller.
    async fn discard_lost_share_creation(&self, store: &dyn ObjectStore, library_id: Uuid) {
        match discard_lost_creation(&self.db, store, &self.share_writer, library_id).await {
            Ok(LostCreation::PrefixRemoved) => log::info!(
                "Share for library {library_id} belongs to another library; removed this device's prefix"
            ),
            Ok(LostCreation::Kept) => (),
            Err(error) => log::warn!(
                "Could not remove this device's prefix from the share of another library: {error:#}"
            ),
        }
    }
    async fn push_share_pass(
        &self,
        store: Arc<dyn ObjectStore>,
        library_id: Uuid,
        cache_path: PathBuf,
        capabilities: ShareCapabilities,
    ) -> SharePushResult<Option<Head>> {
        self.pull_share(store.clone(), library_id, cache_path.clone(), capabilities)
            .await
            .map_err(|error| match error {
                SharePullError::ShareChanged => SharePushError::ShareChanged,
                SharePullError::ReadOnly => SharePushError::ReadOnly,
                SharePullError::CredentialsRejected => SharePushError::CredentialsRejected,
                SharePullError::MissingObject(_) => SharePushError::MissingObject,
                other => SharePushError::Failed(other.to_string()),
            })?;

        let identity = SyncPeer::identity(&self.endpoint)
            .await
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
        let mut state = load_or_create_state(
            &self.db,
            identity.replica,
            library_id,
            cache_path,
            Utc::now().timestamp_millis(),
        )
        .await
        .map_err(|error| SharePushError::Failed(error.to_string()))?;

        let listed = store
            .list_prefix("devices/")
            .await
            .map_err(push_store_error)?;
        let mut heads = BTreeMap::<DevicePrefix, Head>::new();
        for key in listed {
            let ShareObjectKey::Head { prefix, slot } =
                parse_key(&key).map_err(|error| SharePushError::Failed(error.to_string()))?
            else {
                continue;
            };
            let key = head_key(&prefix.to_string(), slot)
                .map_err(|error| SharePushError::Failed(error.to_string()))?;
            let value = match store.get(&key).await {
                Ok(value) => value,
                Err(StoreError::NotFound) => continue,
                Err(error) => return Err(push_store_error(error)),
            };
            let head = match decode_head(&value.bytes) {
                Ok(head) => head,
                Err(FormatError::Checksum) => continue,
                Err(error) => return Err(SharePushError::Failed(error.to_string())),
            };
            if head.stream != prefix.stream || head.generation != prefix.generation {
                return Err(SharePushError::Failed(
                    "head identity does not match its object key".into(),
                ));
            }
            if head.library_id != library_id {
                continue;
            }
            if heads
                .get(&prefix)
                .is_none_or(|existing| head.counter > existing.counter)
            {
                heads.insert(prefix, head);
            }
        }
        let own_prefix = DevicePrefix {
            stream: state.stream,
            generation: state.generation,
        };
        let scope = sync::Scope {
            collections: RuneModel::collections().into_iter().collect(),
        };
        let scope_id =
            sync::scope_id(&scope).map_err(|error| SharePushError::Failed(error.to_string()))?;
        if !heads.contains_key(&own_prefix) {
            let mut covers = BTreeMap::new();
            for (prefix, head) in &heads {
                let checkpoint = sync::CheckpointKey {
                    namespace: identity.namespace,
                    source_replica: head.stream,
                    source_generation: head.generation,
                    scope_id,
                };
                if let Some(progress) = self
                    .endpoint
                    .checkpoint(&checkpoint)
                    .await
                    .map_err(|error| SharePushError::Failed(error.to_string()))?
                {
                    covers.insert(
                        prefix.to_string(),
                        Checkpoint {
                            seq: progress.sequence,
                            anchor: progress.anchor,
                        },
                    );
                }
            }
            let head = cut_base(BaseCutRequest {
                db: &self.db,
                state: &mut state,
                writer: &self.share_writer,
                store: &*store,
                identity: &identity,
                source: &self.endpoint,
                current_head: None,
                covers,
                capabilities,
            })
            .await?;
            let head = collect_own_prefix(
                &self.db,
                &mut state,
                &self.share_writer,
                &*store,
                &head,
                &heads,
                capabilities,
            )
            .await?;
            return Ok(Some(head));
        }
        let current_head = heads
            .get(&own_prefix)
            .cloned()
            .ok_or(SharePushError::NeedsBase)?;
        let mut acknowledgements = BTreeMap::new();
        let mut share_stream_ids = BTreeSet::new();
        let mut share_local_replicas = BTreeSet::new();
        for (prefix, head) in &heads {
            share_stream_ids.insert(head.stream);
            share_local_replicas.insert(head.local_replica);
            if prefix == &own_prefix {
                continue;
            }
            let checkpoint = sync::CheckpointKey {
                namespace: identity.namespace,
                source_replica: head.stream,
                source_generation: head.generation,
                scope_id,
            };
            if let Some(progress) = self
                .endpoint
                .checkpoint(&checkpoint)
                .await
                .map_err(|error| SharePushError::Failed(error.to_string()))?
            {
                acknowledgements.insert(prefix.clone(), progress.sequence);
            }
        }

        let journal = self.endpoint.store();
        let through = SyncPeer::head(&self.endpoint)
            .await
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
        let retained = journal
            .progress_at(state.local_cursor)
            .await
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
        // A tail that compaction has already passed is not read: it goes out as a base.
        let tail_compacted = retained.is_none();
        let mut after = retained.unwrap_or_else(|| through.clone());
        let mut entries = Vec::<JournalEvent<RuneModel>>::new();
        while after.sequence < through.sequence {
            let page_read = journal
                .read_page_with_origins(sync::PageRequest {
                    source: identity.clone(),
                    scope: scope.clone(),
                    after: after.clone(),
                    through: through.clone(),
                    limits: sync::PageLimits {
                        max_scanned: sync::MAX_PAGE_ITEMS,
                        max_bytes: sync::MAX_PAGE_BYTES,
                    },
                })
                .await
                .map_err(|error| SharePushError::Failed(error.to_string()))?;
            let PageRead::Page(page) = page_read.read else {
                return Err(SharePushError::NeedsBase);
            };
            for (link, origin) in page.links.into_iter().zip(page_read.origins) {
                let event = match link.event {
                    Some(EventRef::Inline(event)) => event,
                    Some(EventRef::Fragmented(_)) | None => {
                        return Err(SharePushError::NeedsBase);
                    }
                };
                let origin = origin.origin.map(|origin| StoreIdentity {
                    namespace: identity.namespace,
                    model: identity.model.clone(),
                    replica: origin.replica,
                    generation: origin.generation,
                });
                entries.push(JournalEvent {
                    local_sequence: link.sequence,
                    event,
                    origin,
                });
            }
            after = page.next;
            if page.done {
                break;
            }
        }
        let cursor_status = {
            let tx = self
                .db
                .begin()
                .await
                .map_err(|error| SharePushError::Failed(error.to_string()))?;
            let cursor = read_on(&tx, identity.namespace, state.stream.as_bytes())
                .await
                .map_err(|error| SharePushError::Failed(error.to_string()))?;
            tx.commit()
                .await
                .map_err(|error| SharePushError::Failed(error.to_string()))?;
            if tail_compacted || cursor.is_some_and(|cursor| cursor.needs_base()) {
                ExportCursorStatus::NeedsBase
            } else {
                ExportCursorStatus::Current
            }
        };
        let now_ms = Utc::now().timestamp_millis().max(0) as u64;
        // The sizes of the base's compressed content and of its manifest steer the decisions to
        // cut. They are recorded with the manifest when the base is cut, and measured once for a
        // manifest from an earlier build; the cut and the collection of this pass confirm every
        // manifest they depend on before they write or delete anything.
        let stored = if let Some(base) = current_head.bases.last() {
            base_gzip_sizes::<RuneModel>(
                &*store,
                &mut state,
                &own_prefix,
                &base.base_id,
                capabilities,
                now_ms as i64,
            )
            .await?
        } else {
            None
        };
        let newest_base = match stored {
            Some(sizes) => BaseSize::Present {
                content_gzip_bytes: sizes.content,
                manifest_gzip_bytes: sizes.manifest,
                has_candidates: sizes.has_candidates,
            },
            None => BaseSize::Absent,
        };
        let activities = heads
            .iter()
            .map(|(prefix, head)| StreamActivity {
                stream_id: head.stream,
                last_head_change_ms: [0, 1]
                    .into_iter()
                    .filter_map(|slot| {
                        state
                            .observed_objects
                            .get(&ShareObjectKey::Head {
                                prefix: prefix.clone(),
                                slot,
                            })
                            .map(|observed| observed.last_change_ms.max(0) as u64)
                    })
                    .max()
                    .unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        let newest = current_head
            .bases
            .last()
            .map_or(NewestBase::Absent, |_base| NewestBase::Present {
                created_ms: current_head
                    .bases
                    .last()
                    .and_then(|base| {
                        state.observed_objects.get(&ShareObjectKey::Base {
                            prefix: own_prefix.clone(),
                            base_id: base.base_id.clone(),
                        })
                    })
                    .map(|observed| observed.first_seen_ms.max(0) as u64)
                    .unwrap_or(now_ms),
                content_gzip_bytes: match newest_base {
                    BaseSize::Present {
                        content_gzip_bytes, ..
                    } => content_gzip_bytes,
                    BaseSize::Absent => 0,
                },
                manifest_gzip_bytes: match newest_base {
                    BaseSize::Present {
                        manifest_gzip_bytes,
                        ..
                    } => manifest_gzip_bytes,
                    BaseSize::Absent => 0,
                },
                has_candidates: match newest_base {
                    BaseSize::Present { has_candidates, .. } => has_candidates,
                    BaseSize::Absent => false,
                },
            });
        let tail_bytes = current_head
            .segments
            .iter()
            .map(|segment| segment.bytes)
            .sum();
        let tail_has_candidates = entries.iter().any(|entry| match &entry.event {
            sync::Event::Key(delta) => !delta.added.is_empty(),
            sync::Event::Merge(delta) => !delta.added.is_empty(),
        });
        let keeper = plan_keeper(
            state.stream,
            now_ms,
            &activities,
            newest,
            tail_bytes,
            tail_has_candidates,
        );
        let bulk_cut = cursor_status == ExportCursorStatus::NeedsBase
            || matches!(
                plan_publish(PublishInput {
                    identity: &StoreIdentity {
                        namespace: identity.namespace,
                        model: identity.model.clone(),
                        replica: own_prefix.stream,
                        generation: own_prefix.generation,
                    },
                    after_cursor: state.local_cursor,
                    entries: &entries,
                    share_stream_ids: &share_stream_ids,
                    share_local_replicas: &share_local_replicas,
                    head: Checkpoint {
                        seq: current_head.head.seq,
                        anchor: current_head.head.anchor,
                    },
                    existing_segments: &current_head.segments,
                    newest_base,
                    export_cursor: cursor_status,
                    max_events_per_segment: 100,
                }),
                Ok(PublishDecision::CutBase { .. })
            );
        if bulk_cut || matches!(keeper, KeeperDecision::CutBase(_)) {
            let mut covers = BTreeMap::new();
            for (prefix, head) in &heads {
                let checkpoint = sync::CheckpointKey {
                    namespace: identity.namespace,
                    source_replica: head.stream,
                    source_generation: head.generation,
                    scope_id,
                };
                if let Some(progress) = self
                    .endpoint
                    .checkpoint(&checkpoint)
                    .await
                    .map_err(|error| SharePushError::Failed(error.to_string()))?
                {
                    covers.insert(
                        prefix.to_string(),
                        Checkpoint {
                            seq: progress.sequence,
                            anchor: progress.anchor,
                        },
                    );
                }
            }
            let head = cut_base(BaseCutRequest {
                db: &self.db,
                state: &mut state,
                writer: &self.share_writer,
                store: &*store,
                identity: &identity,
                source: &self.endpoint,
                current_head: Some(&current_head),
                covers,
                capabilities,
            })
            .await?;
            let head = collect_own_prefix(
                &self.db,
                &mut state,
                &self.share_writer,
                &*store,
                &head,
                &heads,
                capabilities,
            )
            .await?;
            return Ok(Some(head));
        }
        let published = publish_journal(PublishJournalRequest {
            db: &self.db,
            state: &mut state,
            writer: &self.share_writer,
            store: &*store,
            identity: &identity,
            current_head: &current_head,
            entries: &entries,
            acknowledgements: &acknowledgements,
            share_stream_ids: &share_stream_ids,
            share_local_replicas: &share_local_replicas,
            newest_base,
            cursor_status,
            capabilities,
            max_events_per_segment: 100,
        })
        .await?;
        let gc_head = collect_own_prefix(
            &self.db,
            &mut state,
            &self.share_writer,
            &*store,
            published.as_ref().unwrap_or(&current_head),
            &heads,
            capabilities,
        )
        .await?;
        Ok(published.or_else(|| (gc_head != current_head).then_some(gc_head)))
    }
    pub async fn identity(&self) -> Result<StoreIdentity> {
        Ok(ReceiveStore::identity(&self.endpoint).await?)
    }
    pub async fn head(&self) -> Result<sync::ReceiveProgress> {
        Ok(SyncPeer::head(&self.endpoint).await?)
    }
    pub async fn get(&self, key: &ObjectKey) -> Result<Option<Record>> {
        Ok(self.endpoint.get(key).await?)
    }
    pub async fn write_local(&self, key: ObjectKey, value: Option<Payload>) -> Result<Record> {
        Ok(self.endpoint.write_local(key, value).await?)
    }
    pub async fn pending_mixes(&self) -> Result<Vec<(Uuid, String)>> {
        Ok(collect(&self.db, &self.inner, [Collection::Mixes])
            .await?
            .into_iter()
            .map(|(key, reason)| (key.id, reason))
            .collect())
    }
    /// Explicit diagnostic; automatic synchronization does not traverse these pages.
    pub async fn pending_references(&self) -> Result<Vec<(ObjectKey, String)>> {
        collect(
            &self.db,
            &self.inner,
            <RuneModel as SyncModel>::collections(),
        )
        .await
    }
}
