//! Rune library composition and transactional receive.
use super::SeaOrmSyncStore;
use super::model::Collection;
use super::model::{ObjectKey, Payload, Record, RuneModel};
use super::references::collect;
use super::share::{
    BaseCutRequest, PublishJournalRequest, ShareCapabilities, SharePullError, SharePullRequest,
    SharePullResult, SharePushError, SharePushResult, ShareWriter, collect_own_prefix, cut_base,
    load_or_create_state, publish_journal, pull, push_store_error,
};
use crate::sync::{import, legacy};
use anyhow::{Result, ensure};
use sea_orm::{DatabaseConnection, TransactionTrait};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};
use sync::{
    EventRef, PageRead, ReceiveStore, StoreIdentity, SyncModel, SyncPeer, SyncStore,
    share::{
        BaseSize, Checkpoint, DevicePrefix, ExportCursorStatus, FormatError, Head, JournalEvent,
        KeeperDecision, NewestBase, ObjectKey as ShareObjectKey, ObjectStore, PublishDecision,
        PublishInput, StoreError, StreamActivity, base_key, decode_head, head_key, parse_key,
        plan_keeper, plan_publish,
    },
};
use sync_orm::{
    causal_capabilities::CausalEndpoint,
    causal_export_cursors::read_on,
    causal_schema::clear_namespace_on,
    causal_store::namespace_model_on,
    registry::{ProjectionBudget, RegistryProjection},
};
use uuid::Uuid;

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
            chrono::Utc::now().timestamp_millis(),
        )
        .await?;
        Ok(format!("{}.{}", state.stream, state.generation))
    }
    /// Classify the local journal tail after the share export cursor for timer selection.
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
            chrono::Utc::now().timestamp_millis(),
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
        let mut after = journal.progress_at(state.local_cursor).await?;
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
    /// Persist the writer counter before the owning library database is closed.
    pub async fn clean_close(&self) -> Result<()> {
        self.inner.clean_close().await
    }
    /// Explicitly traverse the registry; reads never initiate source import.
    async fn import_collections(&self, collections: &[Collection]) -> Result<()> {
        if sync_orm::causal_progress::initialization_complete_on(&self.db, self.inner.namespace())
            .await?
        {
            return Ok(());
        }
        for (index, collection) in collections.iter().enumerate() {
            let mut after = None;
            loop {
                let tx = self.db.begin().await?;
                let page = import::batch(&tx, &self.inner, collection, after).await?;
                if page.done && index + 1 == collections.len() {
                    sync_orm::causal_progress::complete_initialization_on(
                        &tx,
                        self.inner.namespace(),
                    )
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
        let mut existing = sync_orm::causal_store::list_namespace_ids_on(&db, 2).await?;
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
        let mut rebuild_encoding = false;
        if !existing.is_empty()
            && let Some(model) = namespace_model_on(&db, actual).await?
            && model.model == RuneModel::model_id()
            && model.schema == "1"
        {
            // Candidate digests bind the old JSON arrays; rebuild causal history from source rows.
            let tx = db.begin().await?;
            clear_namespace_on(&tx, actual).await?;
            tx.commit().await?;
            rebuild_encoding = true;
        }
        let inner = if existing.is_empty() || rebuild_encoding {
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
    pub async fn maintain(&self, state: &mut sync_orm::maintenance::Maintenance) -> Result<()> {
        state
            .advance::<RuneModel>(
                &self.db,
                self.identity().await?,
                chrono::Utc::now().timestamp_millis(),
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
        pull::<RuneModel>(SharePullRequest {
            db: &self.db,
            local: &self.endpoint,
            sink: &self.endpoint,
            local_source: &self.endpoint,
            store,
            library_id,
            scope: sync::Scope {
                collections: RuneModel::collections().into_iter().collect(),
            },
            limits: super::session_limits(),
            cache_path,
            capabilities,
        })
        .await
    }
    /// Pull every stream through Continuous, then publish this device's local journal tail.
    pub async fn push_share(
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
            chrono::Utc::now().timestamp_millis(),
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
        let mut after = journal
            .progress_at(state.local_cursor)
            .await
            .map_err(|_| SharePushError::NeedsBase)?;
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
            if cursor.is_some_and(|cursor| cursor.needs_base()) {
                ExportCursorStatus::NeedsBase
            } else {
                ExportCursorStatus::Current
            }
        };
        let newest_base = if let Some(base) = current_head.bases.last() {
            let key = base_key(&own_prefix.to_string(), &base.base_id)
                .map_err(|error| SharePushError::Failed(error.to_string()))?;
            match store.get(&key).await {
                Ok(object) => {
                    state.observe_tag(
                        ShareObjectKey::Base {
                            prefix: own_prefix.clone(),
                            base_id: base.base_id.clone(),
                        },
                        object.tag,
                        chrono::Utc::now().timestamp_millis(),
                    );
                    BaseSize::GzipBytes(object.bytes.len() as u64)
                }
                Err(StoreError::NotFound) => BaseSize::Absent,
                Err(error) => return Err(push_store_error(error)),
            }
        } else {
            BaseSize::Absent
        };
        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
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
                gzip_bytes: match newest_base {
                    BaseSize::GzipBytes(bytes) => bytes,
                    BaseSize::Absent => 0,
                },
            });
        let tail_bytes = current_head
            .segments
            .iter()
            .map(|segment| segment.bytes)
            .sum();
        let keeper = plan_keeper(state.stream, now_ms, &activities, newest, tail_bytes);
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
                    newest_base_gzip_bytes: newest_base,
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
