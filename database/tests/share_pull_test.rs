use database::test_fixtures::causal_library as support;

use anyhow::{Context, Result, bail, ensure};
use chrono::Utc;
use database::sync::model::{Collection, ObjectKey, RuneModel};
use database::sync::replica::PendingShareChange;
use database::sync::share::{
    BaseCutRequest, OwnContents, OwnRecord, PublishJournalRequest, ShareCapabilities,
    SharePullError, SharePushError, SharePushResult, ShareState, ShareWriter, cut_base,
    load_or_create_state, publish_journal, save_state,
};
use rand::{Rng, SeedableRng, distributions::Alphanumeric, rngs::StdRng};
use sea_orm::{
    ConnectionTrait, Database, ExprTrait, TransactionTrait,
    sea_query::{Alias, Expr, Query},
};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path as std_path;
use std::result;
use std::str::FromStr;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use support::SqlLibrary;
use sync::share::{
    BaseRef, Checkpoint, ConditionalGet, DevicePrefix, FloorBase, GcObservation, Head, LibraryFile,
    MemoryObjectStore, ObjectGcState, ObjectKey as ShareObjectKey, ObjectStore, ObjectTag,
    ObjectValue, Segment, SegmentLink, SegmentRef, StoreError, StoreObjectKind, StoreOperation,
    parse_key,
};
use sync::{self as p, SyncModel, SyncPeer, SyncStore, share};
use sync_orm::base_exporter::{BaseExportOptions, export_base};
use sync_orm::causal_capabilities::CausalEndpoint;
use sync_orm::causal_export_cursors;
use sync_orm::causal_export_cursors::EXPORT_CURSOR_MAX_AGE_MS;
use sync_orm::maintenance::Maintenance;
use tokio::sync::Notify;
use uuid::Uuid;

struct Published {
    endpoint: CausalEndpoint<RuneModel>,
    prefix: String,
    head: Head,
    bucket_chunks: Vec<String>,
    store: Arc<MemoryObjectStore>,
}

fn cache_path() -> std_path::PathBuf {
    env::temp_dir().join(format!("share-pull-{}", Uuid::new_v4()))
}

#[tokio::test]
async fn a_new_device_can_join_a_share_published_before_its_first_edit() -> Result<()> {
    let namespace = Uuid::new_v4();
    let owner = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    let publish = || {
        owner
            .sync_store
            .push_share(store.clone(), namespace, cache_path(), Default::default())
    };
    let empty = publish()
        .await?
        .context("empty share did not publish its head")?;
    let stream = p::StoreIdentity {
        replica: empty.stream,
        generation: empty.generation,
        ..owner.sync_store.identity().await?
    };
    let genesis = Checkpoint {
        seq: 0,
        anchor: p::initial_anchor(&stream)?,
    };
    assert_eq!(empty.floor, genesis);
    assert_eq!(empty.floor_base.covers_anchor, genesis.anchor);
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::new_v4(),
    };
    owner
        .sync_store
        .write_local(key.clone(), support::playlist("first edit"))
        .await?;
    let mut head = publish()
        .await?
        .context("first edit did not publish a head")?;
    assert_eq!(head.floor, genesis);
    assert_eq!(head.segments[0].prev_anchor, genesis.anchor);
    let join = || async {
        let joining = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        joining
            .sync_store
            .pull_share(store.clone(), namespace, cache_path(), Default::default())
            .await?;
        assert_eq!(
            joining
                .sync_store
                .get(&key)
                .await?
                .context("missing first edit")?
                .value,
            support::playlist("first edit")
        );
        anyhow::Ok(())
    };
    join().await?;

    // Recreate the base reference an older publisher left behind. Its first
    // segment is already chained to share genesis and does not need rewriting.
    let prefix = head.floor_base.owner_prefix.clone();
    let base = &mut head.bases[0];
    let base_key = share::base_key(&prefix, &base.base_id)?;
    let old_anchor = p::initial_anchor(&owner.sync_store.identity().await?)?;
    assert_ne!(old_anchor, genesis.anchor);
    let bytes = store.get(&base_key).await.unwrap().bytes;
    let mut manifest = share::decode_manifest::<RuneModel>(&bytes, &base.base_id)?;
    manifest.covers.get_mut(&prefix).unwrap().anchor = old_anchor;
    let (id, bytes) = share::encode_manifest(&manifest)?;
    store
        .put(&share::base_key(&prefix, &id)?, &bytes)
        .await
        .unwrap();
    base.covers = manifest.covers;
    base.base_id = id.clone();
    head.floor_base.base_id = id;
    head.floor_base.covers_anchor = old_anchor;
    head.counter += 1;
    store
        .put(&share::head_key(&prefix, 0)?, &share::encode_head(&head)?)
        .await
        .unwrap();
    join().await
}

async fn age_base_observation(
    source: &SqlLibrary,
    published: &Published,
    store: &MemoryObjectStore,
    age_ms: i64,
) -> Result<()> {
    source
        .sync_store
        .pull_share(
            published.store.clone(),
            source.sync_store.identity().await?.namespace,
            cache_path(),
            Default::default(),
        )
        .await?;
    let identity = source.sync_store.identity().await?;
    let now_ms = Utc::now().timestamp_millis();
    let mut state = load_or_create_state(
        &source.db,
        identity.replica,
        identity.namespace,
        cache_path(),
        now_ms,
    )
    .await?;
    let base = published
        .head
        .bases
        .last()
        .context("base reference missing")?;
    let key = share::base_key(&published.prefix, &base.base_id)?;
    let object = store
        .get(&key)
        .await
        .map_err(|error| anyhow::anyhow!("read base for age fixture: {error:?}"))?;
    state.observe_tag(
        ShareObjectKey::Base {
            prefix: DevicePrefix::from_str(&published.prefix)?,
            base_id: base.base_id.clone(),
        },
        object.tag,
        now_ms - age_ms,
    );
    save_state(&source.db, &state).await?;
    Ok(())
}

async fn publish_base(
    source: &SqlLibrary,
    store: Arc<MemoryObjectStore>,
    covers: BTreeMap<String, Checkpoint>,
) -> Result<Published> {
    let endpoint = source.sync_store.endpoint();
    let identity = SyncPeer::identity(&endpoint).await?;
    let mut state = load_or_create_state(
        &source.db,
        identity.replica,
        identity.namespace,
        cache_path(),
        Utc::now().timestamp_millis(),
    )
    .await?;
    let stream = state.stream;
    let generation = state.generation;
    let prefix = format!("{stream}.{generation}");
    let export = export_base(
        &endpoint,
        BaseExportOptions {
            own_prefix: prefix.clone(),
            covers,
            ..BaseExportOptions::default()
        },
    )
    .await?;
    let bucket_chunks = export
        .manifest
        .collections
        .values()
        .flat_map(|collection| collection.buckets.iter().map(|bucket| bucket.chunk.clone()))
        .collect();
    let base_id = export.base_id.clone();
    let library = LibraryFile {
        format: share::FORMAT_VERSION,
        library_id: identity.namespace,
        model: RuneModel::model_id().to_owned(),
        schema: RuneModel::schema_id().to_owned(),
    };
    store
        .put(&share::library_key(), &share::encode_library(&library)?)
        .await
        .map_err(|error| anyhow::anyhow!("put library.json: {error:?}"))?;
    for object in &export.objects {
        let key = if object.name == export.base_id {
            share::base_key(&prefix, &object.name)?
        } else {
            share::chunk_key(&prefix, &object.name)?
        };
        store
            .put(&key, &object.bytes)
            .await
            .map_err(|error| anyhow::anyhow!("put base object: {error:?}"))?;
    }
    let cover = export
        .manifest
        .covers
        .get(&prefix)
        .context("base export omitted owner cover")?
        .clone();
    let head = Head {
        format: share::FORMAT_VERSION,
        library_id: identity.namespace,
        stream,
        generation,
        local_replica: identity.replica,
        writer_session: state.writer_session,
        counter: 1,
        head: cover.clone(),
        floor: cover.clone(),
        floor_base: FloorBase {
            owner_prefix: prefix.clone(),
            base_id: base_id.clone(),
            covers_seq: cover.seq,
            covers_anchor: cover.anchor,
        },
        segments: Vec::new(),
        bases: vec![BaseRef {
            base_id: base_id.clone(),
            covers: export.manifest.covers.clone(),
        }],
        acks: Default::default(),
        local_cursor: cover.seq,
        retired: Vec::new(),
        updated_ms: 1,
    };
    store
        .put(&share::head_key(&prefix, 0)?, &share::encode_head(&head)?)
        .await
        .map_err(|error| anyhow::anyhow!("put initial head: {error:?}"))?;
    state.head_counter = head.counter;
    state.local_cursor = head.local_cursor;
    save_state(&source.db, &state).await?;
    Ok(Published {
        endpoint,
        prefix,
        head,
        bucket_chunks,
        store,
    })
}

async fn append_segment(published: &mut Published, id: u128, text: &str) -> Result<()> {
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(id),
    };
    published
        .endpoint
        .write_local(key, support::playlist(text))
        .await?;
    let identity = SyncPeer::identity(&published.endpoint).await?;
    let through = SyncPeer::head(&published.endpoint).await?;
    let scope = p::Scope {
        collections: RuneModel::collections().into_iter().collect(),
    };
    let page = SyncPeer::read_page(
        &published.endpoint,
        p::PageRequest {
            source: identity,
            scope,
            after: p::ReceiveProgress {
                sequence: published.head.head.seq,
                anchor: published.head.head.anchor,
            },
            through: through.clone(),
            limits: support::limits().pages,
        },
    )
    .await?;
    let p::PageRead::Page(page) = page else {
        anyhow::bail!("small share changes should fit in one segment page");
    };
    let mut anchor = published.head.head.anchor;
    let previous_anchor = anchor;
    let mut links = Vec::new();
    for link in page.links {
        let event = match link.event.context("segment link omitted event")? {
            p::EventRef::Inline(event) => event,
            p::EventRef::Fragmented(_) => anyhow::bail!("small share event was fragmented"),
        };
        anchor = p::next_anchor::<RuneModel>(
            anchor,
            link.sequence,
            &link.collection,
            link.event_digest,
        )?;
        links.push(SegmentLink {
            seq: link.sequence,
            collection: link.collection,
            event,
        });
    }
    ensure!(!links.is_empty(), "share segment has no links");
    ensure!(
        anchor == through.anchor,
        "segment anchor did not reach source head"
    );
    let first = links.first().context("missing first segment link")?.seq;
    let last = links.last().context("missing last segment link")?.seq;
    let segment = Segment {
        format: share::FORMAT_VERSION,
        prev_anchor: previous_anchor,
        links,
        last_anchor: anchor,
    };
    let bytes = share::encode_segment(&segment)?;
    let name = blake3::hash(&bytes).to_hex()[..16].to_owned();
    published
        .store
        .put(
            &share::segment_key(&published.prefix, first, last, &name)?,
            &bytes,
        )
        .await
        .map_err(|error| anyhow::anyhow!("put share segment: {error:?}"))?;
    let mut head = published.head.clone();
    head.counter += 1;
    head.head = Checkpoint {
        seq: through.sequence,
        anchor: through.anchor,
    };
    head.segments.push(SegmentRef {
        level: 0,
        first,
        last,
        prev_anchor: previous_anchor,
        last_anchor: anchor,
        name,
        bytes: bytes.len() as u64,
    });
    head.updated_ms += 1;
    published
        .store
        .put(
            &share::head_key(&published.prefix, (head.counter % 2) as u8)?,
            &share::encode_head(&head)?,
        )
        .await
        .map_err(|error| anyhow::anyhow!("put updated share head: {error:?}"))?;
    published.head = head;
    Ok(())
}

fn full_scope() -> p::Scope<RuneModel> {
    p::Scope {
        collections: RuneModel::collections().into_iter().collect(),
    }
}

struct FailAtRead {
    inner: Arc<MemoryObjectStore>,
    fail_at: usize,
    reads: AtomicUsize,
}

struct StaleConditionalPutOnce {
    inner: Arc<MemoryObjectStore>,
    calls: AtomicUsize,
}

impl ObjectStore for StaleConditionalPutOnce {
    fn get<'a>(&'a self, key: &'a str) -> p::BoxFuture<'a, ObjectValue, StoreError> {
        Box::pin(async move { self.inner.get(key).await })
    }

    fn get_if_none_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
    ) -> p::BoxFuture<'a, ConditionalGet, StoreError> {
        Box::pin(async move { self.inner.get_if_none_match(key, tag).await })
    }

    fn put<'a>(&'a self, key: &'a str, bytes: &'a [u8]) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move { self.inner.put(key, bytes).await })
    }

    fn create<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move { self.inner.create(key, bytes).await })
    }

    fn put_if_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
        bytes: &'a [u8],
    ) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(StoreError::PreconditionFailed);
            }
            self.inner.put_if_match(key, tag, bytes).await
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> p::BoxFuture<'a, (), StoreError> {
        Box::pin(async move { self.inner.delete(key).await })
    }

    fn list_prefix<'a>(&'a self, prefix: &'a str) -> p::BoxFuture<'a, Vec<String>, StoreError> {
        Box::pin(async move { self.inner.list_prefix(prefix).await })
    }
}

impl FailAtRead {
    fn should_fail(&self) -> bool {
        self.reads.fetch_add(1, Ordering::SeqCst) == self.fail_at
    }
}

impl ObjectStore for FailAtRead {
    fn get<'a>(&'a self, key: &'a str) -> p::BoxFuture<'a, ObjectValue, StoreError> {
        Box::pin(async move {
            if self.should_fail() {
                return Err(StoreError::Transient("injected read fault".into()));
            }
            self.inner.get(key).await
        })
    }

    fn get_if_none_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
    ) -> p::BoxFuture<'a, ConditionalGet, StoreError> {
        Box::pin(async move {
            if self.should_fail() {
                return Err(StoreError::Transient(
                    "injected conditional read fault".into(),
                ));
            }
            self.inner.get_if_none_match(key, tag).await
        })
    }

    fn put<'a>(&'a self, key: &'a str, bytes: &'a [u8]) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move { self.inner.put(key, bytes).await })
    }

    fn create<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move { self.inner.create(key, bytes).await })
    }

    fn put_if_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
        bytes: &'a [u8],
    ) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move { self.inner.put_if_match(key, tag, bytes).await })
    }

    fn delete<'a>(&'a self, key: &'a str) -> p::BoxFuture<'a, (), StoreError> {
        Box::pin(async move { self.inner.delete(key).await })
    }

    fn list_prefix<'a>(&'a self, prefix: &'a str) -> p::BoxFuture<'a, Vec<String>, StoreError> {
        Box::pin(async move {
            if self.should_fail() {
                return Err(StoreError::Transient("injected list fault".into()));
            }
            self.inner.list_prefix(prefix).await
        })
    }
}

async fn pull(published: &Published, target: &SqlLibrary) -> Result<()> {
    let store: Arc<dyn ObjectStore> = published.store.clone();
    target
        .sync_store
        .pull_share(
            store,
            target.sync_store.identity().await?.namespace,
            cache_path(),
            Default::default(),
        )
        .await
        .map_err(|error| anyhow::anyhow!("share pull: {error}"))?;
    Ok(())
}

async fn journal_events_since(
    endpoint: &CausalEndpoint<RuneModel>,
    after: &Head,
) -> Result<Vec<share::JournalEvent<RuneModel>>> {
    let identity = SyncPeer::identity(endpoint).await?;
    let through = SyncPeer::head(endpoint).await?;
    let journal = endpoint.store();
    let after = journal
        .progress_at(after.local_cursor)
        .await?
        .context("missing local journal cursor")?;
    let page_read = journal
        .read_page_with_origins(p::PageRequest {
            source: identity.clone(),
            scope: full_scope(),
            after,
            through: through.clone(),
            limits: support::limits().pages,
        })
        .await?;
    let p::PageRead::Page(page) = page_read.read else {
        anyhow::bail!("small share change unexpectedly required a fragment");
    };
    page.links
        .into_iter()
        .zip(page_read.origins)
        .map(|(link, origin)| {
            let event = match link.event.context("local journal link omitted event")? {
                p::EventRef::Inline(event) => event,
                p::EventRef::Fragmented(_) => {
                    anyhow::bail!("small event unexpectedly required a fragment")
                }
            };
            let origin = origin.origin.map(|origin| p::StoreIdentity {
                namespace: identity.namespace,
                model: identity.model.clone(),
                replica: origin.replica,
                generation: origin.generation,
            });
            Ok(share::JournalEvent {
                local_sequence: link.sequence,
                event,
                origin,
            })
        })
        .collect()
}

fn state_for(published: &Published, identity: &p::StoreIdentity) -> Result<ShareState> {
    let mut state = ShareState::new(identity.replica, identity.namespace, cache_path())?;
    state.generation = published.head.generation;
    state.writer_session = published.head.writer_session;
    state.head_counter = published.head.counter;
    state.local_cursor = published.head.local_cursor;
    Ok(state)
}

async fn export_cursor_sequence(
    source: &SqlLibrary,
    namespace: Uuid,
    stream: Uuid,
) -> Result<Option<u64>> {
    let tx = source.db.begin().await?;
    let cursor = causal_export_cursors::read_on(&tx, namespace, stream.as_bytes()).await?;
    tx.commit().await?;
    cursor
        .map(|cursor| u64::try_from(cursor.sequence))
        .transpose()
        .map_err(Into::into)
}

#[tokio::test]
async fn local_journal_pushes_a_segment_then_another_device_pulls_it() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    source
        .sync_store
        .write_local(
            ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(43),
            },
            support::playlist("included in the base"),
        )
        .await
        .map_err(|error| anyhow::anyhow!("put valid alternate head: {error:?}"))?;
    let store = Arc::new(MemoryObjectStore::default());
    let mut published = publish_base(&source, store.clone(), Default::default()).await?;
    source
        .sync_store
        .write_local(
            ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(44),
            },
            support::playlist("published by the local journal"),
        )
        .await?;
    let identity = SyncPeer::identity(&published.endpoint).await?;
    let events = journal_events_since(&published.endpoint, &published.head).await?;
    let mut state = state_for(&published, &identity)?;
    let writer = ShareWriter::default();
    store
        .put(
            &share::head_key(&published.prefix, 1)?,
            &share::encode_head(&published.head)?,
        )
        .await
        .map_err(|error| anyhow::anyhow!("seed alternate head slot: {error:?}"))?;
    store.inject_error(StoreOperation::Create, StoreError::PermissionDenied);
    assert_eq!(
        push_entries(
            &source,
            &mut state,
            &writer,
            &*store,
            &identity,
            &published.head,
            &events,
        )
        .await
        .unwrap_err(),
        SharePushError::ReadOnly
    );
    assert_eq!(state.local_cursor, published.head.local_cursor);
    assert_eq!(
        export_cursor_sequence(&source, namespace, state.stream).await?,
        None
    );
    assert_eq!(
        export_cursor_sequence(&source, namespace, state.stream).await?,
        None
    );
    assert!(
        store
            .list_prefix("devices/")
            .await
            .unwrap()
            .into_iter()
            .all(|key| !matches!(parse_key(&key), Ok(ShareObjectKey::Segment { .. })))
    );
    store.clear_error(StoreOperation::Create);
    store.inject_error(
        StoreOperation::ConditionalPut,
        StoreError::Transient("head write interrupted".into()),
    );
    assert!(
        push_entries(
            &source,
            &mut state,
            &writer,
            &*store,
            &identity,
            &published.head,
            &events
        )
        .await
        .is_err()
    );
    assert_eq!(state.local_cursor, published.head.local_cursor);
    let segment_keys = store
        .list_prefix("devices/")
        .await
        .unwrap()
        .into_iter()
        .filter(|key| matches!(parse_key(key), Ok(ShareObjectKey::Segment { .. })))
        .collect::<Vec<_>>();
    assert_eq!(segment_keys.len(), 1);
    assert_eq!(
        share::decode_head(
            &store
                .get(&share::head_key(&published.prefix, 0)?)
                .await
                .unwrap()
                .bytes
        )?,
        published.head
    );
    pull(&published, &target).await?;
    store.clear_error(StoreOperation::ConditionalPut);
    let retry_store = StaleConditionalPutOnce {
        inner: store.clone(),
        calls: AtomicUsize::new(0),
    };
    let result = push_entries(
        &source,
        &mut state,
        &writer,
        &retry_store,
        &identity,
        &published.head,
        &events,
    )
    .await?;
    assert_eq!(retry_store.calls.load(Ordering::SeqCst), 2);
    published.head = result.context("local journal had no publishable event")?;
    assert_eq!(
        export_cursor_sequence(&source, namespace, state.stream).await?,
        Some(published.head.local_cursor)
    );
    let tail = published
        .head
        .segments
        .last()
        .context("published segment missing")?;
    assert_eq!(tail.prev_anchor, published.head.floor.anchor);
    pull(&published, &target).await?;
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(44),
    };
    assert_eq!(
        target.sync_store.get(&key).await?,
        source.sync_store.get(&key).await?
    );
    Ok(())
}

#[tokio::test]
async fn push_pass_pulls_before_reading_and_publishing_the_local_tail() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let remote = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let mut rng = StdRng::seed_from_u64(761);
    source
        .sync_store
        .write_local(
            ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(147),
            },
            support::playlist(&random_name(&mut rng, 4096)),
        )
        .await?;
    let store = Arc::new(MemoryObjectStore::default());
    let published = publish_base(&source, store.clone(), Default::default()).await?;
    let remote_key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(149),
    };
    remote
        .sync_store
        .write_local(remote_key.clone(), support::playlist("remote stream"))
        .await?;
    let _published_remote = publish_base(&remote, store.clone(), Default::default()).await?;
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(148),
    };
    source
        .sync_store
        .write_local(key.clone(), support::playlist("push pass local tail"))
        .await?;
    let head = source
        .sync_store
        .push_share(
            store.clone(),
            namespace,
            cache_path(),
            ShareCapabilities {
                conditional_create: true,
                conditional_head_write: true,
                conditional_get: true,
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!("share push: {error}"))?
        .context("push pass did not publish the local journal")?;
    assert_eq!(head.head.seq, published.head.head.seq + 1);
    assert_eq!(
        source.sync_store.get(&remote_key).await?,
        remote.sync_store.get(&remote_key).await?
    );
    let target_result = target
        .sync_store
        .pull_share(store, namespace, cache_path(), Default::default())
        .await;
    target_result.map_err(|error| anyhow::anyhow!("target pull: {error}"))?;
    assert_eq!(
        target.sync_store.get(&key).await?,
        source.sync_store.get(&key).await?
    );
    assert_eq!(
        target.sync_store.get(&remote_key).await?,
        remote.sync_store.get(&remote_key).await?
    );
    Ok(())
}

#[tokio::test]
async fn stale_base_is_replaced_by_own_stream_cut() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    source
        .sync_store
        .write_local(
            ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(913),
            },
            support::playlist("base cut value"),
        )
        .await?;
    let mut published = publish_base(&source, store.clone(), Default::default()).await?;
    let unchanged_chunk_keys = published
        .bucket_chunks
        .iter()
        .map(|name| share::chunk_key(&published.prefix, name))
        .collect::<result::Result<Vec<_>, _>>()?;
    let creates_before = unchanged_chunk_keys
        .iter()
        .map(|key| store.create_count(key))
        .collect::<Vec<_>>();
    append_segment(&mut published, 920, "tail compacted by the new base").await?;
    let old_segment = published
        .head
        .segments
        .last()
        .context("segment for base-cut GC missing")?;
    let old_segment_key = share::segment_key(
        &published.prefix,
        old_segment.first,
        old_segment.last,
        &old_segment.name,
    )?;
    let old_base_key = share::base_key(&published.prefix, &published.head.bases[0].base_id)?;
    source
        .sync_store
        .pull_share(store.clone(), namespace, cache_path(), Default::default())
        .await?;
    let identity = source.sync_store.identity().await?;
    let mut state = load_or_create_state(
        &source.db,
        identity.replica,
        namespace,
        cache_path(),
        Utc::now().timestamp_millis(),
    )
    .await?;
    let base_key = share::base_key(&published.prefix, &published.head.bases[0].base_id)?;
    let base = store
        .get(&base_key)
        .await
        .map_err(|error| anyhow::anyhow!("read base: {error:?}"))?;
    let old_time = Utc::now().timestamp_millis() - share::BASE_MAX_AGE_MS as i64 - 1;
    state.observe_tag(
        ShareObjectKey::Base {
            prefix: DevicePrefix::from_str(&published.prefix)?,
            base_id: published.head.bases[0].base_id.clone(),
        },
        base.tag,
        old_time,
    );
    save_state(&source.db, &state).await?;

    let head = source
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), Default::default())
        .await?
        .context("stale base was not replaced")?;
    ensure!(
        head.bases.len() == published.head.bases.len() + 1,
        "base cut did not publish a new manifest"
    );
    ensure!(
        head.floor_base.base_id
            == head
                .bases
                .last()
                .context("missing replacement base")?
                .base_id,
        "head does not reference the new base"
    );
    ensure!(
        unchanged_chunk_keys
            .iter()
            .zip(creates_before)
            .all(|(key, before)| store.create_count(key) == before),
        "base cut fetched or rewrote unchanged chunks"
    );
    ensure!(
        matches!(store.get(&old_segment_key).await, Err(StoreError::NotFound)),
        "base cut retained a superseded segment with no active readers"
    );
    ensure!(
        store.get(&old_base_key).await.is_ok(),
        "GC removed a superseded base before its retention period"
    );
    let mut state = load_or_create_state(
        &source.db,
        source.sync_store.identity().await?.replica,
        namespace,
        cache_path(),
        Utc::now().timestamp_millis(),
    )
    .await?;
    let old_base = ShareObjectKey::Base {
        prefix: DevicePrefix::from_str(&published.prefix)?,
        base_id: published.head.bases[0].base_id.clone(),
    };
    let old_base_observation = state
        .observed_objects
        .get_mut(&old_base)
        .context("old base timer was not recorded")?;
    old_base_observation.gc_state = ObjectGcState::Superseded {
        since_ms: Utc::now().timestamp_millis() - share::SUPERSEDED_BASE_RETENTION_MS as i64 - 1,
    };
    save_state(&source.db, &state).await?;
    let tail_key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(914),
    };
    source
        .sync_store
        .write_local(tail_key.clone(), support::playlist("tail after base cut"))
        .await?;
    source
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), Default::default())
        .await?;
    ensure!(
        matches!(store.get(&old_base_key).await, Err(StoreError::NotFound)),
        "GC retained a superseded base after its retention deadline"
    );
    let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    target
        .sync_store
        .pull_share(store, namespace, cache_path(), Default::default())
        .await?;
    ensure!(
        target.sync_store.get(&tail_key).await? == source.sync_store.get(&tail_key).await?,
        "new device did not apply the base plus its tail"
    );
    Ok(())
}

#[tokio::test]
async fn first_share_push_creates_a_base_before_its_head() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(916),
    };
    source
        .sync_store
        .write_local(key.clone(), support::playlist("first share base"))
        .await?;

    let head = source
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), Default::default())
        .await?
        .context("first push did not create a base")?;
    ensure!(
        head.bases.len() == 1,
        "first share head does not reference its base"
    );
    target
        .sync_store
        .pull_share(store, namespace, cache_path(), Default::default())
        .await?;
    ensure!(
        target.sync_store.get(&key).await? == source.sync_store.get(&key).await?,
        "first base did not initialize a new device"
    );
    Ok(())
}

#[tokio::test]
async fn new_device_receives_every_record_of_a_multi_bucket_base() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    // Random 8 KiB names keep the compressed base large enough for eight key-space buckets.
    // The ids include the outermost keys and both keys at every bucket boundary.
    let mut ids = BTreeSet::from([1, u128::MAX]);
    for bucket in 1..8_u128 {
        ids.extend([(bucket << 125) - 1, bucket << 125]);
    }
    let mut rng = StdRng::seed_from_u64(577);
    while ids.len() < 1024 {
        ids.insert(rng.gen_range(1..=u128::MAX));
    }
    let mut keys = Vec::with_capacity(ids.len());
    for id in ids {
        let name: String = (&mut rng)
            .sample_iter(Alphanumeric)
            .take(8192)
            .map(char::from)
            .collect();
        let key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::from_u128(id),
        };
        source
            .sync_store
            .write_local(key.clone(), support::playlist(&name))
            .await?;
        keys.push(key);
    }
    let store = Arc::new(MemoryObjectStore::default());
    let head = source
        .sync_store
        .push_share(
            store.clone(),
            namespace,
            source.root.path().join("share-cache"),
            Default::default(),
        )
        .await?
        .context("first push did not publish a base")?;
    let base = head.bases.last().context("published head names no base")?;
    let prefix = format!("{}.{}", head.stream, head.generation);
    let object = store
        .get(&share::base_key(&prefix, &base.base_id)?)
        .await
        .map_err(|error| anyhow::anyhow!("read published base: {error:?}"))?;
    let manifest = share::decode_manifest::<RuneModel>(&object.bytes, &base.base_id)?;
    let buckets = manifest
        .collections
        .get("playlists")
        .context("published base omitted playlists")?
        .bucket_count;
    ensure!(
        buckets == 8,
        "the fixture published {buckets} playlist buckets instead of eight"
    );
    for attempt in 1..=2 {
        target
            .sync_store
            .pull_share(
                store.clone(),
                namespace,
                target.root.path().join("share-cache"),
                Default::default(),
            )
            .await
            .with_context(|| format!("pull {attempt} of the multi-bucket base"))?;
        for key in &keys {
            let received = target.sync_store.get(key).await?;
            ensure!(
                received.is_some() && received == source.sync_store.get(key).await?,
                "pull {attempt} did not deliver playlist {}",
                key.id
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn only_the_elected_keeper_cuts_a_retained_base() -> Result<()> {
    let namespace = Uuid::new_v4();
    let a = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let b = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    a.sync_store
        .write_local(
            ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(918),
            },
            support::playlist("keeper A"),
        )
        .await?;
    b.sync_store
        .write_local(
            ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(919),
            },
            support::playlist("keeper B"),
        )
        .await?;
    let published_a = publish_base(&a, store.clone(), Default::default()).await?;
    let published_b = publish_base(&b, store.clone(), Default::default()).await?;
    let old_base_age = 31 * 24 * 60 * 60 * 1000_i64;
    age_base_observation(&a, &published_a, &store, old_base_age).await?;
    age_base_observation(&b, &published_b, &store, old_base_age).await?;

    let (keeper, keeper_base, non_keeper, non_keeper_base) =
        if published_a.head.stream < published_b.head.stream {
            (&a, &published_a, &b, &published_b)
        } else {
            (&b, &published_b, &a, &published_a)
        };
    let non_keeper_head = non_keeper
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), Default::default())
        .await?;
    let non_keeper_head =
        non_keeper_head.context("non-keeper did not publish its acknowledgement")?;
    ensure!(
        non_keeper_head.bases.len() == non_keeper_base.head.bases.len(),
        "non-keeper cut the base"
    );
    let keeper_head = keeper
        .sync_store
        .push_share(store, namespace, cache_path(), Default::default())
        .await?
        .context("elected keeper did not cut the stale base")?;
    ensure!(
        keeper_head.bases.len() == keeper_base.head.bases.len() + 1,
        "elected keeper did not publish the replacement base"
    );
    Ok(())
}

#[tokio::test]
async fn interrupted_base_cut_leaves_orphans_for_delayed_own_gc() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    let foreign = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    foreign
        .sync_store
        .write_local(
            ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(920),
            },
            support::playlist("foreign prefix stays live"),
        )
        .await?;
    let foreign_published = publish_base(&foreign, store.clone(), Default::default()).await?;
    let foreign_objects = store
        .list_prefix(&format!("devices/{}/", foreign_published.prefix))
        .await
        .map_err(|error| anyhow::anyhow!("list foreign prefix before GC: {error:?}"))?;
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(917),
    };
    source
        .sync_store
        .write_local(key.clone(), support::playlist("before interrupted cut"))
        .await?;
    source
        .sync_store
        .pull_share(store.clone(), namespace, cache_path(), Default::default())
        .await?;
    let identity = source.sync_store.identity().await?;
    let state = load_or_create_state(
        &source.db,
        identity.replica,
        namespace,
        cache_path(),
        Utc::now().timestamp_millis(),
    )
    .await?;
    let prefix = format!("{}.{}", state.stream, state.generation);
    let export = export_base(
        &source.sync_store.endpoint(),
        BaseExportOptions {
            own_prefix: prefix.clone(),
            ..BaseExportOptions::default()
        },
    )
    .await?;
    let manifest_key = share::base_key(&prefix, &export.base_id)?;
    let mut orphan_keys = export
        .objects
        .iter()
        .filter(|object| object.name != export.base_id)
        .map(|object| share::chunk_key(&prefix, &object.name))
        .collect::<result::Result<Vec<_>, _>>()?;
    ensure!(
        !orphan_keys.is_empty(),
        "interrupted cut fixture has no chunks"
    );
    store.inject_error_for_kind(
        StoreOperation::Put,
        StoreObjectKind::Base,
        StoreError::Transient("injected manifest failure".into()),
    );
    let interrupted = source
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), Default::default())
        .await;
    if interrupted.is_ok() {
        let listed = store
            .list_prefix("devices/")
            .await
            .map_err(|error| anyhow::anyhow!("list successful cut: {error:?}"))?;
        anyhow::bail!(
            "manifest failure did not interrupt the base cut; expected={manifest_key}, observed={listed:?}"
        );
    }
    ensure!(
        store
            .list_prefix("devices/")
            .await
            .map_err(|error| anyhow::anyhow!("list after interrupted cut: {error:?}"))?
            .iter()
            .all(|object| {
                !matches!(parse_key(object), Ok(ShareObjectKey::Head { prefix: owner, .. }) if owner.to_string() == prefix)
            }),
        "interrupted base became reachable from a head"
    );
    for orphan in &orphan_keys {
        store
            .get(orphan)
            .await
            .map_err(|error| anyhow::anyhow!("partial base chunk missing: {error:?}"))?;
    }

    store.clear_error_for_kind(StoreOperation::Put, StoreObjectKind::Base);
    source
        .sync_store
        .write_local(key, support::playlist("replacement snapshot"))
        .await?;
    let replacement = source
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), Default::default())
        .await?
        .context("replacement base was not published")?;
    let replacement_key = share::base_key(
        &prefix,
        &replacement
            .bases
            .last()
            .context("replacement base reference missing")?
            .base_id,
    )?;
    let replacement_object = store
        .get(&replacement_key)
        .await
        .map_err(|error| anyhow::anyhow!("read replacement manifest: {error:?}"))?;
    let replacement_manifest = share::decode_manifest::<RuneModel>(
        &replacement_object.bytes,
        &replacement
            .bases
            .last()
            .context("replacement base reference missing")?
            .base_id,
    )?;
    let retained = replacement_manifest
        .collections
        .values()
        .flat_map(|collection| {
            collection
                .buckets
                .iter()
                .map(|bucket| bucket.chunk.clone())
                .chain(collection.context_fragments.iter().cloned())
        })
        .collect::<BTreeSet<_>>();
    orphan_keys.retain(|key| {
        matches!(parse_key(key), Ok(ShareObjectKey::Chunk { digest, .. }) if !retained.contains(&digest))
    });
    ensure!(
        !orphan_keys.is_empty(),
        "replacement base retained every interrupted chunk"
    );
    let mut state = load_or_create_state(
        &source.db,
        identity.replica,
        namespace,
        cache_path(),
        Utc::now().timestamp_millis(),
    )
    .await?;
    let old_time = Utc::now().timestamp_millis() - share::OBJECT_UNREFERENCED_MS as i64 - 10;
    let second_observation = Utc::now().timestamp_millis();
    for key in &orphan_keys {
        let ShareObjectKey::Chunk { prefix, digest } = parse_key(key)? else {
            anyhow::bail!("orphan chunk key did not parse as a chunk");
        };
        let observed = state
            .observed_objects
            .get_mut(&ShareObjectKey::Chunk { prefix, digest })
            .context("GC did not persist the first orphan observation")?;
        observed.gc_state = ObjectGcState::Unreferenced {
            first_seen_ms: old_time,
            second_observation: GcObservation::ObservedAt(second_observation as u64),
        };
    }
    save_state(&source.db, &state).await?;
    source
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), Default::default())
        .await?;
    for orphan in orphan_keys {
        ensure!(
            matches!(store.get(&orphan).await, Err(StoreError::NotFound)),
            "own-prefix GC retained an old interrupted-cut orphan"
        );
    }
    for object in foreign_objects {
        store.get(&object).await.map_err(|error| {
            anyhow::anyhow!("own-prefix GC touched a foreign object: {error:?}")
        })?;
    }
    Ok(())
}

/// A share in which the keeper has advertised the retirement of a device whose heads stayed
/// unchanged for the whole retirement period, so the keeper's next pass deletes its prefix.
/// The covered head is in slot 1 and the device's previous head in slot 0.
struct DueRetirement {
    namespace: Uuid,
    store: Arc<MemoryObjectStore>,
    keeper: SqlLibrary,
    retired_device: SqlLibrary,
    retired: Published,
    covered_key: ObjectKey,
}

/// Whether the retired device received the keeper base before the keeper advertised its
/// retirement.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KeeperBase {
    NotReceived,
    Received,
}

async fn due_retirement() -> Result<DueRetirement> {
    due_retirement_with(KeeperBase::NotReceived).await
}

async fn due_retirement_with(keeper_base: KeeperBase) -> Result<DueRetirement> {
    let namespace = Uuid::new_v4();
    let first = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let second = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    // The retired device's prefix sorts first. A new device's pull takes, among the bases that
    // cover a stream, the one with the largest total coverage, and on a tie the one whose owner
    // prefix sorts first, even the keeper base when it covers the retired stream below a newer
    // base of the retired device; that pull then cannot finish. This is a separate problem from
    // retirement cleanup, which the tests here are about.
    let (retired_device, keeper) = if first.sync_store.share_prefix(cache_path()).await?
        < second.sync_store.share_prefix(cache_path()).await?
    {
        (first, second)
    } else {
        (second, first)
    };
    let covered_key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(2201),
    };
    retired_device
        .sync_store
        .write_local(
            covered_key.clone(),
            support::playlist("covered by the keeper base"),
        )
        .await?;
    let store = Arc::new(MemoryObjectStore::default());
    let mut retired = publish_base(&retired_device, store.clone(), Default::default()).await?;
    // The device wrote its head once more before it went quiet, as an acknowledgement update
    // does, so both of its slots hold a head.
    retired.head.counter += 1;
    retired.head.updated_ms += 1;
    store
        .put(
            &share::head_key(&retired.prefix, 1)?,
            &share::encode_head(&retired.head)?,
        )
        .await
        .map_err(|error| anyhow::anyhow!("put the covered head: {error:?}"))?;
    if keeper_base == KeeperBase::Received {
        // The keeper cuts its base over the device's heads while they are recent, and the device
        // receives it before they age.
        keeper
            .sync_store
            .push_share(store.clone(), namespace, cache_path(), Default::default())
            .await?
            .context("keeper published no head")?;
        retired_device
            .sync_store
            .pull_share(store.clone(), namespace, cache_path(), Default::default())
            .await?;
    }
    age_retired_heads(&keeper, &store, &retired.prefix).await?;
    // This pass advertises the retirement, after it has cut the keeper base over the retired head
    // if the keeper had none yet. Deletion waits for a later pass. The pass returns the head it
    // published before its collection, so the advertising head is read from the slots.
    keeper
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), Default::default())
        .await?;
    let keeper_head =
        newest_head(&store, &keeper.sync_store.share_prefix(cache_path()).await?).await?;
    ensure!(
        keeper_head.retired == [retired.prefix.clone()],
        "keeper did not advertise the retirement: {:?}",
        keeper_head.retired
    );
    ensure!(
        !share_prefix_keys(&store, &retired.prefix).await?.is_empty(),
        "the prefix was deleted in the pass that advertised its retirement"
    );
    Ok(DueRetirement {
        namespace,
        store,
        keeper,
        retired_device,
        retired,
        covered_key,
    })
}

/// Makes the keeper see both heads of the retired prefix as first seen a whole retirement period
/// ago and unchanged since.
async fn age_retired_heads(
    keeper: &SqlLibrary,
    store: &MemoryObjectStore,
    retired_prefix: &str,
) -> Result<()> {
    let identity = keeper.sync_store.identity().await?;
    let now_ms = Utc::now().timestamp_millis();
    let mut state = load_or_create_state(
        &keeper.db,
        identity.replica,
        identity.namespace,
        cache_path(),
        now_ms,
    )
    .await?;
    for slot in [0, 1] {
        let head = store
            .get(&share::head_key(retired_prefix, slot)?)
            .await
            .map_err(|error| anyhow::anyhow!("read retired head: {error:?}"))?;
        let key = ShareObjectKey::Head {
            prefix: DevicePrefix::from_str(retired_prefix)?,
            slot,
        };
        state.observed_objects.remove(&key);
        state.observe_tag(key, head.tag, now_ms - share::RETIREMENT_MS as i64 - 60_000);
    }
    save_state(&keeper.db, &state).await?;
    Ok(())
}

/// The newest head in the slots of `prefix`.
async fn newest_head(store: &MemoryObjectStore, prefix: &str) -> Result<Head> {
    let mut newest: Option<Head> = None;
    for slot in [0, 1] {
        match store.get(&share::head_key(prefix, slot)?).await {
            Ok(value) => {
                let head = share::decode_head(&value.bytes)?;
                if newest
                    .as_ref()
                    .is_none_or(|known| head.counter > known.counter)
                {
                    newest = Some(head);
                }
            }
            Err(StoreError::NotFound) => (),
            Err(error) => anyhow::bail!("read a head of {prefix}: {error:?}"),
        }
    }
    newest.context("the prefix has no head")
}

/// Where the keeper's cleanup waits when it lists the prefix it retires.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RetiredListing {
    /// Before the listing, so the listing includes the change.
    Before,
    /// After the listing and before the head slots are read again.
    After,
}

/// Holds a cleanup pass at its listing of a retired prefix until another device has changed
/// that prefix.
struct RetiredListingGate {
    inner: Arc<MemoryObjectStore>,
    prefix: String,
    pause: RetiredListing,
    held: AtomicBool,
    reached: Notify,
    resume: Notify,
}

impl RetiredListingGate {
    async fn hold_at(&self, prefix: &str) {
        if prefix == self.prefix && !self.held.swap(true, Ordering::SeqCst) {
            self.reached.notify_one();
            self.resume.notified().await;
        }
    }
}

impl ObjectStore for RetiredListingGate {
    fn get<'a>(&'a self, key: &'a str) -> p::BoxFuture<'a, ObjectValue, StoreError> {
        self.inner.get(key)
    }

    fn get_if_none_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
    ) -> p::BoxFuture<'a, ConditionalGet, StoreError> {
        self.inner.get_if_none_match(key, tag)
    }

    fn put<'a>(&'a self, key: &'a str, bytes: &'a [u8]) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        self.inner.put(key, bytes)
    }

    fn create<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        self.inner.create(key, bytes)
    }

    fn put_if_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
        bytes: &'a [u8],
    ) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        self.inner.put_if_match(key, tag, bytes)
    }

    fn delete<'a>(&'a self, key: &'a str) -> p::BoxFuture<'a, (), StoreError> {
        self.inner.delete(key)
    }

    fn list_prefix<'a>(&'a self, prefix: &'a str) -> p::BoxFuture<'a, Vec<String>, StoreError> {
        Box::pin(async move {
            if self.pause == RetiredListing::Before {
                self.hold_at(prefix).await;
            }
            let listed = self.inner.list_prefix(prefix).await;
            if self.pause == RetiredListing::After {
                self.hold_at(prefix).await;
            }
            listed
        })
    }
}

/// Runs the keeper pass that deletes the retired prefix, with `change` running at that pass's
/// listing of the prefix as another device's store operations would.
async fn retire_with_change<T>(
    due: &DueRetirement,
    pause: RetiredListing,
    change: impl Future<Output = Result<T>>,
) -> Result<T> {
    let gate = Arc::new(RetiredListingGate {
        inner: due.store.clone(),
        prefix: format!("devices/{}/", due.retired.prefix),
        pause,
        held: AtomicBool::new(false),
        reached: Notify::new(),
        resume: Notify::new(),
    });
    let (cleanup, changed) = tokio::join!(
        async {
            let cleanup = due
                .keeper
                .sync_store
                .push_share(
                    gate.clone(),
                    due.namespace,
                    cache_path(),
                    Default::default(),
                )
                .await;
            // Ends the wait below when the pass never listed the retired prefix.
            gate.reached.notify_one();
            cleanup
        },
        async {
            gate.reached.notified().await;
            ensure!(
                gate.held.load(Ordering::SeqCst),
                "the keeper pass did not list the retired prefix"
            );
            let changed = change.await;
            gate.resume.notify_one();
            changed
        }
    );
    let changed = changed?;
    cleanup.map_err(|error| anyhow::anyhow!("keeper cleanup pass: {error}"))?;
    Ok(changed)
}

#[tokio::test]
async fn retirement_deletes_an_unchanged_prefix_covered_by_the_keeper_base() -> Result<()> {
    let due = due_retirement().await?;
    due.keeper
        .sync_store
        .push_share(
            due.store.clone(),
            due.namespace,
            cache_path(),
            Default::default(),
        )
        .await?;
    assert_eq!(
        share_prefix_keys(&due.store, &due.retired.prefix).await?,
        Vec::<String>::new()
    );
    // A device that joins later still gets the retired device's data from the keeper base.
    let joining = SqlLibrary::new(due.namespace, Uuid::new_v4()).await?;
    pull(&due.retired, &joining).await?;
    assert_eq!(
        joining.sync_store.get(&due.covered_key).await?,
        due.retired_device.sync_store.get(&due.covered_key).await?
    );
    Ok(())
}

/// The retired device's previous head, in the other slot, still names a base that the covered
/// head dropped and whose manifest the device has deleted, as its own collection does once the
/// head without the base is written. Nothing else is in the prefix, so it is still deleted.
#[tokio::test]
async fn retirement_deletes_a_prefix_whose_older_head_names_a_deleted_base() -> Result<()> {
    let due = due_retirement().await?;
    let key = share::head_key(&due.retired.prefix, 0)?;
    let mut previous = share::decode_head(
        &due.store
            .get(&key)
            .await
            .map_err(|error| anyhow::anyhow!("read the previous head: {error:?}"))?
            .bytes,
    )?;
    ensure!(
        previous.counter < due.retired.head.counter,
        "slot 0 does not hold the previous head"
    );
    let dropped = BaseRef {
        base_id: "0".repeat(64),
        covers: previous
            .bases
            .first()
            .context("the previous head names no base")?
            .covers
            .clone(),
    };
    previous.bases.insert(0, dropped);
    due.store
        .put(&key, &share::encode_head(&previous)?)
        .await
        .map_err(|error| anyhow::anyhow!("put the previous head: {error:?}"))?;
    // The rewritten slot counts as unchanged for the whole retirement period, like the other.
    age_retired_heads(&due.keeper, &due.store, &due.retired.prefix).await?;
    due.keeper
        .sync_store
        .push_share(
            due.store.clone(),
            due.namespace,
            cache_path(),
            Default::default(),
        )
        .await?;
    assert_eq!(
        share_prefix_keys(&due.store, &due.retired.prefix).await?,
        Vec::<String>::new(),
        "retirement cleanup kept a prefix whose older head names a deleted base"
    );
    let joining = SqlLibrary::new(due.namespace, Uuid::new_v4()).await?;
    pull(&due.retired, &joining).await?;
    assert_eq!(
        joining.sync_store.get(&due.covered_key).await?,
        due.retired_device.sync_store.get(&due.covered_key).await?
    );
    Ok(())
}

/// Runs a keeper cleanup pass with deletes of `kind` failing, so the pass stops after it has
/// deleted the objects listed before the first one of that kind, the covered head's manifest
/// first. Returns what the pass left in the retired prefix.
async fn fail_cleanup_at(due: &DueRetirement, kind: StoreObjectKind) -> Result<Vec<String>> {
    let before = share_prefix_keys(&due.store, &due.retired.prefix).await?;
    due.store.inject_error_for_kind(
        StoreOperation::Delete,
        kind,
        StoreError::Transient("delete failed".into()),
    );
    let failed = due
        .keeper
        .sync_store
        .push_share(
            due.store.clone(),
            due.namespace,
            cache_path(),
            Default::default(),
        )
        .await;
    due.store.clear_error_for_kind(StoreOperation::Delete, kind);
    ensure!(failed.is_err(), "the cleanup pass did not fail: {failed:?}");
    let left = share_prefix_keys(&due.store, &due.retired.prefix).await?;
    ensure!(
        !left.is_empty()
            && left.len() < before.len()
            && !left
                .iter()
                .any(|key| matches!(parse_key(key), Ok(ShareObjectKey::Base { .. }))),
        "the failed pass did not stop after deleting the covered manifest: {left:?}"
    );
    Ok(left)
}

/// A cleanup pass fails half way at a delete of `kind`. The next pass, with deletes working
/// again, has to delete everything that is left, and a new device still gets the retired
/// device's data from the keeper base. Returns what the failed pass left.
async fn retirement_finishes_after_a_failed_delete(kind: StoreObjectKind) -> Result<Vec<String>> {
    let due = due_retirement().await?;
    let left = fail_cleanup_at(&due, kind).await?;
    due.keeper
        .sync_store
        .push_share(
            due.store.clone(),
            due.namespace,
            cache_path(),
            Default::default(),
        )
        .await?;
    assert_eq!(
        share_prefix_keys(&due.store, &due.retired.prefix).await?,
        Vec::<String>::new(),
        "the retried cleanup left objects of the retired prefix"
    );
    let joining = SqlLibrary::new(due.namespace, Uuid::new_v4()).await?;
    pull(&due.retired, &joining).await?;
    assert_eq!(
        joining.sync_store.get(&due.covered_key).await?,
        due.retired_device.sync_store.get(&due.covered_key).await?
    );
    Ok(left)
}

/// The cleanup deleted the covered head's manifest and then failed to delete a head, so both
/// heads are still there when the next pass checks the prefix again.
#[tokio::test]
async fn retirement_finishes_a_deletion_that_failed_at_a_head() -> Result<()> {
    let left = retirement_finishes_after_a_failed_delete(StoreObjectKind::Head).await?;
    ensure!(
        left.iter()
            .any(|key| matches!(parse_key(key), Ok(ShareObjectKey::Head { .. }))),
        "the failed pass deleted the heads: {left:?}"
    );
    Ok(())
}

/// The cleanup deleted the manifest and both heads and then failed to delete a chunk, so the
/// prefix no longer has the heads that made it due for retirement.
#[tokio::test]
async fn retirement_finishes_a_deletion_that_failed_at_a_chunk() -> Result<()> {
    let left = retirement_finishes_after_a_failed_delete(StoreObjectKind::Chunk).await?;
    ensure!(
        !left
            .iter()
            .any(|key| matches!(parse_key(key), Ok(ShareObjectKey::Head { .. }))),
        "the failed pass left a head: {left:?}"
    );
    Ok(())
}

/// After a cleanup pass failed half way, the retired device writes a newer head into its prefix,
/// as an acknowledgement update does, with no new objects. The next pass keeps the prefix.
#[tokio::test]
async fn retirement_retry_keeps_a_prefix_with_a_newer_head() -> Result<()> {
    let due = due_retirement().await?;
    fail_cleanup_at(&due, StoreObjectKind::Head).await?;
    let mut newer = due.retired.head.clone();
    newer.counter += 1;
    newer.updated_ms += 1;
    due.store
        .put(
            &share::head_key(&due.retired.prefix, 0)?,
            &share::encode_head(&newer)?,
        )
        .await
        .map_err(|error| anyhow::anyhow!("put the newer head: {error:?}"))?;
    let left = share_prefix_keys(&due.store, &due.retired.prefix).await?;
    due.keeper
        .sync_store
        .push_share(
            due.store.clone(),
            due.namespace,
            cache_path(),
            Default::default(),
        )
        .await?;
    assert_eq!(
        share_prefix_keys(&due.store, &due.retired.prefix).await?,
        left,
        "the retried cleanup deleted a prefix with a newer head"
    );
    Ok(())
}

/// The manifest that the covered head names was already missing before any cleanup began, so
/// nothing shows what the listed chunks belong to, and the keeper keeps the whole prefix.
#[tokio::test]
async fn retirement_keeps_a_prefix_whose_covered_manifest_is_missing() -> Result<()> {
    let due = due_retirement().await?;
    let base = due
        .retired
        .head
        .bases
        .first()
        .context("the covered head names no base")?;
    due.store
        .delete(&share::base_key(&due.retired.prefix, &base.base_id)?)
        .await
        .map_err(|error| anyhow::anyhow!("delete the covered manifest: {error:?}"))?;
    let before = share_prefix_keys(&due.store, &due.retired.prefix).await?;
    due.keeper
        .sync_store
        .push_share(
            due.store.clone(),
            due.namespace,
            cache_path(),
            Default::default(),
        )
        .await?;
    assert_eq!(
        share_prefix_keys(&due.store, &due.retired.prefix).await?,
        before,
        "retirement cleanup deleted a prefix whose covered manifest was missing"
    );
    Ok(())
}

/// The retired device publishes to its prefix while the keeper deletes it, and the keeper
/// must leave the newer publication available.
async fn publication_during_retirement_stays_available(pause: RetiredListing) -> Result<()> {
    let due = due_retirement().await?;
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(2202),
    };
    due.retired_device
        .sync_store
        .write_local(
            key.clone(),
            support::playlist("published during retirement"),
        )
        .await?;
    let identity = SyncPeer::identity(&due.retired.endpoint).await?;
    let events = journal_events_since(&due.retired.endpoint, &due.retired.head).await?;
    let mut state = state_for(&due.retired, &identity)?;
    let writer = ShareWriter::default();
    // The device pulled before the keeper advertised the retirement, so it still publishes to
    // its old prefix: a segment the keeper base does not cover, then a newer head in slot 0,
    // next to the covered head in slot 1.
    let written = retire_with_change(&due, pause, async {
        let head = push_entries(
            &due.retired_device,
            &mut state,
            &writer,
            &*due.store,
            &identity,
            &due.retired.head,
            &events,
        )
        .await
        .map_err(|error| anyhow::anyhow!("publish during retirement: {error}"))?
        .context("the retired device published no head")?;
        let sibling = due
            .store
            .get(&share::head_key(&due.retired.prefix, 0)?)
            .await
            .map_err(|error| anyhow::anyhow!("read the newer head: {error:?}"))?;
        ensure!(
            share::decode_head(&sibling.bytes)? == head
                && head.head.seq > due.retired.head.head.seq,
            "the publication did not put a newer head in the other slot"
        );
        share_prefix_keys(&due.store, &due.retired.prefix).await
    })
    .await?;
    assert_eq!(
        share_prefix_keys(&due.store, &due.retired.prefix).await?,
        written,
        "retirement cleanup deleted objects of a newer publication"
    );
    let joining = SqlLibrary::new(due.namespace, Uuid::new_v4()).await?;
    pull(&due.retired, &joining).await?;
    assert_eq!(
        joining.sync_store.get(&key).await?,
        due.retired_device.sync_store.get(&key).await?
    );
    Ok(())
}

#[tokio::test]
async fn retirement_keeps_a_publication_that_lands_before_the_final_listing() -> Result<()> {
    publication_during_retirement_stays_available(RetiredListing::Before).await
}

#[tokio::test]
async fn retirement_keeps_a_publication_that_lands_after_the_final_listing() -> Result<()> {
    publication_during_retirement_stays_available(RetiredListing::After).await
}

#[tokio::test]
async fn retirement_keeps_a_prefix_whose_other_head_slot_is_partly_written() -> Result<()> {
    let due = due_retirement().await?;
    let mut next = due.retired.head.clone();
    next.counter += 1;
    let bytes = share::encode_head(&next)?;
    let sibling = share::head_key(&due.retired.prefix, 0)?;
    // The device is still writing a newer head over its previous one, on a store whose writes
    // are not atomic.
    let kept = retire_with_change(&due, RetiredListing::Before, async {
        due.store
            .put(&sibling, &bytes[..bytes.len() / 2])
            .await
            .map_err(|error| anyhow::anyhow!("write partial head: {error:?}"))?;
        share_prefix_keys(&due.store, &due.retired.prefix).await
    })
    .await?;
    assert_eq!(
        share_prefix_keys(&due.store, &due.retired.prefix).await?,
        kept
    );
    Ok(())
}

#[tokio::test]
async fn retirement_keeps_a_prefix_whose_covered_head_is_missing() -> Result<()> {
    let due = due_retirement().await?;
    let covered = share::head_key(&due.retired.prefix, 1)?;
    // Another device that retires the same prefix has already removed the covered head, and only
    // the older head is left.
    let kept = retire_with_change(&due, RetiredListing::Before, async {
        due.store
            .delete(&covered)
            .await
            .map_err(|error| anyhow::anyhow!("delete covered head: {error:?}"))?;
        share_prefix_keys(&due.store, &due.retired.prefix).await
    })
    .await?;
    assert_eq!(
        share_prefix_keys(&due.store, &due.retired.prefix).await?,
        kept
    );
    Ok(())
}

/// The retired device has uploaded the segment of a new publication but not yet written the head
/// that lists it, as `publish_journal` uploads segments first, when the keeper lists its prefix
/// for deletion. Both slots still hold the heads the retirement covers, so only the segment shows
/// the publication. The keeper keeps the whole prefix, and once the device has written its head,
/// a new device gets the publication.
#[tokio::test]
async fn retirement_keeps_a_segment_whose_head_is_not_written_yet() -> Result<()> {
    segment_whose_head_is_not_written_yet(false).await
}

/// Like `retirement_keeps_a_segment_whose_head_is_not_written_yet`, but a cleanup pass failed
/// half way before the retired device uploaded the segment, so the pass that finds the segment
/// would otherwise finish that deletion.
#[tokio::test]
async fn retirement_retry_keeps_a_segment_that_came_after_the_failed_pass() -> Result<()> {
    segment_whose_head_is_not_written_yet(true).await
}

async fn segment_whose_head_is_not_written_yet(after_failed_pass: bool) -> Result<()> {
    let due = due_retirement().await?;
    if after_failed_pass {
        fail_cleanup_at(&due, StoreObjectKind::Head).await?;
    }
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(2203),
    };
    due.retired_device
        .sync_store
        .write_local(key.clone(), support::playlist("uploaded before its head"))
        .await?;
    let identity = SyncPeer::identity(&due.retired.endpoint).await?;
    let events = journal_events_since(&due.retired.endpoint, &due.retired.head).await?;
    let writer = ShareWriter::default();
    // The device's head write does not get through, so its publication stops after the segment
    // upload and leaves both slots as the retirement saw them. Only heads are written with
    // `put_if_match`, which the memory store fails for every object kind at once; the segment
    // goes through `create`.
    let held_back = StoreError::Transient("head write held back".into());
    due.store
        .inject_error(StoreOperation::ConditionalPut, held_back.clone());
    for operation in [StoreOperation::Put, StoreOperation::Create] {
        due.store
            .inject_error_for_kind(operation, StoreObjectKind::Head, held_back.clone());
    }
    let mut state = state_for(&due.retired, &identity)?;
    let interrupted = push_entries(
        &due.retired_device,
        &mut state,
        &writer,
        &*due.store,
        &identity,
        &due.retired.head,
        &events,
    )
    .await;
    due.store.clear_error(StoreOperation::ConditionalPut);
    for operation in [StoreOperation::Put, StoreOperation::Create] {
        due.store
            .clear_error_for_kind(operation, StoreObjectKind::Head);
    }
    ensure!(
        interrupted.is_err(),
        "the publication wrote its head: {interrupted:?}"
    );
    let uploaded = share_prefix_keys(&due.store, &due.retired.prefix).await?;
    ensure!(
        uploaded.iter().any(|key| matches!(
            parse_key(key),
            Ok(ShareObjectKey::Segment { last, .. }) if last > due.retired.head.head.seq
        )),
        "the publication uploaded no segment past the covered head: {uploaded:?}"
    );

    due.keeper
        .sync_store
        .push_share(
            due.store.clone(),
            due.namespace,
            cache_path(),
            Default::default(),
        )
        .await?;
    assert_eq!(
        share_prefix_keys(&due.store, &due.retired.prefix).await?,
        uploaded,
        "retirement cleanup deleted an uncovered segment whose head had not been written"
    );

    let mut state = state_for(&due.retired, &identity)?;
    push_entries(
        &due.retired_device,
        &mut state,
        &writer,
        &*due.store,
        &identity,
        &due.retired.head,
        &events,
    )
    .await
    .map_err(|error| anyhow::anyhow!("write the head after the cleanup: {error}"))?
    .context("the retired device published no head")?;
    let joining = SqlLibrary::new(due.namespace, Uuid::new_v4()).await?;
    pull(&due.retired, &joining).await?;
    assert_eq!(
        joining.sync_store.get(&key).await?,
        due.retired_device.sync_store.get(&key).await?
    );
    Ok(())
}

/// Where a base cut of the retired device is held while the keeper cleans up.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CutStop {
    /// At its manifest upload, after its chunk uploads.
    BeforeManifest,
    /// At its head write, after its chunk and manifest uploads.
    BeforeHead,
}

/// Holds the retired device's first upload of a base manifest or a head, as `stop` says, until
/// the test lets it go on, as a slow upload would.
struct UploadGate {
    inner: Arc<MemoryObjectStore>,
    prefix: String,
    stop: CutStop,
    held: AtomicBool,
    reached: Notify,
    resume: Notify,
}

impl UploadGate {
    async fn hold_at(&self, key: &str) {
        let held = match parse_key(key) {
            Ok(ShareObjectKey::Base { prefix, .. }) => {
                self.stop == CutStop::BeforeManifest && prefix.to_string() == self.prefix
            }
            Ok(ShareObjectKey::Head { prefix, .. }) => {
                self.stop == CutStop::BeforeHead && prefix.to_string() == self.prefix
            }
            _ => false,
        };
        if held && !self.held.swap(true, Ordering::SeqCst) {
            self.reached.notify_one();
            self.resume.notified().await;
        }
    }
}

impl ObjectStore for UploadGate {
    fn get<'a>(&'a self, key: &'a str) -> p::BoxFuture<'a, ObjectValue, StoreError> {
        self.inner.get(key)
    }

    fn get_if_none_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
    ) -> p::BoxFuture<'a, ConditionalGet, StoreError> {
        self.inner.get_if_none_match(key, tag)
    }

    fn put<'a>(&'a self, key: &'a str, bytes: &'a [u8]) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            self.hold_at(key).await;
            self.inner.put(key, bytes).await
        })
    }

    fn create<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            self.hold_at(key).await;
            self.inner.create(key, bytes).await
        })
    }

    fn put_if_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
        bytes: &'a [u8],
    ) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move {
            self.hold_at(key).await;
            self.inner.put_if_match(key, tag, bytes).await
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> p::BoxFuture<'a, (), StoreError> {
        self.inner.delete(key)
    }

    fn list_prefix<'a>(&'a self, prefix: &'a str) -> p::BoxFuture<'a, Vec<String>, StoreError> {
        self.inner.list_prefix(prefix)
    }
}

/// Runs a base cut of the retired device over its covered head through `cut_base`, as its push
/// pass does, with `covers` for the other prefixes it has received, and holds the cut at `stop`.
/// While the cut is held, the keeper's cleanup pass runs and has to keep every object of the
/// prefix; then the cut goes on. Returns the objects that the cut had added when it was held and
/// the head that it wrote afterwards.
async fn cut_base_during_retirement(
    due: &DueRetirement,
    covers: BTreeMap<String, Checkpoint>,
    stop: CutStop,
) -> Result<(Vec<String>, Head)> {
    let before = share_prefix_keys(&due.store, &due.retired.prefix).await?;
    let gate = Arc::new(UploadGate {
        inner: due.store.clone(),
        prefix: due.retired.prefix.clone(),
        stop,
        held: AtomicBool::new(false),
        reached: Notify::new(),
        resume: Notify::new(),
    });
    let identity = SyncPeer::identity(&due.retired.endpoint).await?;
    let mut state = state_for(&due.retired, &identity)?;
    let writer = ShareWriter::default();
    let (cut, cleanup) = tokio::join!(
        async {
            let cut = cut_base(BaseCutRequest {
                db: &due.retired_device.db,
                state: &mut state,
                writer: &writer,
                store: &*gate,
                identity: &identity,
                source: &due.retired.endpoint,
                current_head: Some(&due.retired.head),
                covers,
                capabilities: Default::default(),
            })
            .await;
            // Ends the wait below when the cut never reached the held upload.
            gate.reached.notify_one();
            cut
        },
        async {
            gate.reached.notified().await;
            let cleanup = async {
                ensure!(
                    gate.held.load(Ordering::SeqCst),
                    "the base cut did not reach the held upload"
                );
                let uploaded = share_prefix_keys(&due.store, &due.retired.prefix).await?;
                due.keeper
                    .sync_store
                    .push_share(
                        due.store.clone(),
                        due.namespace,
                        cache_path(),
                        Default::default(),
                    )
                    .await
                    .map_err(|error| anyhow::anyhow!("keeper cleanup pass: {error}"))?;
                let kept = share_prefix_keys(&due.store, &due.retired.prefix).await?;
                Ok((uploaded, kept))
            }
            .await;
            gate.resume.notify_one();
            cleanup
        }
    );
    let (uploaded, kept) = cleanup?;
    assert_eq!(
        kept, uploaded,
        "retirement cleanup deleted an uncovered base upload whose head had not been written"
    );
    let head =
        cut.map_err(|error| anyhow::anyhow!("finish the base cut after the cleanup: {error}"))?;
    let added = uploaded
        .into_iter()
        .filter(|key| !before.contains(key))
        .collect();
    Ok((added, head))
}

/// The base manifests and the chunks among `keys`.
fn bases_and_chunks(keys: &[String]) -> (Vec<String>, Vec<String>) {
    let mut bases = Vec::new();
    let mut chunks = Vec::new();
    for key in keys {
        match parse_key(key) {
            Ok(ShareObjectKey::Base { .. }) => bases.push(key.clone()),
            Ok(ShareObjectKey::Chunk { .. }) => chunks.push(key.clone()),
            _ => (),
        }
    }
    (bases, chunks)
}

/// Checks that the head a held cut wrote names the base manifest that the cut had uploaded.
fn ensure_names_base(head: &Head, prefix: &str, manifests: &[String]) -> Result<()> {
    let newest = head.bases.last().context("the base cut named no base")?;
    ensure!(
        manifests == [share::base_key(prefix, &newest.base_id)?],
        "the base cut named another base than the one it had uploaded: {newest:?}"
    );
    Ok(())
}

/// The retired device has uploaded the chunks and the manifest of a base cut but not yet written
/// the head that names the base, as `cut_base` uploads them first, when the keeper lists its
/// prefix for deletion. Both slots still hold the heads the retirement covers, and the new base
/// holds a playlist that the keeper base does not. The keeper keeps the whole prefix, and once
/// the cut has written its head, a new device gets the playlist.
#[tokio::test]
async fn retirement_keeps_a_base_whose_head_is_not_written_yet() -> Result<()> {
    let due = due_retirement().await?;
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(2204),
    };
    due.retired_device
        .sync_store
        .write_local(key.clone(), support::playlist("in a base before its head"))
        .await?;
    let (added, head) =
        cut_base_during_retirement(&due, BTreeMap::new(), CutStop::BeforeHead).await?;
    let (manifests, chunks) = bases_and_chunks(&added);
    ensure!(
        manifests.len() == 1 && !chunks.is_empty(),
        "the held cut had not uploaded one manifest and new chunks: {added:?}"
    );
    ensure_names_base(&head, &due.retired.prefix, &manifests)?;
    let joining = SqlLibrary::new(due.namespace, Uuid::new_v4()).await?;
    pull(&due.retired, &joining).await?;
    assert_eq!(
        joining.sync_store.get(&key).await?,
        due.retired_device.sync_store.get(&key).await?
    );
    Ok(())
}

/// Like `retirement_keeps_a_base_whose_head_is_not_written_yet`, but the cut is held at its
/// manifest upload, so only new chunks that no listed manifest references show the cut.
#[tokio::test]
async fn retirement_keeps_chunks_whose_manifest_is_not_written_yet() -> Result<()> {
    let due = due_retirement().await?;
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(2205),
    };
    due.retired_device
        .sync_store
        .write_local(
            key.clone(),
            support::playlist("in chunks before their manifest"),
        )
        .await?;
    let before = share_prefix_keys(&due.store, &due.retired.prefix).await?;
    let (added, head) =
        cut_base_during_retirement(&due, BTreeMap::new(), CutStop::BeforeManifest).await?;
    let (manifests, chunks) = bases_and_chunks(&added);
    ensure!(
        manifests.is_empty() && !chunks.is_empty(),
        "the held cut had not uploaded new chunks alone: {added:?}"
    );
    // After the hold the cut uploaded its manifest and no more chunks, so its base uses the
    // chunks that the cleanup kept.
    let finished = share_prefix_keys(&due.store, &due.retired.prefix)
        .await?
        .into_iter()
        .filter(|key| !before.contains(key))
        .collect::<Vec<_>>();
    let (manifests, finished_chunks) = bases_and_chunks(&finished);
    ensure!(
        finished_chunks == chunks,
        "the cut uploaded chunks after the hold: {finished:?}"
    );
    ensure_names_base(&head, &due.retired.prefix, &manifests)?;
    let joining = SqlLibrary::new(due.namespace, Uuid::new_v4()).await?;
    pull(&due.retired, &joining).await?;
    assert_eq!(
        joining.sync_store.get(&key).await?,
        due.retired_device.sync_store.get(&key).await?
    );
    Ok(())
}

/// Like `retirement_keeps_a_base_whose_head_is_not_written_yet`, but the device's data has not
/// changed: it cuts a base over the keeper base it received before the retirement was advertised,
/// so the new manifest only references chunks that the listed manifest already references.
#[tokio::test]
async fn retirement_keeps_a_base_whose_chunks_are_all_listed() -> Result<()> {
    let due = due_retirement_with(KeeperBase::Received).await?;
    let keeper_prefix =
        DevicePrefix::from_str(&due.keeper.sync_store.share_prefix(cache_path()).await?)?;
    let received = p::ReceiveStore::checkpoint(
        &due.retired.endpoint,
        &p::CheckpointKey {
            namespace: due.namespace,
            source_replica: keeper_prefix.stream,
            source_generation: keeper_prefix.generation,
            scope_id: p::scope_id(&full_scope())?,
        },
    )
    .await?
    .context("the retired device did not receive the keeper base")?;
    let covers = BTreeMap::from([(
        keeper_prefix.to_string(),
        Checkpoint {
            seq: received.sequence,
            anchor: received.anchor,
        },
    )]);
    let (added, head) = cut_base_during_retirement(&due, covers, CutStop::BeforeHead).await?;
    let (manifests, chunks) = bases_and_chunks(&added);
    ensure!(
        manifests.len() == 1 && chunks.is_empty(),
        "the held cut had not uploaded a manifest alone: {added:?}"
    );
    ensure_names_base(&head, &due.retired.prefix, &manifests)?;
    let joining = SqlLibrary::new(due.namespace, Uuid::new_v4()).await?;
    pull(&due.retired, &joining).await?;
    assert_eq!(
        joining.sync_store.get(&due.covered_key).await?,
        due.retired_device.sync_store.get(&due.covered_key).await?
    );
    Ok(())
}

async fn push_entries(
    source: &SqlLibrary,
    state: &mut ShareState,
    writer: &ShareWriter,
    store: &dyn ObjectStore,
    identity: &p::StoreIdentity,
    head: &Head,
    entries: &[share::JournalEvent<RuneModel>],
) -> SharePushResult<Option<Head>> {
    let mut streams = BTreeSet::new();
    let mut local_replicas = BTreeSet::new();
    for key in store
        .list_prefix("devices/")
        .await
        .map_err(|error| SharePushError::Failed(format!("list share heads: {error:?}")))?
    {
        let ShareObjectKey::Head { prefix, slot } =
            parse_key(&key).map_err(|error| SharePushError::Failed(error.to_string()))?
        else {
            continue;
        };
        let key = share::head_key(&prefix.to_string(), slot)
            .map_err(|error| SharePushError::Failed(error.to_string()))?;
        if let Ok(value) = store.get(&key).await
            && let Ok(other_head) = share::decode_head(&value.bytes)
        {
            streams.insert(other_head.stream);
            local_replicas.insert(other_head.local_replica);
        }
    }
    streams.insert(head.stream);
    local_replicas.insert(identity.replica);
    publish_journal(PublishJournalRequest {
        db: &source.db,
        state,
        writer,
        store,
        identity,
        current_head: head,
        entries,
        acknowledgements: &BTreeMap::new(),
        share_stream_ids: &streams,
        share_local_replicas: &local_replicas,
        newest_base: share::BaseSize::Absent,
        cursor_status: share::ExportCursorStatus::Current,
        capabilities: ShareCapabilities {
            conditional_create: true,
            conditional_head_write: true,
            conditional_get: true,
        },
        max_events_per_segment: 100,
    })
    .await
}

#[tokio::test]
async fn concurrent_devices_publish_distinct_streams_and_a_third_device_converges() -> Result<()> {
    let namespace = Uuid::new_v4();
    let a = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let b = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let c = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    let mut published_a = publish_base(&a, store.clone(), Default::default()).await?;
    let mut published_b = publish_base(&b, store.clone(), Default::default()).await?;
    let key_a = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(140),
    };
    let key_b = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(141),
    };
    a.sync_store
        .write_local(key_a.clone(), support::playlist("authored on A"))
        .await?;
    b.sync_store
        .write_local(key_b.clone(), support::playlist("authored on B"))
        .await?;
    let identity_a = SyncPeer::identity(&published_a.endpoint).await?;
    let identity_b = SyncPeer::identity(&published_b.endpoint).await?;
    let entries_a = journal_events_since(&published_a.endpoint, &published_a.head).await?;
    let entries_b = journal_events_since(&published_b.endpoint, &published_b.head).await?;
    let mut state_a = state_for(&published_a, &identity_a)?;
    let mut state_b = state_for(&published_b, &identity_b)?;
    let writer_a = ShareWriter::default();
    let writer_b = ShareWriter::default();
    let (result_a, result_b) = tokio::join!(
        push_entries(
            &a,
            &mut state_a,
            &writer_a,
            &*store,
            &identity_a,
            &published_a.head,
            &entries_a,
        ),
        push_entries(
            &b,
            &mut state_b,
            &writer_b,
            &*store,
            &identity_b,
            &published_b.head,
            &entries_b,
        )
    );
    published_a.head = result_a?.context("A had no local event to publish")?;
    published_b.head = result_b?.context("B had no local event to publish")?;
    pull(&published_a, &c).await?;
    assert_eq!(
        c.sync_store.get(&key_a).await?,
        a.sync_store.get(&key_a).await?
    );
    assert_eq!(
        c.sync_store.get(&key_b).await?,
        b.sync_store.get(&key_b).await?
    );
    Ok(())
}

#[tokio::test]
async fn relayed_event_is_filtered_from_the_receiving_devices_own_stream() -> Result<()> {
    let namespace = Uuid::new_v4();
    let a = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let b = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    for (library, id) in [(&a, 144), (&b, 145)] {
        library
            .sync_store
            .write_local(
                ObjectKey {
                    collection: Collection::Playlists,
                    id: Uuid::from_u128(id),
                },
                support::playlist("base record"),
            )
            .await?;
    }
    let store = Arc::new(MemoryObjectStore::default());
    let mut published_a = publish_base(&a, store.clone(), Default::default()).await?;
    let published_b = publish_base(&b, store.clone(), Default::default()).await?;
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(142),
    };
    a.sync_store
        .write_local(key.clone(), support::playlist("relay once"))
        .await?;
    let identity_a = SyncPeer::identity(&published_a.endpoint).await?;
    let events_a = journal_events_since(&published_a.endpoint, &published_a.head).await?;
    let mut state_a = state_for(&published_a, &identity_a)?;
    let writer_a = ShareWriter::default();
    published_a.head = push_entries(
        &a,
        &mut state_a,
        &writer_a,
        &*store,
        &identity_a,
        &published_a.head,
        &events_a,
    )
    .await?
    .context("A had no local event to publish")?;
    pull(&published_a, &b).await?;

    let identity_b = SyncPeer::identity(&published_b.endpoint).await?;
    let relayed = journal_events_since(&published_b.endpoint, &published_b.head).await?;
    ensure!(
        !relayed.is_empty() && relayed.iter().all(|event| event.origin.is_some()),
        "unexpected relayed journal entries: {relayed:?}"
    );
    let mut state_b = state_for(&published_b, &identity_b)?;
    let writer_b = ShareWriter::default();
    let result_b = push_entries(
        &b,
        &mut state_b,
        &writer_b,
        &*store,
        &identity_b,
        &published_b.head,
        &relayed,
    )
    .await?;
    let head_b = result_b.context("ack-only publication should record the scanned cursor")?;
    assert_eq!(head_b.head.seq, published_b.head.head.seq);
    assert!(head_b.segments.is_empty());
    assert!(head_b.local_cursor > published_b.head.local_cursor);
    assert_eq!(
        export_cursor_sequence(&b, namespace, state_b.stream).await?,
        Some(head_b.local_cursor)
    );
    Ok(())
}

#[tokio::test]
async fn publication_survives_a_torn_alternate_head_slot() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    let mut published = publish_base(&source, store.clone(), Default::default()).await?;
    let next_slot = 1;
    store
        .put(
            &share::head_key(&published.prefix, 0)?,
            &share::encode_head(&published.head)?,
        )
        .await
        .map_err(|error| anyhow::anyhow!("put valid current head: {error:?}"))?;
    store
        .put(&share::head_key(&published.prefix, next_slot)?, b"torn")
        .await
        .map_err(|error| anyhow::anyhow!("corrupt alternate head: {error:?}"))?;
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(143),
    };
    source
        .sync_store
        .write_local(key, support::playlist("survive torn slot"))
        .await?;
    let identity = SyncPeer::identity(&published.endpoint).await?;
    let events = journal_events_since(&published.endpoint, &published.head).await?;
    let mut state = state_for(&published, &identity)?;
    let writer = ShareWriter::default();
    published.head = push_entries(
        &source,
        &mut state,
        &writer,
        &*store,
        &identity,
        &published.head,
        &events,
    )
    .await?
    .context("local journal had no publishable event")?;
    assert_eq!(published.head.counter, 2);
    assert_eq!(
        share::decode_head(
            &store
                .get(&share::head_key(&published.prefix, next_slot)?)
                .await
                .map_err(|error| anyhow::anyhow!("read repaired head: {error:?}"))?
                .bytes
        )?,
        published.head
    );
    Ok(())
}

#[tokio::test]
async fn changed_library_file_is_typed_and_never_recreated() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    source
        .sync_store
        .write_local(
            ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(146),
            },
            support::playlist("library binding baseline"),
        )
        .await?;
    let store = Arc::new(MemoryObjectStore::default());
    let published = publish_base(&source, store.clone(), Default::default()).await?;
    pull(&published, &target).await?;
    let changed = LibraryFile {
        format: share::FORMAT_VERSION,
        library_id: Uuid::new_v4(),
        model: RuneModel::model_id().to_owned(),
        schema: RuneModel::schema_id().to_owned(),
    };
    let changed_bytes = share::encode_library(&changed)?;
    store
        .put(&share::library_key(), &changed_bytes)
        .await
        .map_err(|error| anyhow::anyhow!("replace library.json: {error:?}"))?;
    let error = target
        .sync_store
        .pull_share(store.clone(), namespace, cache_path(), Default::default())
        .await
        .unwrap_err();
    assert_eq!(error, SharePullError::ShareChanged);
    assert_eq!(
        store
            .get(&share::library_key())
            .await
            .map_err(|error| anyhow::anyhow!("read changed library.json: {error:?}"))?
            .bytes,
        changed_bytes
    );
    Ok(())
}

async fn content_get_counts(store: &MemoryObjectStore) -> Result<BTreeMap<String, u64>> {
    let mut counts = BTreeMap::new();
    for key in store
        .list_prefix("devices/")
        .await
        .map_err(|error| anyhow::anyhow!("list share keys: {error:?}"))?
    {
        if matches!(
            parse_key(&key)?,
            ShareObjectKey::Base { .. }
                | ShareObjectKey::Segment { .. }
                | ShareObjectKey::Chunk { .. }
        ) {
            counts.insert(key.clone(), store.get_count(&key));
        }
    }
    Ok(counts)
}

async fn clear_checkpoint(target: &SqlLibrary, key: &p::CheckpointKey) -> Result<()> {
    let delete = Query::delete()
        .from_table(Alias::new("sync_checkpoints"))
        .and_where(Expr::col(Alias::new("namespace")).eq(key.namespace.as_bytes().to_vec()))
        .and_where(
            Expr::col(Alias::new("source_replica")).eq(key.source_replica.as_bytes().to_vec()),
        )
        .and_where(
            Expr::col(Alias::new("source_generation"))
                .eq(key.source_generation.as_bytes().to_vec()),
        )
        .and_where(Expr::col(Alias::new("scope_digest")).eq(key.scope_id.to_vec()))
        .to_owned();
    target.db.execute(&delete).await?;
    Ok(())
}

#[tokio::test]
async fn device_pulls_exported_base_segments_and_incremental_segment() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    source
        .sync_store
        .write_local(
            ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(1),
            },
            support::playlist("from base"),
        )
        .await?;
    let store = Arc::new(MemoryObjectStore::default());
    let mut published = publish_base(&source, store, Default::default()).await?;
    append_segment(&mut published, 2, "after base").await?;
    pull(&published, &target).await?;
    for id in [1, 2] {
        let key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::from_u128(id),
        };
        let actual = target
            .sync_store
            .get(&key)
            .await?
            .context("pulled playlist missing")?;
        assert_eq!(Some(actual), source.sync_store.get(&key).await?);
    }

    append_segment(&mut published, 3, "incremental").await?;
    pull(&published, &target).await?;
    let actual = target
        .sync_store
        .get(&ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::from_u128(3),
        })
        .await?
        .context("incremental playlist missing")?;
    assert_eq!(
        Some(actual),
        source
            .sync_store
            .get(&ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(3),
            })
            .await?
    );
    Ok(())
}

#[tokio::test]
async fn pulls_read_only_new_segments_and_skip_equal_base_bucket_chunks() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    source
        .sync_store
        .write_local(
            ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(100),
            },
            support::playlist("counted base"),
        )
        .await?;
    let store = Arc::new(MemoryObjectStore::default());
    let mut published = publish_base(&source, store.clone(), Default::default()).await?;
    pull(&published, &target).await?;

    let before_idle = content_get_counts(&store).await?;
    pull(&published, &target).await?;
    assert_eq!(
        content_get_counts(&store).await?,
        before_idle,
        "idle pull fetched share content"
    );

    let first_contact = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    pull(&published, &first_contact).await?;
    let checkpoint = p::CheckpointKey {
        namespace,
        source_replica: published.head.stream,
        source_generation: published.head.generation,
        scope_id: p::scope_id(&full_scope())?,
    };
    clear_checkpoint(&first_contact, &checkpoint).await?;
    let bucket_keys = published
        .bucket_chunks
        .iter()
        .map(|name| share::chunk_key(&published.prefix, name))
        .collect::<result::Result<Vec<_>, _>>()?;
    ensure!(!bucket_keys.is_empty(), "base fixture has no bucket chunks");
    let bucket_counts = bucket_keys
        .iter()
        .map(|key| (key.clone(), store.get_count(key)))
        .collect::<BTreeMap<_, _>>();

    let before_incremental = content_get_counts(&store).await?;
    append_segment(&mut published, 101, "counted incremental segment").await?;
    let new_segment = published
        .head
        .segments
        .last()
        .context("new segment missing")?;
    let new_segment_key = share::segment_key(
        &published.prefix,
        new_segment.first,
        new_segment.last,
        &new_segment.name,
    )?;
    let previous_content_count = before_incremental.len();
    pull(&published, &target).await?;
    let after_incremental = content_get_counts(&store).await?;
    for (key, count) in before_incremental {
        assert_eq!(
            after_incremental.get(&key),
            Some(&count),
            "incremental pull fetched old share object {key}"
        );
    }
    assert_eq!(
        after_incremental.len(),
        previous_content_count + 1,
        "incremental pull fetched more than the new segment"
    );
    assert_eq!(
        after_incremental.get(&new_segment_key),
        Some(&1),
        "incremental pull must fetch exactly its new segment"
    );

    pull(&published, &first_contact).await?;
    for (key, count) in bucket_counts {
        assert_eq!(
            store.get_count(&key),
            count,
            "first-contact reconciliation fetched equal bucket {key}"
        );
    }
    assert_eq!(store.get_count(&new_segment_key), 2);
    Ok(())
}

#[tokio::test]
async fn malformed_share_keys_and_covers_return_typed_errors() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    let published = publish_base(&source, store.clone(), Default::default()).await?;
    store
        .put("devices/not-a-prefix/head.0.json", b"malformed")
        .await
        .map_err(|error| anyhow::anyhow!("put malformed key: {error:?}"))?;
    let result = target
        .sync_store
        .pull_share(store.clone(), namespace, cache_path(), Default::default())
        .await;
    assert!(matches!(result, Err(SharePullError::Invalid(_))));

    store
        .delete("devices/not-a-prefix/head.0.json")
        .await
        .map_err(|error| anyhow::anyhow!("delete malformed key: {error:?}"))?;
    let head_key = share::head_key(&published.prefix, 0)?;
    let mut malformed_head = published.head.clone();
    malformed_head.bases[0].covers.insert(
        "malformed-prefix".into(),
        Checkpoint {
            seq: 0,
            anchor: [0; 32],
        },
    );
    store
        .put(&head_key, &share::encode_head(&malformed_head)?)
        .await
        .map_err(|error| anyhow::anyhow!("put malformed head cover: {error:?}"))?;
    let result = target
        .sync_store
        .pull_share(store, namespace, cache_path(), Default::default())
        .await;
    assert!(matches!(result, Err(SharePullError::Invalid(_))));
    Ok(())
}

async fn assert_target_consistent_at_checkpoint(
    target: &SqlLibrary,
    published: &Published,
    namespace: Uuid,
) -> Result<()> {
    let key = p::CheckpointKey {
        namespace,
        source_replica: published.head.stream,
        source_generation: published.head.generation,
        scope_id: p::scope_id(&full_scope())?,
    };
    let checkpoint = p::ReceiveStore::checkpoint(&target.sync_store.endpoint(), &key).await?;
    let base_key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(10),
    };
    let segment_key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(11),
    };
    let base_value = target.sync_store.get(&base_key).await?;
    let segment_value = target.sync_store.get(&segment_key).await?;
    match checkpoint {
        None => {
            assert!(base_value.is_none());
            assert!(segment_value.is_none());
        }
        Some(checkpoint) => {
            assert!(checkpoint.sequence <= published.head.head.seq);
            if checkpoint.sequence == published.head.floor.seq {
                assert_eq!(checkpoint.anchor, published.head.floor.anchor);
            } else if checkpoint.sequence == published.head.head.seq {
                assert_eq!(checkpoint.anchor, published.head.head.anchor);
            } else {
                anyhow::bail!("checkpoint is not at an applied atomic unit: {checkpoint:?}");
            }
            assert_eq!(
                base_value.map(|record| record.value),
                Some(support::playlist("failure source"))
            );
            if checkpoint.sequence == published.head.head.seq {
                assert_eq!(
                    segment_value.map(|record| record.value),
                    Some(support::playlist("failure segment"))
                );
            } else {
                assert!(segment_value.is_none());
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn transient_read_at_every_pull_step_preserves_consistency_and_retry_converges() -> Result<()>
{
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    source
        .sync_store
        .write_local(
            ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(10),
            },
            support::playlist("failure source"),
        )
        .await?;
    let store = Arc::new(MemoryObjectStore::default());
    let mut published = publish_base(&source, store.clone(), Default::default()).await?;
    append_segment(&mut published, 11, "failure segment").await?;

    let counter = Arc::new(FailAtRead {
        inner: store.clone(),
        fail_at: usize::MAX,
        reads: AtomicUsize::new(0),
    });
    let count_target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let counted: Arc<dyn ObjectStore> = counter.clone();
    count_target
        .sync_store
        .pull_share(counted, namespace, cache_path(), Default::default())
        .await
        .map_err(|error| anyhow::anyhow!("counting successful share pull: {error}"))?;
    let read_steps = counter.reads.load(Ordering::SeqCst);
    ensure!(read_steps > 0, "share pull made no store reads");

    for fail_at in 0..read_steps {
        let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        let failing = Arc::new(FailAtRead {
            inner: store.clone(),
            fail_at,
            reads: AtomicUsize::new(0),
        });
        let failing_store: Arc<dyn ObjectStore> = failing;
        let result = target
            .sync_store
            .pull_share(failing_store, namespace, cache_path(), Default::default())
            .await;
        assert!(
            matches!(result, Err(SharePullError::Transient(_))),
            "read step {fail_at} did not return a transient state: {result:?}"
        );
        assert_target_consistent_at_checkpoint(&target, &published, namespace).await?;

        pull(&published, &target).await?;
        for id in [10, 11] {
            let key = ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(id),
            };
            assert_eq!(
                target.sync_store.get(&key).await?,
                source.sync_store.get(&key).await?,
                "retry after read step {fail_at} did not converge for {id}"
            );
        }
        let checkpoint_key = p::CheckpointKey {
            namespace,
            source_replica: published.head.stream,
            source_generation: published.head.generation,
            scope_id: p::scope_id(&full_scope())?,
        };
        assert_eq!(
            p::ReceiveStore::checkpoint(&target.sync_store.endpoint(), &checkpoint_key).await?,
            Some(p::ReceiveProgress {
                sequence: published.head.head.seq,
                anchor: published.head.head.anchor,
            })
        );
    }
    Ok(())
}

#[tokio::test]
async fn deleted_floor_base_reconciles_from_another_live_base() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    source
        .sync_store
        .write_local(
            ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(20),
            },
            support::playlist("covered by first base"),
        )
        .await?;
    let store = Arc::new(MemoryObjectStore::default());
    let mut published = publish_base(&source, store.clone(), Default::default()).await?;
    pull(&published, &target).await?;
    append_segment(&mut published, 21, "covered by replacement base").await?;

    let floor_export = export_base(
        &published.endpoint,
        BaseExportOptions {
            own_prefix: published.prefix.clone(),
            covers: BTreeMap::from([(published.prefix.clone(), published.head.head.clone())]),
            ..BaseExportOptions::default()
        },
    )
    .await?;
    for object in &floor_export.objects {
        let key = if object.name == floor_export.base_id {
            share::base_key(&published.prefix, &object.name)?
        } else {
            share::chunk_key(&published.prefix, &object.name)?
        };
        store
            .put(&key, &object.bytes)
            .await
            .map_err(|error| anyhow::anyhow!("put replacement floor object: {error:?}"))?;
    }
    let floor_cover = floor_export
        .manifest
        .covers
        .get(&published.prefix)
        .context("replacement floor does not cover its stream")?
        .clone();
    let mut advanced_head = published.head.clone();
    advanced_head.counter += 1;
    advanced_head.floor = advanced_head.head.clone();
    advanced_head.floor_base = FloorBase {
        owner_prefix: published.prefix.clone(),
        base_id: floor_export.base_id.clone(),
        covers_seq: floor_cover.seq,
        covers_anchor: floor_cover.anchor,
    };
    advanced_head.bases.push(BaseRef {
        base_id: floor_export.base_id.clone(),
        covers: floor_export.manifest.covers.clone(),
    });
    advanced_head.updated_ms += 1;
    store
        .put(
            &share::head_key(&published.prefix, (advanced_head.counter % 2) as u8)?,
            &share::encode_head(&advanced_head)?,
        )
        .await
        .map_err(|error| anyhow::anyhow!("publish advanced floor: {error:?}"))?;
    let missing_floor = share::base_key(&published.prefix, &floor_export.base_id)?;
    store
        .delete(&missing_floor)
        .await
        .map_err(|error| anyhow::anyhow!("delete floor base: {error:?}"))?;
    published.head = advanced_head;

    let fallback_source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    support::exchange_scoped(&source.peer, &fallback_source.peer, support::scope()).await?;
    let covers = BTreeMap::from([(published.prefix.clone(), published.head.head.clone())]);
    let fallback = publish_base(&fallback_source, store, covers).await?;
    ensure!(
        fallback.prefix != published.prefix,
        "fallback base shares the old prefix"
    );

    let before = target
        .sync_store
        .get(&ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::from_u128(21),
        })
        .await?;
    assert!(before.is_none(), "target unexpectedly has post-floor data");
    pull(&published, &target).await?;
    let recovered = target
        .sync_store
        .get(&ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::from_u128(21),
        })
        .await?
        .context("target did not reconcile from the live fallback base")?;
    assert_eq!(
        recovered.value,
        support::playlist("covered by replacement base")
    );
    let checkpoint = p::CheckpointKey {
        namespace,
        source_replica: published.head.stream,
        source_generation: published.head.generation,
        scope_id: p::scope_id(&full_scope())?,
    };
    assert_eq!(
        p::ReceiveStore::checkpoint(&target.sync_store.endpoint(), &checkpoint).await?,
        Some(p::ReceiveProgress {
            sequence: published.head.head.seq,
            anchor: published.head.head.anchor,
        })
    );
    Ok(())
}

enum OtherBase {
    TiedBeforeSource,
    TiedAfterSource,
    GreaterCoverage,
}

async fn share_with_newer_floor_base(
    other_base: OtherBase,
) -> Result<(Published, SqlLibrary, ObjectKey)> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let other = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let source_prefix = source.sync_store.share_prefix(cache_path()).await?;
    let other_prefix = other.sync_store.share_prefix(cache_path()).await?;
    let (source, other) =
        if (other_prefix < source_prefix) == matches!(other_base, OtherBase::TiedBeforeSource) {
            (source, other)
        } else {
            (other, source)
        };

    let first = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(2291),
    };
    source
        .sync_store
        .write_local(first, support::playlist("first"))
        .await?;
    let store = Arc::new(MemoryObjectStore::default());
    let mut published = publish_base(&source, store.clone(), BTreeMap::new()).await?;
    if matches!(other_base, OtherBase::GreaterCoverage) {
        other
            .sync_store
            .write_local(
                ObjectKey {
                    collection: Collection::Playlists,
                    id: Uuid::from_u128(2293),
                },
                support::playlist("other device's playlist"),
            )
            .await?;
    }
    let other_head = other
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), Default::default())
        .await?
        .context("other device did not publish a base")?;
    let competing = other_head
        .bases
        .iter()
        .find(|base| base.covers.get(&published.prefix) == Some(&published.head.head))
        .context("other base did not cover the source's first checkpoint")?;

    let second = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(2292),
    };
    source
        .sync_store
        .write_local(second.clone(), support::playlist("second"))
        .await?;
    let identity = SyncPeer::identity(&published.endpoint).await?;
    let mut state = state_for(&published, &identity)?;
    published.head = cut_base(BaseCutRequest {
        db: &source.db,
        state: &mut state,
        writer: &ShareWriter::default(),
        store: &*store,
        identity: &identity,
        source: &published.endpoint,
        current_head: Some(&published.head),
        covers: BTreeMap::new(),
        capabilities: Default::default(),
    })
    .await?;
    let floor_base = published
        .head
        .bases
        .iter()
        .find(|base| base.base_id == published.head.floor_base.base_id)
        .context("missing floor base reference")?;
    let total = |base: &BaseRef| base.covers.values().map(|cover| cover.seq).sum::<u64>();
    match other_base {
        OtherBase::GreaterCoverage => assert!(total(competing) > total(floor_base)),
        _ => assert_eq!(total(competing), total(floor_base)),
    }
    Ok((published, other, second))
}

async fn assert_join_receives_second(published: &Published, second: &ObjectKey) -> Result<()> {
    let joining = SqlLibrary::new(published.head.library_id, Uuid::new_v4()).await?;
    pull(published, &joining).await?;
    let received = joining
        .sync_store
        .get(second)
        .await?
        .context("joining device missed the second playlist")?;
    assert_eq!(received.value, support::playlist("second"));
    Ok(())
}

#[tokio::test]
async fn a_new_device_joins_after_a_newer_base_whose_device_sorts_after_the_other() -> Result<()> {
    let (published, _, second) = share_with_newer_floor_base(OtherBase::TiedBeforeSource).await?;
    assert_join_receives_second(&published, &second).await
}

#[tokio::test]
async fn a_new_device_joins_after_a_newer_base_whose_device_sorts_before_the_other() -> Result<()> {
    let (published, _, second) = share_with_newer_floor_base(OtherBase::TiedAfterSource).await?;
    assert_join_receives_second(&published, &second).await
}

#[tokio::test]
async fn stale_base_with_greater_total_coverage_does_not_displace_the_floor_base() -> Result<()> {
    let (published, _, second) = share_with_newer_floor_base(OtherBase::GreaterCoverage).await?;
    assert_join_receives_second(&published, &second).await
}

#[tokio::test]
async fn base_at_floor_sequence_with_a_different_anchor_is_skipped() -> Result<()> {
    let (mut published, other, second) =
        share_with_newer_floor_base(OtherBase::GreaterCoverage).await?;
    append_segment(&mut published, 2294, "after the floor").await?;
    let mut divergent = published.head.floor.clone();
    divergent.anchor[0] ^= 1;
    publish_base(
        &other,
        published.store.clone(),
        BTreeMap::from([(published.prefix.clone(), divergent)]),
    )
    .await?;
    assert_join_receives_second(&published, &second).await
}

#[tokio::test]
async fn missing_floor_base_with_only_stale_alternatives_reports_no_live_base() -> Result<()> {
    let (published, _, _) = share_with_newer_floor_base(OtherBase::GreaterCoverage).await?;
    let floor_key = share::base_key(&published.prefix, &published.head.floor_base.base_id)?;
    published
        .store
        .delete(&floor_key)
        .await
        .map_err(|error| anyhow::anyhow!("remove floor base: {error:?}"))?;
    let namespace = published.head.library_id;
    let joining = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let error = joining
        .sync_store
        .pull_share(published.store, namespace, cache_path(), Default::default())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        SharePullError::Invalid("no live base covers the required checkpoint".into())
    );
    Ok(())
}

async fn base_manifest(
    store: &MemoryObjectStore,
    prefix: &str,
    base_id: &str,
) -> Result<share::BaseManifest<RuneModel>> {
    let object = store
        .get(&share::base_key(prefix, base_id)?)
        .await
        .map_err(|error| anyhow::anyhow!("read base manifest: {error:?}"))?;
    Ok(share::decode_manifest(&object.bytes, base_id)?)
}

fn playlist_summary(
    manifest: &share::BaseManifest<RuneModel>,
) -> Result<&share::CollectionSummary<RuneModel>> {
    manifest
        .collections
        .get(RuneModel::collection_tag(&Collection::Playlists))
        .context("base has no playlist collection")
}

#[tokio::test]
async fn restarted_receiver_reuses_its_base_after_a_missing_context_chunk_returns() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let playlist = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(50),
    };
    source
        .sync_store
        .write_local(
            playlist.clone(),
            support::playlist("behind a missing chunk"),
        )
        .await?;
    let store = Arc::new(MemoryObjectStore::default());
    let published = publish_base(&source, store.clone(), Default::default()).await?;
    let base = base_manifest(
        &store,
        &published.prefix,
        &published.head.floor_base.base_id,
    )
    .await?;
    let chunk = share::chunk_key(
        &published.prefix,
        playlist_summary(&base)?
            .context_fragments
            .first()
            .context("playlist context has no chunk")?,
    )?;

    let directory = tempfile::tempdir()?;
    let database = directory.path().join("receiver.sqlite");
    let cache = directory.path().join("share-cache");
    let actor = Uuid::new_v4();
    let mut db = support::fixtures::disk_database(&database, actor).await?;
    let mut receiver = support::fixtures::open_library(db.clone(), actor, Some(namespace)).await?;
    // The share answers not-found for the chunk while its service recovers.
    let bytes = store
        .get(&chunk)
        .await
        .map_err(|error| anyhow::anyhow!("read context chunk: {error:?}"))?
        .bytes;
    store
        .delete(&chunk)
        .await
        .map_err(|error| anyhow::anyhow!("hide context chunk: {error:?}"))?;
    let result = receiver
        .pull_share(store.clone(), namespace, cache.clone(), Default::default())
        .await;
    assert!(
        matches!(result, Err(SharePullError::MissingObject(_))),
        "a missing context chunk was not reported as missing: {result:?}"
    );
    assert!(receiver.get(&playlist).await?.is_none());

    // The same chunk is back. Each retry starts the same receiver database cold.
    store
        .put(&chunk, &bytes)
        .await
        .map_err(|error| anyhow::anyhow!("restore context chunk: {error:?}"))?;
    let reads = store.get_count(&chunk);
    for restart in 1..=2 {
        receiver.clean_close().await?;
        drop(receiver);
        db.close().await?;
        db = Database::connect(format!("sqlite://{}", database.display())).await?;
        receiver = support::fixtures::open_library(db.clone(), actor, Some(namespace)).await?;
        receiver
            .pull_share(store.clone(), namespace, cache.clone(), Default::default())
            .await
            .map_err(|error| anyhow::anyhow!("share pull after restart {restart}: {error}"))?;
    }
    assert!(
        store.get_count(&chunk) > reads,
        "the receiver did not read the context chunk of its base again"
    );
    assert_eq!(
        receiver.get(&playlist).await?,
        source.sync_store.get(&playlist).await?
    );
    let checkpoint = p::CheckpointKey {
        namespace,
        source_replica: published.head.stream,
        source_generation: published.head.generation,
        scope_id: p::scope_id(&full_scope())?,
    };
    assert_eq!(
        p::ReceiveStore::checkpoint(&receiver.endpoint(), &checkpoint).await?,
        Some(p::ReceiveProgress {
            sequence: published.head.head.seq,
            anchor: published.head.head.anchor,
        })
    );
    Ok(())
}

/// Exports the stream's current state as another base of its own and lists it in the head
/// without moving the floor. It covers more of the stream than the floor base, so a receiver
/// tries it first.
async fn publish_newer_base(
    published: &mut Published,
) -> Result<(String, share::BaseManifest<RuneModel>)> {
    let newer = export_base(
        &published.endpoint,
        BaseExportOptions {
            own_prefix: published.prefix.clone(),
            ..BaseExportOptions::default()
        },
    )
    .await?;
    for object in &newer.objects {
        let key = if object.name == newer.base_id {
            share::base_key(&published.prefix, &object.name)?
        } else {
            share::chunk_key(&published.prefix, &object.name)?
        };
        published
            .store
            .put(&key, &object.bytes)
            .await
            .map_err(|error| anyhow::anyhow!("put newer base object: {error:?}"))?;
    }
    published.head.counter += 1;
    published.head.bases.push(BaseRef {
        base_id: newer.base_id.clone(),
        covers: newer.manifest.covers.clone(),
    });
    published.head.updated_ms += 1;
    published
        .store
        .put(
            &share::head_key(&published.prefix, (published.head.counter % 2) as u8)?,
            &share::encode_head(&published.head)?,
        )
        .await
        .map_err(|error| anyhow::anyhow!("publish newer base: {error:?}"))?;
    Ok((newer.base_id, newer.manifest))
}

#[derive(Clone, Copy, Debug)]
enum Corruption {
    ContextChunk,
    BucketChunk,
    Manifest,
}

#[tokio::test]
async fn corrupt_preferred_base_falls_back_to_another_live_base_in_the_same_pull() -> Result<()> {
    for corruption in [
        Corruption::ContextChunk,
        Corruption::BucketChunk,
        Corruption::Manifest,
    ] {
        let namespace = Uuid::new_v4();
        let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        source
            .sync_store
            .write_local(
                ObjectKey {
                    collection: Collection::Playlists,
                    id: Uuid::from_u128(60),
                },
                support::playlist("covered by the floor base"),
            )
            .await?;
        let store = Arc::new(MemoryObjectStore::default());
        let mut published = publish_base(&source, store.clone(), Default::default()).await?;
        append_segment(&mut published, 61, "covered by the newer base").await?;
        let (newer_id, newer) = publish_newer_base(&mut published).await?;

        let corrupt = if let Corruption::Manifest = corruption {
            // Canonical, but no longer the manifest its base id names.
            let mut tampered = newer.clone();
            tampered
                .collections
                .get_mut(RuneModel::collection_tag(&Collection::Playlists))
                .context("newer base has no playlist collection")?
                .root
                .candidate_count += 1;
            let (_, bytes) = share::encode_manifest(&tampered)?;
            let key = share::base_key(&published.prefix, &newer_id)?;
            store
                .put(&key, &bytes)
                .await
                .map_err(|error| anyhow::anyhow!("corrupt newer manifest: {error:?}"))?;
            key
        } else {
            let newer_playlists = playlist_summary(&newer)?;
            let name = if let Corruption::ContextChunk = corruption {
                newer_playlists
                    .context_fragments
                    .first()
                    .context("newer playlist context has no chunk")?
            } else {
                &newer_playlists
                    .buckets
                    .first()
                    .context("newer playlist collection has no bucket")?
                    .chunk
            };
            let floor = base_manifest(
                &store,
                &published.prefix,
                &published.head.floor_base.base_id,
            )
            .await?;
            ensure!(
                floor.collections.values().all(|collection| {
                    !collection.context_fragments.contains(name)
                        && collection
                            .buckets
                            .iter()
                            .all(|bucket| &bucket.chunk != name)
                }),
                "the floor base shares the chunk that is corrupted"
            );
            let key = share::chunk_key(&published.prefix, name)?;
            store
                .put(&key, b"corrupt gzip")
                .await
                .map_err(|error| anyhow::anyhow!("corrupt newer base chunk: {error:?}"))?;
            key
        };

        let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        pull(&published, &target)
            .await
            .with_context(|| format!("pull with a corrupt newer {corruption:?}"))?;
        assert_ne!(
            store.get_count(&corrupt),
            0,
            "the receiver did not try the newer base first"
        );
        for id in [60, 61] {
            let key = ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(id),
            };
            assert_eq!(
                target.sync_store.get(&key).await?,
                source.sync_store.get(&key).await?,
                "playlist {id} did not converge from the floor base"
            );
        }
        let checkpoint = p::CheckpointKey {
            namespace,
            source_replica: published.head.stream,
            source_generation: published.head.generation,
            scope_id: p::scope_id(&full_scope())?,
        };
        assert_eq!(
            p::ReceiveStore::checkpoint(&target.sync_store.endpoint(), &checkpoint).await?,
            Some(p::ReceiveProgress {
                sequence: published.head.head.seq,
                anchor: published.head.head.anchor,
            })
        );
    }
    Ok(())
}

#[tokio::test]
async fn bases_failing_after_reconciliation_began_leave_room_for_a_valid_base() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    source
        .sync_store
        .write_local(
            ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(70),
            },
            support::playlist("covered by the floor base"),
        )
        .await?;
    let store = Arc::new(MemoryObjectStore::default());
    let mut published = publish_base(&source, store.clone(), Default::default()).await?;
    // Four newer bases rank above the floor base. Each one fails on its playlist bucket chunk,
    // which a receiver reads only after it has begun a reconciliation session.
    let mut corrupt = Vec::new();
    for id in 71..75 {
        append_segment(&mut published, id, "covered by a newer base").await?;
        let (_, newer) = publish_newer_base(&mut published).await?;
        let bucket = &playlist_summary(&newer)?
            .buckets
            .first()
            .context("newer playlist collection has no bucket")?
            .chunk;
        let key = share::chunk_key(&published.prefix, bucket)?;
        store
            .put(&key, b"corrupt gzip")
            .await
            .map_err(|error| anyhow::anyhow!("corrupt newer base chunk: {error:?}"))?;
        corrupt.push(key);
    }

    let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    pull(&published, &target).await?;
    for key in &corrupt {
        assert_ne!(
            store.get_count(key),
            0,
            "the receiver did not try every newer base"
        );
    }
    for id in 70..75 {
        let key = ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::from_u128(id),
        };
        assert_eq!(
            target.sync_store.get(&key).await?,
            source.sync_store.get(&key).await?,
            "playlist {id} did not converge from the floor base"
        );
    }
    let checkpoint = p::CheckpointKey {
        namespace,
        source_replica: published.head.stream,
        source_generation: published.head.generation,
        scope_id: p::scope_id(&full_scope())?,
    };
    assert_eq!(
        p::ReceiveStore::checkpoint(&target.sync_store.endpoint(), &checkpoint).await?,
        Some(p::ReceiveProgress {
            sequence: published.head.head.seq,
            anchor: published.head.head.anchor,
        })
    );
    Ok(())
}

async fn share_prefix_keys(store: &MemoryObjectStore, prefix: &str) -> Result<Vec<String>> {
    store
        .list_prefix(&format!("devices/{prefix}/"))
        .await
        .map_err(|error| anyhow::anyhow!("list prefix {prefix}: {error:?}"))
}

/// Another library's upload of library.json lands after this share was created.
async fn hand_share_to_another_library(store: &MemoryObjectStore) -> Result<Uuid> {
    let other = Uuid::new_v4();
    let library = LibraryFile {
        format: share::FORMAT_VERSION,
        library_id: other,
        model: RuneModel::model_id().to_owned(),
        schema: RuneModel::schema_id().to_owned(),
    };
    store
        .put(&share::library_key(), &share::encode_library(&library)?)
        .await
        .map_err(|error| anyhow::anyhow!("replace library.json: {error:?}"))?;
    Ok(other)
}

async fn publish_first_share_pass(
    device: &SqlLibrary,
    store: &Arc<MemoryObjectStore>,
    namespace: Uuid,
    id: u128,
) -> Result<String> {
    device
        .sync_store
        .write_local(
            ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(id),
            },
            support::playlist("share creation race"),
        )
        .await?;
    device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), Default::default())
        .await?;
    let prefix = device.sync_store.share_prefix(cache_path()).await?;
    ensure!(
        !share_prefix_keys(store, &prefix).await?.is_empty(),
        "the device published nothing to its prefix"
    );
    Ok(prefix)
}

#[tokio::test]
async fn device_that_lost_the_share_creation_removes_only_its_own_prefix() -> Result<()> {
    let namespace = Uuid::new_v4();
    let store = Arc::new(MemoryObjectStore::default());
    // The creator writes library.json with a plain PUT, as on a store without conditional
    // create, and publishes in the same pass. Another device of the library joins after it.
    let creator = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let creator_prefix = publish_first_share_pass(&creator, &store, namespace, 1701).await?;
    let joiner = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let joiner_prefix = publish_first_share_pass(&joiner, &store, namespace, 1702).await?;

    // Another library's upload of library.json lands late, and that library publishes too.
    let other_namespace = hand_share_to_another_library(&store).await?;
    let other = SqlLibrary::new(other_namespace, Uuid::new_v4()).await?;
    let other_prefix = publish_first_share_pass(&other, &store, other_namespace, 1703).await?;
    let joiner_keys = share_prefix_keys(&store, &joiner_prefix).await?;
    let other_keys = share_prefix_keys(&store, &other_prefix).await?;

    // The creator meets the other library on its next pass and removes what it wrote.
    assert_eq!(
        creator
            .sync_store
            .pull_share(store.clone(), namespace, cache_path(), Default::default())
            .await
            .unwrap_err(),
        SharePullError::ShareChanged
    );
    assert_eq!(
        share_prefix_keys(&store, &creator_prefix).await?,
        Vec::<String>::new()
    );

    // A device that did not create library.json keeps its prefix, and so does the other library,
    // which owns the share and keeps syncing.
    assert_eq!(
        joiner
            .sync_store
            .pull_share(store.clone(), namespace, cache_path(), Default::default())
            .await
            .unwrap_err(),
        SharePullError::ShareChanged
    );
    assert_eq!(
        share_prefix_keys(&store, &joiner_prefix).await?,
        joiner_keys
    );
    assert_eq!(share_prefix_keys(&store, &other_prefix).await?, other_keys);
    other
        .sync_store
        .pull_share(
            store.clone(),
            other_namespace,
            cache_path(),
            Default::default(),
        )
        .await?;
    Ok(())
}

#[tokio::test]
async fn device_keeps_its_prefix_once_its_share_creation_held() -> Result<()> {
    let namespace = Uuid::new_v4();
    let store = Arc::new(MemoryObjectStore::default());
    let creator = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let creator_prefix = publish_first_share_pass(&creator, &store, namespace, 1711).await?;
    // The next pass after the first publication still finds this library's library.json.
    creator
        .sync_store
        .pull_share(store.clone(), namespace, cache_path(), Default::default())
        .await?;
    let creator_keys = share_prefix_keys(&store, &creator_prefix).await?;

    hand_share_to_another_library(&store).await?;
    assert_eq!(
        creator
            .sync_store
            .pull_share(store.clone(), namespace, cache_path(), Default::default())
            .await
            .unwrap_err(),
        SharePullError::ShareChanged
    );
    assert_eq!(
        share_prefix_keys(&store, &creator_prefix).await?,
        creator_keys
    );
    Ok(())
}

#[tokio::test]
async fn share_prefix_whose_head_another_writer_session_wrote_is_never_removed() -> Result<()> {
    let namespace = Uuid::new_v4();
    let store = Arc::new(MemoryObjectStore::default());
    let replica = Uuid::new_v4();
    let creator = SqlLibrary::new(namespace, replica).await?;
    let creator_prefix = publish_first_share_pass(&creator, &store, namespace, 1721).await?;
    let creator_keys = share_prefix_keys(&store, &creator_prefix).await?;
    // This database now holds another writer session than the one that wrote the prefix's head,
    // as a restored copy of the device does.
    let mut state = load_or_create_state(
        &creator.db,
        replica,
        namespace,
        cache_path(),
        Utc::now().timestamp_millis(),
    )
    .await?;
    state.writer_session = Uuid::new_v4();
    save_state(&creator.db, &state).await?;

    hand_share_to_another_library(&store).await?;
    assert_eq!(
        creator
            .sync_store
            .pull_share(store.clone(), namespace, cache_path(), Default::default())
            .await
            .unwrap_err(),
        SharePullError::ShareChanged
    );
    assert_eq!(
        share_prefix_keys(&store, &creator_prefix).await?,
        creator_keys
    );
    Ok(())
}

async fn journal_head_and_floor(library: &SqlLibrary) -> Result<(i64, i64)> {
    let row = library
        .db
        .query_one(
            &Query::select()
                .columns([Alias::new("head"), Alias::new("history_floor")])
                .from(Alias::new("sync_meta"))
                .to_owned(),
        )
        .await?
        .context("missing journal metadata")?;
    Ok((row.try_get_by_index(0)?, row.try_get_by_index(1)?))
}

/// Runs maintenance, as the hub does in the background, until compaction has moved the
/// history floor up to the journal head.
async fn compact_journal(library: &SqlLibrary) -> Result<i64> {
    let mut maintenance = Maintenance::default();
    for _ in 0..64 {
        let (head, floor) = journal_head_and_floor(library).await?;
        if floor == head {
            return Ok(floor);
        }
        library.sync_store.maintain(&mut maintenance).await?;
    }
    anyhow::bail!("maintenance did not compact the journal")
}

/// Runs what the share worker runs for a pass: it asks for pending changes, then pulls and
/// pushes.
async fn share_worker_pass(device: &SqlLibrary, store: &Arc<MemoryObjectStore>) -> Result<Head> {
    let namespace = device.sync_store.identity().await?.namespace;
    let pending = device
        .sync_store
        .share_pending_changes(cache_path())
        .await?
        .context("the compacted journal tail was not reported as work to publish")?;
    ensure!(
        pending.change == PendingShareChange::Content,
        "the compacted journal tail was reported as statistics only"
    );
    device
        .sync_store
        .pull_share(store.clone(), namespace, cache_path(), Default::default())
        .await?;
    device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), Default::default())
        .await?
        .context("the push pass published no head")
}

#[tokio::test]
async fn journal_compacted_before_the_first_publication_is_published_as_a_base() -> Result<()> {
    let namespace = Uuid::new_v4();
    let device = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(2301),
    };
    device
        .sync_store
        .write_local(
            key.clone(),
            support::playlist("written before the first publication"),
        )
        .await?;
    // Nothing pins the journal before the first publication creates the export cursor.
    let floor = compact_journal(&device).await?;
    ensure!(floor > 0, "maintenance left the journal uncompacted");

    let store = Arc::new(MemoryObjectStore::default());
    let head = share_worker_pass(&device, &store).await?;
    ensure!(head.bases.len() == 1, "the first publication is not a base");
    let joining = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    joining
        .sync_store
        .pull_share(store, namespace, cache_path(), Default::default())
        .await?;
    assert_eq!(
        joining.sync_store.get(&key).await?,
        device.sync_store.get(&key).await?
    );
    Ok(())
}

#[tokio::test]
async fn journal_compacted_past_an_expired_export_cursor_is_published_as_a_base() -> Result<()> {
    let namespace = Uuid::new_v4();
    let device = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    let first = device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), Default::default())
        .await?
        .context("the first push published no head")?;
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(2302),
    };
    device
        .sync_store
        .write_local(
            key.clone(),
            support::playlist("written after the export cursor expired"),
        )
        .await?;
    // An export cursor older than its maximum age stops pinning, so compaction passes it.
    let expired = Utc::now().timestamp_millis() - EXPORT_CURSOR_MAX_AGE_MS - 60_000;
    device
        .db
        .execute(
            &Query::update()
                .table(Alias::new("sync_export_cursors"))
                .value(Alias::new("advanced_at_ms"), expired)
                .to_owned(),
        )
        .await?;
    let floor = compact_journal(&device).await?;
    ensure!(
        u64::try_from(floor)? > first.local_cursor,
        "compaction did not pass the export cursor"
    );

    let head = share_worker_pass(&device, &store).await?;
    ensure!(
        head.bases.len() == first.bases.len() + 1,
        "the publication after compaction is not a base"
    );
    let joining = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    joining
        .sync_store
        .pull_share(store, namespace, cache_path(), Default::default())
        .await?;
    assert_eq!(
        joining.sync_store.get(&key).await?,
        device.sync_store.get(&key).await?
    );
    Ok(())
}

/// Puts the scratch objects that `OpenDalObjectStore::probe` (fsio) writes into the folder it
/// probes, `devices/<folder>/`, as the share holds them while a probe runs, or after a probe was cut
/// short or could not delete them. The names are the ones shares already hold.
async fn put_store_probe_objects(store: &MemoryObjectStore, folder: &str) -> Result<Vec<String>> {
    let mut keys = Vec::new();
    for name in [
        ".probe-",
        ".conditional-create-",
        ".conditional-put-",
        ".conditional-get-",
    ] {
        let key = format!("devices/{folder}/{name}{}", Uuid::new_v4());
        store
            .put(&key, b"rune object-store probe")
            .await
            .map_err(|error| anyhow::anyhow!("put probe object {key}: {error:?}"))?;
        keys.push(key);
    }
    keys.sort();
    Ok(keys)
}

/// A device publishes to the share, a store probe's scratch objects are in `devices/<folder>/`, and
/// a new device pulls the share as usual.
async fn pull_passes_over_store_probe_objects_in(folder: &str) -> Result<()> {
    let namespace = Uuid::new_v4();
    let store = Arc::new(MemoryObjectStore::default());
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    publish_first_share_pass(&source, &store, namespace, 1731).await?;
    put_store_probe_objects(&store, folder).await?;
    let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    target
        .sync_store
        .pull_share(store, namespace, cache_path(), Default::default())
        .await
        .map_err(|error| anyhow::anyhow!("pull with probe objects in {folder}: {error:?}"))?;
    let playlist = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(1731),
    };
    ensure!(
        target.sync_store.get(&playlist).await?.is_some(),
        "the pull did not receive the published playlist"
    );
    assert_eq!(
        target.sync_store.get(&playlist).await?,
        source.sync_store.get(&playlist).await?
    );
    Ok(())
}

/// The share worker probes the device's own folder each time it opens the store, so another device
/// can list the share while that folder holds the probe's scratch objects.
#[tokio::test]
async fn pull_passes_over_store_probe_objects_in_a_device_folder() -> Result<()> {
    pull_passes_over_store_probe_objects_in(&format!("{}.{}", Uuid::new_v4(), Uuid::new_v4())).await
}

/// The Add Library guide probes `devices/<uuid>.1/`, a folder that is not a device prefix.
#[tokio::test]
async fn pull_passes_over_store_probe_objects_in_the_add_library_guides_folder() -> Result<()> {
    pull_passes_over_store_probe_objects_in(&format!("{}.1", Uuid::new_v4())).await
}

/// A probe of the device's own folder that could not delete its scratch objects leaves them in the
/// folder that every push pass lists for the device's own garbage collection. The pass publishes
/// as usual and leaves them alone: only the probe deletes them, since one can be running there.
#[tokio::test]
async fn push_passes_over_store_probe_objects_in_its_own_folder() -> Result<()> {
    let namespace = Uuid::new_v4();
    let store = Arc::new(MemoryObjectStore::default());
    let device = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let prefix = publish_first_share_pass(&device, &store, namespace, 1741).await?;
    let probe_keys = put_store_probe_objects(&store, &prefix).await?;
    let playlist = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(1742),
    };
    device
        .sync_store
        .write_local(playlist.clone(), support::playlist("after the probe"))
        .await?;
    let published = device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), Default::default())
        .await
        .map_err(|error| anyhow::anyhow!("push with probe objects in its own folder: {error:?}"))?;
    ensure!(published.is_some(), "the push pass published nothing");
    let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    target
        .sync_store
        .pull_share(store.clone(), namespace, cache_path(), Default::default())
        .await?;
    assert_eq!(
        target.sync_store.get(&playlist).await?,
        device.sync_store.get(&playlist).await?
    );
    let left = share_prefix_keys(&store, &prefix).await?;
    for key in &probe_keys {
        assert!(left.contains(key), "the push pass deleted {key}");
    }
    Ok(())
}

/// A device that lost the share creation removes what it published to its folder and passes over
/// the scratch objects that its store probe left there.
#[tokio::test]
async fn device_that_lost_the_share_creation_passes_over_store_probe_objects() -> Result<()> {
    let namespace = Uuid::new_v4();
    let store = Arc::new(MemoryObjectStore::default());
    let creator = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let creator_prefix = publish_first_share_pass(&creator, &store, namespace, 1751).await?;
    let probe_keys = put_store_probe_objects(&store, &creator_prefix).await?;
    hand_share_to_another_library(&store).await?;
    assert_eq!(
        creator
            .sync_store
            .pull_share(store.clone(), namespace, cache_path(), Default::default())
            .await
            .unwrap_err(),
        SharePullError::ShareChanged
    );
    assert_eq!(
        share_prefix_keys(&store, &creator_prefix).await?,
        probe_keys
    );
    Ok(())
}

/// The memory store, counting per key the object bodies it sends and their bytes: GETs that find
/// the object, and conditional GETs that find it changed. A conditional GET of an unchanged object
/// sends none.
#[derive(Default)]
struct DownloadCounter {
    inner: MemoryObjectStore,
    sent: Mutex<BTreeMap<String, (u64, u64)>>,
}

impl DownloadCounter {
    fn send(&self, key: &str, value: &ObjectValue) {
        let mut sent = self.sent.lock().unwrap();
        let (bodies, bytes) = sent.entry(key.to_owned()).or_default();
        *bodies += 1;
        *bytes += value.bytes.len() as u64;
    }

    fn sent(&self) -> BTreeMap<String, (u64, u64)> {
        self.sent.lock().unwrap().clone()
    }
}

impl ObjectStore for DownloadCounter {
    fn get<'a>(&'a self, key: &'a str) -> p::BoxFuture<'a, ObjectValue, StoreError> {
        Box::pin(async move {
            let value = self.inner.get(key).await?;
            self.send(key, &value);
            Ok(value)
        })
    }

    fn get_if_none_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
    ) -> p::BoxFuture<'a, ConditionalGet, StoreError> {
        Box::pin(async move {
            let result = self.inner.get_if_none_match(key, tag).await?;
            if let ConditionalGet::Modified(value) = &result {
                self.send(key, value);
            }
            Ok(result)
        })
    }

    fn put<'a>(&'a self, key: &'a str, bytes: &'a [u8]) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        self.inner.put(key, bytes)
    }

    fn create<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        self.inner.create(key, bytes)
    }

    fn put_if_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
        bytes: &'a [u8],
    ) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        self.inner.put_if_match(key, tag, bytes)
    }

    fn delete<'a>(&'a self, key: &'a str) -> p::BoxFuture<'a, (), StoreError> {
        self.inner.delete(key)
    }

    fn list_prefix<'a>(&'a self, prefix: &'a str) -> p::BoxFuture<'a, Vec<String>, StoreError> {
        self.inner.list_prefix(prefix)
    }
}

/// What a probe finds on the memory store, or, with `conditional_get` false, on a store that
/// answers a conditional GET with the whole object.
fn memory_store_capabilities(conditional_get: bool) -> ShareCapabilities {
    ShareCapabilities {
        conditional_create: true,
        conditional_head_write: true,
        conditional_get,
    }
}

/// The bodies, and their bytes, that `store` sent of each segment, chunk and manifest in the
/// device's folder since `before`, objects deleted since included.
async fn bodies_since(
    store: &DownloadCounter,
    device: &SqlLibrary,
    before: &BTreeMap<String, (u64, u64)>,
) -> Result<BTreeMap<String, (u64, u64)>> {
    let folder = format!(
        "devices/{}/",
        device.sync_store.share_prefix(cache_path()).await?
    );
    let mut sent = BTreeMap::new();
    for (key, (bodies, bytes)) in store.sent() {
        if !key.starts_with(&folder)
            || !matches!(
                parse_key(&key)?,
                ShareObjectKey::Base { .. }
                    | ShareObjectKey::Chunk { .. }
                    | ShareObjectKey::Segment { .. }
            )
        {
            continue;
        }
        let (bodies_before, bytes_before) = before.get(&key).copied().unwrap_or_default();
        if bodies > bodies_before {
            sent.insert(key, (bodies - bodies_before, bytes - bytes_before));
        }
    }
    Ok(sent)
}

/// By kind of object: the objects in the device's folder now, and the bodies and bytes in `sent`.
async fn by_kind(
    store: &DownloadCounter,
    device: &SqlLibrary,
    sent: &BTreeMap<String, (u64, u64)>,
) -> Result<BTreeMap<&'static str, (u64, u64, u64)>> {
    fn kind(key: &str) -> Result<Option<&'static str>> {
        Ok(match parse_key(key)? {
            ShareObjectKey::Base { .. } => Some("base"),
            ShareObjectKey::Chunk { .. } => Some("chunk"),
            ShareObjectKey::Segment { .. } => Some("segment"),
            _ => None,
        })
    }
    let mut summary = BTreeMap::<&str, (u64, u64, u64)>::new();
    let prefix = device.sync_store.share_prefix(cache_path()).await?;
    for key in share_prefix_keys(&store.inner, &prefix).await? {
        if let Some(kind) = kind(&key)? {
            summary.entry(kind).or_default().0 += 1;
        }
    }
    for (key, (bodies, bytes)) in sent {
        if let Some(kind) = kind(key)? {
            let entry = summary.entry(kind).or_default();
            entry.1 += bodies;
            entry.2 += bytes;
        }
    }
    Ok(summary)
}

/// Backdates the device's first observation of its newest base past the age at which any device
/// cuts a new base.
async fn age_newest_own_base(device: &SqlLibrary, head: &Head) -> Result<()> {
    let identity = device.sync_store.identity().await?;
    let now_ms = Utc::now().timestamp_millis();
    let mut state = load_or_create_state(
        &device.db,
        identity.replica,
        identity.namespace,
        cache_path(),
        now_ms,
    )
    .await?;
    let newest = ShareObjectKey::Base {
        prefix: DevicePrefix {
            stream: state.stream,
            generation: state.generation,
        },
        base_id: head
            .bases
            .last()
            .context("the head has no base")?
            .base_id
            .clone(),
    };
    state
        .observed_objects
        .get_mut(&newest)
        .context("the newest base was never observed")?
        .first_seen_ms = now_ms - share::BASE_MAX_AGE_MS as i64 - 1;
    save_state(&device.db, &state).await?;
    Ok(())
}

/// A deterministic, incompressible name of `len` characters.
fn random_name(rng: &mut StdRng, len: usize) -> String {
    rng.sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

/// Writes a playlist on `device`.
async fn add_playlist(device: &SqlLibrary, id: u128, name: &str) -> Result<()> {
    device
        .sync_store
        .write_local(
            ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(id),
            },
            support::playlist(name),
        )
        .await?;
    Ok(())
}

/// A push pass downloads what it needs to publish its change, and none of the base, chunks and
/// segments that the device published and that have not changed since, also when a base cut
/// takes segments out of the head and deletes them. The device records what its writes returned,
/// and confirms the tags with a conditional GET. A store without conditional GET sends each object
/// once per pass, as the garbage collection has to read every tag, and in a pass that cuts a base
/// the manifest it keeps once more, as the cut checks it before it writes anything.
#[tokio::test]
async fn push_pass_does_not_download_the_devices_unchanged_objects() -> Result<()> {
    for conditional_get in [true, false] {
        let capabilities = memory_store_capabilities(conditional_get);
        let namespace = Uuid::new_v4();
        let store = Arc::new(DownloadCounter::default());
        let device = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        let mut rng = StdRng::seed_from_u64(699);
        for id in 1..=40u128 {
            add_playlist(&device, id, &random_name(&mut rng, 4096)).await?;
        }
        // Pass 0 publishes the 40 playlists in the first base. Passes 1 to 3 each publish one
        // more playlist in a segment, and pass 4 publishes one more in a new base, as the first
        // one is too old; its collection deletes the three segments that the new base covers.
        let mut passes = Vec::new();
        let mut head: Option<Head> = None;
        for pass in 0..=4u128 {
            if pass > 0 {
                add_playlist(&device, 1000 + pass, "one more").await?;
            }
            if pass == 4 {
                age_newest_own_base(&device, head.as_ref().context("no head before pass 4")?)
                    .await?;
            }
            let before = store.sent();
            let published = device
                .sync_store
                .push_share(store.clone(), namespace, cache_path(), capabilities)
                .await
                .map_err(|error| anyhow::anyhow!("push pass {pass}: {error:?}"))?
                .with_context(|| format!("push pass {pass} published no head"))?;
            let sent = bodies_since(&store, &device, &before).await?;
            let summary = by_kind(&store, &device, &sent).await?;
            println!(
                "conditional GET {conditional_get}, push pass {pass}: (objects in the folder, \
                 bodies sent during the pass, bytes) {summary:?}"
            );
            let most = sent.values().map(|(bodies, _)| *bodies).max().unwrap_or(0);
            passes.push((summary, most));
            head = Some(published);
        }
        let head = head.context("no head after the passes")?;
        ensure!(
            head.bases.len() == 2 && head.segments.is_empty(),
            "pass 4 did not cut a new base that covers the segments"
        );
        let prefix = device.sync_store.share_prefix(cache_path()).await?;
        ensure!(
            share_prefix_keys(&store.inner, &prefix)
                .await?
                .iter()
                .all(|key| !matches!(parse_key(key), Ok(ShareObjectKey::Segment { .. }))),
            "pass 4 did not delete the segments that its base covers"
        );
        // Without conditional GET, the cut in pass 4 reads the manifest it keeps to check it before
        // it writes anything, and the collection reads it again for its tag.
        let allowed = |pass: usize| match (conditional_get, pass) {
            (true, _) => 0,
            (false, 4) => 2,
            (false, _) => 1,
        };
        assert!(
            passes
                .iter()
                .enumerate()
                .all(|(pass, (_, most))| *most <= allowed(pass)),
            "with conditional GET {conditional_get}, push passes downloaded objects that the \
             device published and did not change more often than allowed: {passes:?}"
        );
        let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        target
            .sync_store
            .pull_share(store.clone(), namespace, cache_path(), Default::default())
            .await?;
        for id in [1, 40, 1001, 1004] {
            let key = ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(id),
            };
            assert_eq!(
                target.sync_store.get(&key).await?,
                device.sync_store.get(&key).await?,
                "a new device did not receive playlist {id}"
            );
        }
    }
    Ok(())
}

/// A device that is not the keeper rolls its segments up: sixteen L0 segments become one L1. The
/// pass that rolls them up reads the ones it published before, once, since it needs their events.
/// The rolled-up segments stay in the folder until a base of the device covers them, and the
/// passes after the roll-up download none of them.
#[tokio::test]
async fn push_passes_after_a_roll_up_do_not_download_the_rolled_up_segments() -> Result<()> {
    let capabilities = memory_store_capabilities(true);
    let namespace = Uuid::new_v4();
    let store = Arc::new(DownloadCounter::default());
    // The keeper is the active stream with the smallest id. With the other device as the keeper,
    // this device rolls its segments up instead of cutting them into a base.
    let first = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let second = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let (keeper, device) = if first.sync_store.share_prefix(cache_path()).await?
        < second.sync_store.share_prefix(cache_path()).await?
    {
        (first, second)
    } else {
        (second, first)
    };
    add_playlist(&keeper, 1791, "the keeper's").await?;
    keeper
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), capabilities)
        .await
        .map_err(|error| anyhow::anyhow!("the keeper's push: {error:?}"))?;
    let mut rng = StdRng::seed_from_u64(707);
    add_playlist(&device, 1792, &random_name(&mut rng, 8192)).await?;
    let prefix = device.sync_store.share_prefix(cache_path()).await?;
    let mut head: Option<Head> = None;
    let mut rolled_up = None;
    // Per pass: (bodies of the segments that the pass rolled up, any other bodies).
    let mut passes = Vec::new();
    for pass in 0..=18u128 {
        if pass > 0 {
            add_playlist(&device, 1800 + pass, "one more").await?;
        }
        let before = store.sent();
        let published = device
            .sync_store
            .push_share(store.clone(), namespace, cache_path(), capabilities)
            .await
            .map_err(|error| anyhow::anyhow!("push pass {pass}: {error:?}"))?
            .with_context(|| format!("push pass {pass} published no head"))?;
        let sent = bodies_since(&store, &device, &before).await?;
        // A roll-up reads the L0 segments of the previous head that it rolls up.
        let rolls_up = published.segments.iter().any(|segment| segment.level == 1)
            && head
                .as_ref()
                .is_some_and(|head| head.segments.iter().all(|segment| segment.level == 0));
        let inputs = match (&head, rolls_up) {
            (Some(previous), true) => previous
                .segments
                .iter()
                .map(|segment| {
                    share::segment_key(&prefix, segment.first, segment.last, &segment.name)
                })
                .collect::<result::Result<BTreeSet<_>, _>>()?,
            _ => BTreeSet::new(),
        };
        let read_once = |key: &String| inputs.contains(key) && sent[key].0 == 1;
        let rolled = sent.keys().filter(|key| read_once(key)).count();
        let other = sent
            .iter()
            .filter(|(key, _)| !read_once(key))
            .map(|(_, (bodies, _))| bodies)
            .sum::<u64>();
        println!(
            "push pass {pass}: segments it rolled up read once {rolled} of {}, other bodies \
             {other}",
            inputs.len()
        );
        if rolls_up {
            rolled_up = Some((pass, inputs.clone()));
        }
        passes.push((rolled == inputs.len(), other));
        head = Some(published);
    }
    let (pass, inputs) = rolled_up.context("no pass rolled the segments up")?;
    let listed = share_prefix_keys(&store.inner, &prefix).await?;
    ensure!(
        pass < 18 && !inputs.is_empty() && inputs.iter().all(|key| listed.contains(key)),
        "the rolled-up segments did not stay in the folder for the passes after the roll-up"
    );
    assert!(
        passes
            .iter()
            .all(|(all_inputs_once, other)| *all_inputs_once && *other == 0),
        "push passes downloaded more than the segments a roll-up rolls up, once each: \
         {passes:?}"
    );
    Ok(())
}

/// The counting memory store, answering each write of a segment, chunk or manifest with another
/// tag than its reads report for the same bytes, as some stores do.
#[derive(Default)]
struct OtherWriteTags {
    inner: DownloadCounter,
}

impl OtherWriteTags {
    fn written(key: &str, tag: ObjectTag) -> ObjectTag {
        if matches!(
            parse_key(key),
            Ok(ShareObjectKey::Base { .. }
                | ShareObjectKey::Chunk { .. }
                | ShareObjectKey::Segment { .. })
        ) {
            ObjectTag::new(format!("\"written-{}\"", tag.as_str()))
        } else {
            tag
        }
    }
}

impl ObjectStore for OtherWriteTags {
    fn get<'a>(&'a self, key: &'a str) -> p::BoxFuture<'a, ObjectValue, StoreError> {
        self.inner.get(key)
    }

    fn get_if_none_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
    ) -> p::BoxFuture<'a, ConditionalGet, StoreError> {
        self.inner.get_if_none_match(key, tag)
    }

    fn put<'a>(&'a self, key: &'a str, bytes: &'a [u8]) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move { Ok(Self::written(key, self.inner.put(key, bytes).await?)) })
    }

    fn create<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        Box::pin(async move { Ok(Self::written(key, self.inner.create(key, bytes).await?)) })
    }

    fn put_if_match<'a>(
        &'a self,
        key: &'a str,
        tag: &'a ObjectTag,
        bytes: &'a [u8],
    ) -> p::BoxFuture<'a, ObjectTag, StoreError> {
        self.inner.put_if_match(key, tag, bytes)
    }

    fn delete<'a>(&'a self, key: &'a str) -> p::BoxFuture<'a, (), StoreError> {
        self.inner.delete(key)
    }

    fn list_prefix<'a>(&'a self, prefix: &'a str) -> p::BoxFuture<'a, Vec<String>, StoreError> {
        self.inner.list_prefix(prefix)
    }
}

/// Where a store answers a write with another tag than its reads report, the first collection
/// after the write sees another tag and downloads the object. It records what it read, so that no
/// manifest, chunk or segment of the device is downloaded a second time: not while a segment is in
/// the head, and not when a base cut takes the segments out of the head and deletes them.
#[tokio::test]
async fn push_passes_download_an_object_once_where_writes_return_other_tags() -> Result<()> {
    let capabilities = memory_store_capabilities(true);
    let namespace = Uuid::new_v4();
    let store = Arc::new(OtherWriteTags::default());
    let device = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let mut rng = StdRng::seed_from_u64(707);
    for id in 1..=40u128 {
        add_playlist(&device, id, &random_name(&mut rng, 4096)).await?;
    }
    let prefix = device.sync_store.share_prefix(cache_path()).await?;
    // Every manifest, chunk and segment that was in the device's folder after a pass.
    let mut objects = BTreeSet::new();
    let mut head: Option<Head> = None;
    // As in push_pass_does_not_download_the_devices_unchanged_objects, with one more pass after
    // the base cut of pass 4.
    for pass in 0..=5u128 {
        if pass > 0 {
            add_playlist(&device, 1100 + pass, "one more").await?;
        }
        if pass == 4 {
            age_newest_own_base(&device, head.as_ref().context("no head before pass 4")?).await?;
        }
        let published = device
            .sync_store
            .push_share(store.clone(), namespace, cache_path(), capabilities)
            .await
            .map_err(|error| anyhow::anyhow!("push pass {pass}: {error:?}"))?
            .with_context(|| format!("push pass {pass} published no head"))?;
        for key in share_prefix_keys(&store.inner.inner, &prefix).await? {
            if matches!(
                parse_key(&key)?,
                ShareObjectKey::Base { .. }
                    | ShareObjectKey::Chunk { .. }
                    | ShareObjectKey::Segment { .. }
            ) {
                objects.insert(key);
            }
        }
        head = Some(published);
    }
    let head = head.context("no head after the passes")?;
    let is_segment = |key: &&String| matches!(parse_key(key), Ok(ShareObjectKey::Segment { .. }));
    let listed = share_prefix_keys(&store.inner.inner, &prefix).await?;
    ensure!(
        head.bases.len() == 2
            && head.segments.len() == 1
            && objects.iter().filter(is_segment).count() == 4
            && listed.iter().filter(is_segment).count() == 1,
        "pass 4 did not cut a base that took the three segments out of the head and deleted them"
    );
    let downloaded = bodies_since(&store.inner, &device, &BTreeMap::new())
        .await?
        .into_iter()
        .map(|(key, (bodies, _))| (key, bodies))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        downloaded,
        objects.iter().map(|key| (key.clone(), 1)).collect(),
        "push passes did not download each object of the device's folder exactly once"
    );
    Ok(())
}

/// Replaces the manifest of the head's newest base with a canonical manifest that its base id no
/// longer names, or deletes it.
async fn change_or_delete_manifest(
    store: &MemoryObjectStore,
    prefix: &str,
    head: &Head,
    delete: bool,
) -> Result<()> {
    let base_id = &head.bases.last().context("the head has no base")?.base_id;
    let key = share::base_key(prefix, base_id)?;
    if delete {
        return store
            .delete(&key)
            .await
            .map_err(|error| anyhow::anyhow!("delete the manifest: {error:?}"));
    }
    let mut tampered = base_manifest(store, prefix, base_id).await?;
    tampered
        .collections
        .get_mut(RuneModel::collection_tag(&Collection::Playlists))
        .context("the base has no playlist collection")?
        .root
        .candidate_count += 1;
    let (_, bytes) = share::encode_manifest(&tampered)?;
    store
        .put(&key, &bytes)
        .await
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!("replace the manifest: {error:?}"))
}

/// The sizes that the base cut decision compares: the stored manifest and the compressed content
/// that the manifest references. The content size counts each chunk and context fragment once,
/// the way the base exporter totals it.
async fn base_sizes(store: &MemoryObjectStore, prefix: &str, head: &Head) -> Result<(u64, u64)> {
    let base = head.bases.last().context("the head has no base")?;
    let manifest_bytes = store
        .get(&share::base_key(prefix, &base.base_id)?)
        .await
        .map_err(|error| anyhow::anyhow!("read base manifest: {error:?}"))?
        .bytes;
    let manifest = share::decode_manifest::<RuneModel>(&manifest_bytes, &base.base_id)?;
    let mut chunks = BTreeSet::new();
    for collection in manifest.collections.values() {
        chunks.extend(
            collection
                .buckets
                .iter()
                .map(|bucket| bucket.chunk.clone())
                .chain(collection.context_fragments.iter().cloned()),
        );
    }
    let mut content_gzip_bytes = 0_u64;
    for chunk in chunks {
        content_gzip_bytes += store
            .get(&share::chunk_key(prefix, &chunk)?)
            .await
            .map_err(|error| anyhow::anyhow!("read base chunk: {error:?}"))?
            .bytes
            .len() as u64;
    }
    Ok((manifest_bytes.len() as u64, content_gzip_bytes))
}

/// Plans the update after `head` only to measure the gzip size of the segments it would publish.
fn planned_segment_gzip_bytes(
    identity: &p::StoreIdentity,
    head: &Head,
    entries: &[share::JournalEvent<RuneModel>],
) -> Result<u64> {
    let no_streams = BTreeSet::new();
    let no_replicas = BTreeSet::new();
    let share::PublishDecision::PublishSegments {
        compressed_bytes, ..
    } = share::plan_publish(share::PublishInput {
        identity,
        after_cursor: head.local_cursor,
        entries,
        share_stream_ids: &no_streams,
        share_local_replicas: &no_replicas,
        head: Checkpoint {
            seq: head.head.seq,
            anchor: head.head.anchor,
        },
        existing_segments: &head.segments,
        newest_base: share::BaseSize::Absent,
        export_cursor: share::ExportCursorStatus::Current,
        max_events_per_segment: 100,
    })
    .map_err(|error| anyhow::anyhow!("plan the share update: {error}"))?
    else {
        anyhow::bail!("the share update did not plan as a segment");
    };
    Ok(compressed_bytes)
}

/// The base fraction compares the update's gzip bytes with the compressed content of the newest
/// base, not with the size of its manifest. An update above a quarter of the manifest but within a
/// quarter of the content publishes a segment; one above a quarter of the content cuts a base; and
/// a base that is due by age still cuts for a small update.
#[tokio::test]
async fn push_uses_the_compressed_base_content_size_for_the_base_fraction() -> Result<()> {
    let namespace = Uuid::new_v4();
    let store = Arc::new(MemoryObjectStore::default());
    let device = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let capabilities = memory_store_capabilities(true);
    let mut rng = StdRng::seed_from_u64(440);
    // Incompressible playlist names make the base's compressed chunks much larger than its
    // manifest, so the two denominators give different decisions.
    for id in 0..48_u128 {
        add_playlist(&device, 5000 + id, &random_name(&mut rng, 4096)).await?;
    }
    let mut head = device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), capabilities)
        .await?
        .context("the first push pass published no head")?;
    ensure!(head.bases.len() == 1, "the first push did not cut a base");
    let prefix = device.sync_store.share_prefix(cache_path()).await?;
    let (manifest_bytes, base_gzip_bytes) = base_sizes(&store, &prefix, &head).await?;

    add_playlist(&device, 6000, &random_name(&mut rng, 4096)).await?;
    let identity = SyncPeer::identity(&device.sync_store.endpoint()).await?;
    let entries = journal_events_since(&device.sync_store.endpoint(), &head).await?;
    let update_gzip_bytes = planned_segment_gzip_bytes(&identity, &head, &entries)?;
    ensure!(
        manifest_bytes < update_gzip_bytes.saturating_mul(4),
        "the fixture does not exercise the manifest size: manifest {manifest_bytes}, update {update_gzip_bytes}"
    );
    ensure!(
        update_gzip_bytes.saturating_mul(4) <= base_gzip_bytes,
        "the fixture update exceeds a quarter of the base content: update {update_gzip_bytes}, content {base_gzip_bytes}"
    );
    head = device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), capabilities)
        .await?
        .context("the update published no head")?;
    ensure!(
        head.bases.len() == 1,
        "the push cut a base for an update within a quarter of the base content"
    );
    ensure!(
        head.segments.len() == 1 && head.segments[0].bytes == update_gzip_bytes,
        "the push did not publish the planned segment"
    );

    // The other side of the quarter: an update above a quarter of the content still cuts.
    for id in 0..8_u128 {
        add_playlist(&device, 7000 + id, &random_name(&mut rng, 16384)).await?;
    }
    let entries = journal_events_since(&device.sync_store.endpoint(), &head).await?;
    let large_update_gzip_bytes = planned_segment_gzip_bytes(&identity, &head, &entries)?;
    ensure!(
        large_update_gzip_bytes.saturating_mul(4) > base_gzip_bytes,
        "the large fixture update stays within a quarter of the base content: update {large_update_gzip_bytes}, content {base_gzip_bytes}"
    );
    head = device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), capabilities)
        .await?
        .context("the large update published no head")?;
    ensure!(
        head.bases.len() == 2,
        "the push kept the base for an update above a quarter of its content"
    );

    // A cut that is due by age happens for a small update that the base fraction would keep
    // incremental.
    let (_, aged_base_gzip_bytes) = base_sizes(&store, &prefix, &head).await?;
    age_newest_own_base(&device, &head).await?;
    add_playlist(&device, 8000, "an update after the base aged").await?;
    let entries = journal_events_since(&device.sync_store.endpoint(), &head).await?;
    let small_update_gzip_bytes = planned_segment_gzip_bytes(&identity, &head, &entries)?;
    ensure!(
        small_update_gzip_bytes.saturating_mul(4) <= aged_base_gzip_bytes,
        "the small fixture update exceeds a quarter of the aged base content: update {small_update_gzip_bytes}, content {aged_base_gzip_bytes}"
    );
    head = device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), capabilities)
        .await?
        .context("the update after the base aged published no head")?;
    ensure!(
        head.bases.len() == 3,
        "an aged base was not cut for a small update"
    );
    Ok(())
}

/// A base whose manifest is larger than a quarter of the update but whose compressed content is
/// smaller must still cut: the fraction uses the content, not the manifest.
#[tokio::test]
async fn push_cuts_a_small_base_whose_manifest_is_larger_than_its_content() -> Result<()> {
    let namespace = Uuid::new_v4();
    let store = Arc::new(MemoryObjectStore::default());
    let device = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let capabilities = memory_store_capabilities(true);
    add_playlist(&device, 9000, "small base").await?;
    let mut head = device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), capabilities)
        .await?
        .context("the first push pass published no head")?;
    ensure!(head.bases.len() == 1, "the first push did not cut a base");
    let prefix = device.sync_store.share_prefix(cache_path()).await?;
    let (manifest_bytes, content_gzip_bytes) = base_sizes(&store, &prefix, &head).await?;

    add_playlist(&device, 9001, "small update").await?;
    let identity = SyncPeer::identity(&device.sync_store.endpoint()).await?;
    let entries = journal_events_since(&device.sync_store.endpoint(), &head).await?;
    let update_gzip_bytes = planned_segment_gzip_bytes(&identity, &head, &entries)?;
    ensure!(
        update_gzip_bytes.saturating_mul(4) > content_gzip_bytes,
        "the fixture update stays within a quarter of the base content: update {update_gzip_bytes}, content {content_gzip_bytes}"
    );
    ensure!(
        update_gzip_bytes.saturating_mul(4) <= manifest_bytes,
        "the fixture manifest is not larger than a quarter of the update: update {update_gzip_bytes}, manifest {manifest_bytes}"
    );
    head = device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), capabilities)
        .await?
        .context("the update published no head")?;
    ensure!(
        head.bases.len() == 2,
        "the push kept a base whose manifest is larger than a quarter of the update"
    );
    Ok(())
}

/// Replaces the manifest the device recorded for `base_id` with one from an earlier build: the
/// content size and the candidate flag are unknown, as in a state file written before this build
/// kept them.
async fn forget_the_manifest_size_and_flag(device: &SqlLibrary, base_id: &str) -> Result<()> {
    let identity = device.sync_store.identity().await?;
    let now_ms = Utc::now().timestamp_millis();
    let mut state = load_or_create_state(
        &device.db,
        identity.replica,
        identity.namespace,
        cache_path(),
        now_ms,
    )
    .await?;
    let object = ShareObjectKey::Base {
        prefix: DevicePrefix {
            stream: state.stream,
            generation: state.generation,
        },
        base_id: base_id.to_owned(),
    };
    let record = state
        .own_objects
        .get_mut(&object)
        .context("the base was never recorded")?;
    let OwnContents::Manifest { chunks, .. } = &record.contents else {
        bail!(
            "the recorded base {base_id} is not a manifest: {:?}",
            record.contents
        );
    };
    record.contents = OwnContents::Manifest {
        content_gzip_bytes: None,
        chunks: chunks.clone(),
        has_candidates: None,
        manifest_gzip_bytes: 0,
    };
    save_state(&device.db, &state).await?;
    Ok(())
}

/// The content size and candidate flag the device has recorded for `base_id`.
async fn recorded_manifest_size_and_flag(
    device: &SqlLibrary,
    base_id: &str,
) -> Result<(Option<u64>, Option<bool>)> {
    let identity = device.sync_store.identity().await?;
    let now_ms = Utc::now().timestamp_millis();
    let state = load_or_create_state(
        &device.db,
        identity.replica,
        identity.namespace,
        cache_path(),
        now_ms,
    )
    .await?;
    let object = ShareObjectKey::Base {
        prefix: DevicePrefix {
            stream: state.stream,
            generation: state.generation,
        },
        base_id: base_id.to_owned(),
    };
    let Some(OwnRecord {
        contents:
            OwnContents::Manifest {
                content_gzip_bytes,
                has_candidates,
                ..
            },
        ..
    }) = state.own_objects.get(&object)
    else {
        bail!("the base {base_id} was never recorded as a manifest");
    };
    Ok((*content_gzip_bytes, *has_candidates))
}

/// A manifest recorded by an earlier build carries neither a content size nor a candidate flag.
/// The push must resolve both from the manifest itself before it decides with the size: a base of
/// context only keeps its first edit incremental instead of cutting, and a base with candidates
/// records its positive content total and the flag with it.
#[tokio::test]
async fn push_resolves_the_candidate_flag_of_a_manifest_record_from_an_earlier_build() -> Result<()>
{
    let namespace = Uuid::new_v4();
    let store = Arc::new(MemoryObjectStore::default());
    let device = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let capabilities = memory_store_capabilities(true);
    // The keeper cuts a base for the empty library. It holds the collection context fragments and
    // no candidate chunk, so measuring its chunks must not stand in for catalog content.
    let head = device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), capabilities)
        .await?
        .context("the first push pass published no head")?;
    ensure!(head.bases.len() == 1, "the first push did not cut a base");
    let base_id = head.bases[0].base_id.clone();
    let (cut_content_gzip_bytes, cut_has_candidates) =
        recorded_manifest_size_and_flag(&device, &base_id).await?;
    ensure!(
        cut_content_gzip_bytes.unwrap_or(0) > 0 && cut_has_candidates == Some(false),
        "the empty base is not a context-only base: {cut_content_gzip_bytes:?}, {cut_has_candidates:?}"
    );
    forget_the_manifest_size_and_flag(&device, &base_id).await?;

    add_playlist(&device, 9100, "first edit").await?;
    let next = device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), capabilities)
        .await?
        .context("the update published no head")?;
    ensure!(
        next.bases.len() == 1,
        "the push cut a base for the first edit of a context-only base"
    );
    ensure!(
        next.segments.len() == 1,
        "the first edit of a context-only base did not publish a segment"
    );
    let (content_gzip_bytes, has_candidates) =
        recorded_manifest_size_and_flag(&device, &base_id).await?;
    ensure!(
        content_gzip_bytes == cut_content_gzip_bytes,
        "the context-only base did not record its content size: {content_gzip_bytes:?}, cut {cut_content_gzip_bytes:?}"
    );
    ensure!(
        has_candidates == Some(false),
        "the context-only base did not record its candidate flag: {has_candidates:?}"
    );

    // The same unknown flag on a base whose chunks do hold candidates is resolved too, and its
    // positive content total is recorded with the flag before the fraction decides with it.
    let namespace = Uuid::new_v4();
    let store = Arc::new(MemoryObjectStore::default());
    let device = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let mut rng = StdRng::seed_from_u64(440);
    // An incompressible name keeps the base content well above a quarter of the update.
    add_playlist(&device, 9110, &random_name(&mut rng, 4096)).await?;
    let head = device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), capabilities)
        .await?
        .context("the first push pass published no head")?;
    ensure!(head.bases.len() == 1, "the first push did not cut a base");
    let base_id = head.bases[0].base_id.clone();
    let (cut_content_gzip_bytes, cut_has_candidates) =
        recorded_manifest_size_and_flag(&device, &base_id).await?;
    let cut_content_gzip_bytes = cut_content_gzip_bytes
        .filter(|bytes| *bytes > 0)
        .context("the candidate base recorded no content size")?;
    ensure!(
        cut_has_candidates == Some(true),
        "the candidate base did not record its candidate flag: {cut_has_candidates:?}"
    );
    forget_the_manifest_size_and_flag(&device, &base_id).await?;

    add_playlist(&device, 9111, "within the quarter").await?;
    let prefix = device.sync_store.share_prefix(cache_path()).await?;
    let (_, content_gzip_bytes) = base_sizes(&store, &prefix, &head).await?;
    let identity = SyncPeer::identity(&device.sync_store.endpoint()).await?;
    let entries = journal_events_since(&device.sync_store.endpoint(), &head).await?;
    let update_gzip_bytes = planned_segment_gzip_bytes(&identity, &head, &entries)?;
    ensure!(
        update_gzip_bytes.saturating_mul(4) <= content_gzip_bytes,
        "the fixture update does not stay within a quarter of the base content: update {update_gzip_bytes}, content {content_gzip_bytes}"
    );
    let next = device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), capabilities)
        .await?
        .context("the update published no head")?;
    ensure!(
        next.bases.len() == 1,
        "the push cut a base for an update within a quarter of the candidate base content"
    );
    let (recorded, has_candidates) = recorded_manifest_size_and_flag(&device, &base_id).await?;
    ensure!(
        recorded == Some(cut_content_gzip_bytes),
        "the candidate base did not record its measured content total: {recorded:?}, cut {cut_content_gzip_bytes:?}"
    );
    ensure!(
        has_candidates == Some(true),
        "the candidate base did not record its candidate flag: {has_candidates:?}"
    );
    Ok(())
}

/// A base cut takes the chunk names of the bases it keeps from what the device recorded only once
/// the store has confirmed each manifest. When one is gone, or no longer matches its base id, the
/// pass stops before it writes anything of the new base.
#[tokio::test]
async fn base_cut_stops_before_it_writes_when_a_kept_manifest_changed_or_disappeared() -> Result<()>
{
    for conditional_get in [true, false] {
        for disappeared in [false, true] {
            let capabilities = memory_store_capabilities(conditional_get);
            let namespace = Uuid::new_v4();
            let store = Arc::new(MemoryObjectStore::default());
            let device = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
            add_playlist(&device, 1793, "in the first base").await?;
            let head = device
                .sync_store
                .push_share(store.clone(), namespace, cache_path(), capabilities)
                .await?
                .context("the first push pass published no head")?;
            let prefix = device.sync_store.share_prefix(cache_path()).await?;
            change_or_delete_manifest(&store, &prefix, &head, disappeared).await?;
            add_playlist(&device, 1794, "for the next base").await?;
            age_newest_own_base(&device, &head).await?;
            let before = share_prefix_keys(&store, &prefix).await?;
            let result = device
                .sync_store
                .push_share(store.clone(), namespace, cache_path(), capabilities)
                .await;
            let stopped = if disappeared {
                matches!(result, Err(SharePushError::MissingObject))
            } else {
                matches!(&result, Err(SharePushError::Failed(message)) if message.contains("digest"))
            };
            let what = if disappeared {
                "disappeared"
            } else {
                "changed"
            };
            assert!(
                stopped,
                "with conditional GET {conditional_get}, a base cut after the manifest {what} \
                 gave {result:?}"
            );
            assert_eq!(
                share_prefix_keys(&store, &prefix).await?,
                before,
                "with conditional GET {conditional_get}, a base cut after the manifest {what} \
                 wrote to the share"
            );
        }
    }
    Ok(())
}

/// A share state saved by an earlier build has no records of the device's manifests and segments.
/// The next pass reads the device's base manifest to learn it, and the passes after it do not read
/// it again.
#[tokio::test]
async fn push_pass_learns_the_manifests_a_state_from_an_earlier_build_does_not_record() -> Result<()>
{
    let namespace = Uuid::new_v4();
    let store = Arc::new(DownloadCounter::default());
    let device = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let capabilities = memory_store_capabilities(true);
    for pass in 0..=2u128 {
        add_playlist(&device, 1781 + pass, &format!("playlist {pass}")).await?;
        if pass == 1 {
            let identity = device.sync_store.identity().await?;
            let state = load_or_create_state(
                &device.db,
                identity.replica,
                namespace,
                cache_path(),
                Utc::now().timestamp_millis(),
            )
            .await?;
            let mut encoded = serde_json::to_value(&state)?;
            encoded
                .as_object_mut()
                .context("the share state is not a JSON object")?
                .remove("own_objects")
                .context("the share state keeps no records")?;
            let earlier: ShareState = serde_json::from_value(encoded)?;
            ensure!(earlier.own_objects.is_empty());
            save_state(&device.db, &earlier).await?;
        }
        let before = store.sent();
        device
            .sync_store
            .push_share(store.clone(), namespace, cache_path(), capabilities)
            .await
            .map_err(|error| anyhow::anyhow!("push pass {pass}: {error:?}"))?;
        let sent = by_kind(
            &store,
            &device,
            &bodies_since(&store, &device, &before).await?,
        )
        .await?;
        let manifest_bodies = sent.get("base").map_or(0, |(_, bodies, _)| *bodies);
        if pass == 1 {
            ensure!(
                manifest_bodies > 0,
                "the pass after the earlier build did not read the base manifest: {sent:?}"
            );
        } else {
            ensure!(
                sent.values().all(|(_, bodies, _)| *bodies == 0),
                "push pass {pass} downloaded objects that did not change: {sent:?}"
            );
        }
    }
    Ok(())
}

/// The garbage collection deletes an object that two of its runs at least 24 hours apart both saw
/// unreferenced. An object whose tag changed since the first of them is not the object those runs
/// saw, so its wait starts again, whether the pass confirms tags with a conditional GET or
/// downloads the objects.
#[tokio::test]
async fn garbage_collection_starts_the_wait_again_for_an_object_that_changed() -> Result<()> {
    for conditional_get in [true, false] {
        let capabilities = memory_store_capabilities(conditional_get);
        let namespace = Uuid::new_v4();
        let store = Arc::new(MemoryObjectStore::default());
        let device = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
        add_playlist(&device, 1761, "published").await?;
        device
            .sync_store
            .push_share(store.clone(), namespace, cache_path(), capabilities)
            .await?;
        let prefix = device.sync_store.share_prefix(cache_path()).await?;
        // Two chunks that no manifest references, as an interrupted base cut leaves them.
        let unchanged = share::chunk_key(&prefix, blake3::hash(b"unchanged").to_hex().as_str())?;
        let replaced = share::chunk_key(&prefix, blake3::hash(b"replaced").to_hex().as_str())?;
        for key in [&unchanged, &replaced] {
            store
                .put(key, b"orphan chunk")
                .await
                .map_err(|error| anyhow::anyhow!("put {key}: {error:?}"))?;
        }
        device
            .sync_store
            .push_share(store.clone(), namespace, cache_path(), capabilities)
            .await?;
        // The run that first saw them is more than 24 hours ago, and a second run saw them too.
        let identity = device.sync_store.identity().await?;
        let now_ms = Utc::now().timestamp_millis();
        let mut state = load_or_create_state(
            &device.db,
            identity.replica,
            namespace,
            cache_path(),
            now_ms,
        )
        .await?;
        for key in [&unchanged, &replaced] {
            let observed = state
                .observed_objects
                .get_mut(&parse_key(key)?)
                .with_context(|| format!("{key} was not observed"))?;
            ensure!(
                matches!(
                    observed.gc_state,
                    ObjectGcState::Unreferenced {
                        second_observation: GcObservation::NotObserved,
                        ..
                    }
                ),
                "{key} was not seen unreferenced: {:?}",
                observed.gc_state
            );
            observed.gc_state = ObjectGcState::Unreferenced {
                first_seen_ms: now_ms - share::OBJECT_UNREFERENCED_MS as i64 - 10,
                second_observation: GcObservation::ObservedAt(now_ms as u64),
            };
        }
        save_state(&device.db, &state).await?;
        // Someone writes other bytes under one of the names before the next run.
        store
            .put(&replaced, b"another orphan chunk")
            .await
            .map_err(|error| anyhow::anyhow!("replace {replaced}: {error:?}"))?;
        let replaced_tag = store
            .get(&replaced)
            .await
            .map_err(|error| anyhow::anyhow!("read {replaced}: {error:?}"))?
            .tag;
        let run_ms = Utc::now().timestamp_millis();
        device
            .sync_store
            .push_share(store.clone(), namespace, cache_path(), capabilities)
            .await?;
        ensure!(
            matches!(store.get(&unchanged).await, Err(StoreError::NotFound)),
            "with conditional GET {conditional_get}, the run kept the orphan that both runs saw"
        );
        ensure!(
            store.get(&replaced).await.is_ok(),
            "with conditional GET {conditional_get}, the run deleted the orphan that changed"
        );
        let state = load_or_create_state(
            &device.db,
            identity.replica,
            namespace,
            cache_path(),
            Utc::now().timestamp_millis(),
        )
        .await?;
        let observed = state
            .observed_objects
            .get(&parse_key(&replaced)?)
            .context("the replaced orphan is no longer observed")?;
        assert_eq!(observed.tag, replaced_tag);
        assert!(
            matches!(
                observed.gc_state,
                ObjectGcState::Unreferenced {
                    first_seen_ms,
                    second_observation: GcObservation::NotObserved,
                } if first_seen_ms >= run_ms
            ),
            "with conditional GET {conditional_get}, the wait for the replaced orphan did not \
             start again: {:?}",
            observed.gc_state
        );
    }
    Ok(())
}

/// The garbage collection takes the chunks that a base references from the device's record of
/// its manifest only while the store confirms the manifest's tag. A manifest that changed is read
/// again, and the pass stops before it collects anything when the manifest no longer matches its
/// base id or is gone.
#[tokio::test]
async fn garbage_collection_stops_on_a_base_manifest_that_changed_or_disappeared() -> Result<()> {
    for conditional_get in [true, false] {
        for disappeared in [false, true] {
            let capabilities = memory_store_capabilities(conditional_get);
            let namespace = Uuid::new_v4();
            let store = Arc::new(MemoryObjectStore::default());
            let device = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
            add_playlist(&device, 1771, "in the base").await?;
            let head = device
                .sync_store
                .push_share(store.clone(), namespace, cache_path(), capabilities)
                .await?
                .context("the first push pass published no head")?;
            let prefix = device.sync_store.share_prefix(cache_path()).await?;
            change_or_delete_manifest(&store, &prefix, &head, disappeared).await?;
            let result = device
                .sync_store
                .push_share(store.clone(), namespace, cache_path(), capabilities)
                .await;
            let stopped = if disappeared {
                matches!(result, Err(SharePushError::MissingObject))
            } else {
                matches!(&result, Err(SharePushError::Failed(message)) if message.contains("digest"))
            };
            assert!(
                stopped,
                "with conditional GET {conditional_get}, a pass after the manifest {} gave \
                 {result:?}",
                if disappeared {
                    "disappeared"
                } else {
                    "changed"
                }
            );
        }
    }
    Ok(())
}

/// The keeper does not rebuild a base that holds no candidate chunks just because a tail exists:
/// replacing it on the first tail rebuilt the base for every acknowledgement. The tail stays a
/// segment, the head keeps the single base, and a device joining afterwards reads the playlist
/// through it.
#[tokio::test]
async fn pull_joins_a_share_whose_contentless_base_keeps_its_tail() -> Result<()> {
    let namespace = Uuid::new_v4();
    let store = Arc::new(MemoryObjectStore::default());
    let device = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let capabilities = memory_store_capabilities(true);
    // The keeper cuts a base for the empty library, and the first edit stays a segment.
    let head = device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), capabilities)
        .await?
        .context("the first push pass published no head")?;
    ensure!(head.bases.len() == 1, "the first push did not cut a base");
    add_playlist(&device, 9300, "first content").await?;
    let head = device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), capabilities)
        .await?
        .context("the first edit published no head")?;
    ensure!(
        head.bases.len() == 1 && head.segments.len() == 1,
        "the first edit of the empty base did not stay incremental: {:?}",
        (head.bases.len(), head.segments.len())
    );
    let tail_head = device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), capabilities)
        .await
        .map_err(|error| anyhow::anyhow!("the pass with a tail failed: {error:?}"))?;
    let head = tail_head.unwrap_or_else(|| head.clone());
    ensure!(
        head.bases.len() == 1 && head.segments.len() == 1,
        "the keeper rebuilt the contentless base for its first tail: {:?}",
        (head.bases.len(), head.segments.len())
    );

    let joining = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    joining
        .sync_store
        .pull_share(store.clone(), namespace, cache_path(), capabilities)
        .await
        .context("joining the share failed")?;
    let playlist = joining
        .sync_store
        .get(&ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::from_u128(9300),
        })
        .await?
        .context("the joined device is missing the playlist")?;
    ensure!(
        playlist.value == support::playlist("first content"),
        "the joined device read a different playlist: {:?}",
        playlist.value
    );
    Ok(())
}

/// Regression test for the rule: an empty base only cuts when the tail carries catalog
/// content. Share starts empty (base manifest only), then a library is added. The tail
/// carries candidates, so the keeper cuts a new base containing the library. A joining
/// device then reads the library from that new base rather than replaying segments.
#[tokio::test]
async fn pull_joins_with_empty_base_then_adds_library_through_new_base() -> Result<()> {
    let namespace = Uuid::new_v4();
    let store = Arc::new(MemoryObjectStore::default());
    let device = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let capabilities = memory_store_capabilities(true);

    // First push: empty library, cuts a base with manifest only (no candidates)
    let head = device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), capabilities)
        .await?
        .context("the first push pass published no head")?;
    ensure!(head.bases.len() == 1, "the first push did not cut a base");

    // Add 50 playlists to ensure the tail is large enough to exceed manifest/4
    for i in 0..50 {
        add_playlist(&device, 8888 + i, &format!("added library {}", i)).await?;
    }

    // Second push: tail has candidates, should cut a new base containing the library
    let head = device
        .sync_store
        .push_share(store.clone(), namespace, cache_path(), capabilities)
        .await?
        .context("the push with library published no head")?;
    ensure!(
        head.bases.len() == 2,
        "expected 2 bases after adding library (original empty + new with content), got {}",
        head.bases.len()
    );

    // A joining device should read the playlist from the new base
    let joining = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    joining
        .sync_store
        .pull_share(store.clone(), namespace, cache_path(), capabilities)
        .await
        .context("joining the share failed")?;

    let playlist = joining
        .sync_store
        .get(&ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::from_u128(8888),
        })
        .await?
        .context("the joined device is missing the playlist")?;

    ensure!(
        playlist.value == support::playlist("added library 0"),
        "the joined device read a different playlist: {:?}",
        playlist.value
    );

    Ok(())
}
