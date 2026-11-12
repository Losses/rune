#[path = "support/causal_library.rs"]
mod support;

use anyhow::{Context, Result, ensure};
use database::sync::model::{Collection, ObjectKey, RuneModel};
use database::sync::share::{
    PublishJournalRequest, ShareCapabilities, SharePullError, SharePushError, SharePushResult,
    ShareState, ShareWriter, load_or_create_state, publish_journal, save_state,
};
use sea_orm::{
    ConnectionTrait, ExprTrait, TransactionTrait,
    sea_query::{Alias, Expr, Query},
};
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
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
use uuid::Uuid;

struct Published {
    endpoint: CausalEndpoint<RuneModel>,
    prefix: String,
    head: Head,
    bucket_chunks: Vec<String>,
    store: Arc<MemoryObjectStore>,
}

fn cache_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("share-pull-{}", Uuid::new_v4()))
}

async fn age_base_observation(
    source: &SqlLibrary,
    published: &Published,
    store: &MemoryObjectStore,
    age_ms: i64,
) -> Result<()> {
    source
        .library
        .pull_share(
            published.store.clone(),
            source.library.identity().await?.namespace,
            cache_path(),
            Default::default(),
        )
        .await?;
    let identity = source.library.identity().await?;
    let now_ms = chrono::Utc::now().timestamp_millis();
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
    let endpoint = source.library.endpoint();
    let identity = SyncPeer::identity(&endpoint).await?;
    let mut state = load_or_create_state(
        &source.db,
        identity.replica,
        identity.namespace,
        cache_path(),
        chrono::Utc::now().timestamp_millis(),
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
        .library
        .pull_share(
            store,
            target.library.identity().await?.namespace,
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
    let page_read = endpoint
        .store()
        .read_page_with_origins(p::PageRequest {
            source: identity.clone(),
            scope: full_scope(),
            after: p::ReceiveProgress {
                sequence: after.head.seq,
                anchor: after.head.anchor,
            },
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
    let cursor =
        sync_orm::causal_export_cursors::read_on(&tx, namespace, stream.as_bytes()).await?;
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
        .library
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
        .library
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
        target.library.get(&key).await?,
        source.library.get(&key).await?
    );
    Ok(())
}

#[tokio::test]
async fn push_pass_pulls_before_reading_and_publishing_the_local_tail() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let remote = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    source
        .library
        .write_local(
            ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(147),
            },
            support::playlist("push pass baseline"),
        )
        .await?;
    let store = Arc::new(MemoryObjectStore::default());
    let published = publish_base(&source, store.clone(), Default::default()).await?;
    let remote_key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(149),
    };
    remote
        .library
        .write_local(remote_key.clone(), support::playlist("remote stream"))
        .await?;
    let _published_remote = publish_base(&remote, store.clone(), Default::default()).await?;
    let key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(148),
    };
    source
        .library
        .write_local(key.clone(), support::playlist("push pass local tail"))
        .await?;
    let head = source
        .library
        .push_share(
            store.clone(),
            namespace,
            cache_path(),
            ShareCapabilities {
                conditional_create: true,
                conditional_head_write: true,
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!("share push: {error}"))?
        .context("push pass did not publish the local journal")?;
    assert_eq!(head.head.seq, published.head.head.seq + 1);
    assert_eq!(
        source.library.get(&remote_key).await?,
        remote.library.get(&remote_key).await?
    );
    let target_result = target
        .library
        .pull_share(store, namespace, cache_path(), Default::default())
        .await;
    target_result.map_err(|error| anyhow::anyhow!("target pull: {error}"))?;
    assert_eq!(
        target.library.get(&key).await?,
        source.library.get(&key).await?
    );
    assert_eq!(
        target.library.get(&remote_key).await?,
        remote.library.get(&remote_key).await?
    );
    Ok(())
}

#[tokio::test]
async fn stale_base_is_replaced_by_own_stream_cut() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    source
        .library
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
        .collect::<std::result::Result<Vec<_>, _>>()?;
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
        .library
        .pull_share(store.clone(), namespace, cache_path(), Default::default())
        .await?;
    let identity = source.library.identity().await?;
    let mut state = load_or_create_state(
        &source.db,
        identity.replica,
        namespace,
        cache_path(),
        chrono::Utc::now().timestamp_millis(),
    )
    .await?;
    let base_key = share::base_key(&published.prefix, &published.head.bases[0].base_id)?;
    let base = store
        .get(&base_key)
        .await
        .map_err(|error| anyhow::anyhow!("read base: {error:?}"))?;
    let old_time = chrono::Utc::now().timestamp_millis() - share::BASE_MAX_AGE_MS as i64 - 1;
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
        .library
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
        source.library.identity().await?.replica,
        namespace,
        cache_path(),
        chrono::Utc::now().timestamp_millis(),
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
        since_ms: chrono::Utc::now().timestamp_millis()
            - share::SUPERSEDED_BASE_RETENTION_MS as i64
            - 1,
    };
    save_state(&source.db, &state).await?;
    let tail_key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(914),
    };
    source
        .library
        .write_local(tail_key.clone(), support::playlist("tail after base cut"))
        .await?;
    source
        .library
        .push_share(store.clone(), namespace, cache_path(), Default::default())
        .await?;
    ensure!(
        matches!(store.get(&old_base_key).await, Err(StoreError::NotFound)),
        "GC retained a superseded base after its retention deadline"
    );
    let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    target
        .library
        .pull_share(store, namespace, cache_path(), Default::default())
        .await?;
    ensure!(
        target.library.get(&tail_key).await? == source.library.get(&tail_key).await?,
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
        .library
        .write_local(key.clone(), support::playlist("first share base"))
        .await?;

    let head = source
        .library
        .push_share(store.clone(), namespace, cache_path(), Default::default())
        .await?
        .context("first push did not create a base")?;
    ensure!(
        head.bases.len() == 1,
        "first share head does not reference its base"
    );
    target
        .library
        .pull_share(store, namespace, cache_path(), Default::default())
        .await?;
    ensure!(
        target.library.get(&key).await? == source.library.get(&key).await?,
        "first base did not initialize a new device"
    );
    Ok(())
}

#[tokio::test]
async fn only_the_elected_keeper_cuts_a_retained_base() -> Result<()> {
    let namespace = Uuid::new_v4();
    let a = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let b = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let store = Arc::new(MemoryObjectStore::default());
    a.library
        .write_local(
            ObjectKey {
                collection: Collection::Playlists,
                id: Uuid::from_u128(918),
            },
            support::playlist("keeper A"),
        )
        .await?;
    b.library
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
        .library
        .push_share(store.clone(), namespace, cache_path(), Default::default())
        .await?;
    let non_keeper_head =
        non_keeper_head.context("non-keeper did not publish its acknowledgement")?;
    ensure!(
        non_keeper_head.bases.len() == non_keeper_base.head.bases.len(),
        "non-keeper cut the base"
    );
    let keeper_head = keeper
        .library
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
        .library
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
        .library
        .write_local(key.clone(), support::playlist("before interrupted cut"))
        .await?;
    source
        .library
        .pull_share(store.clone(), namespace, cache_path(), Default::default())
        .await?;
    let identity = source.library.identity().await?;
    let state = load_or_create_state(
        &source.db,
        identity.replica,
        namespace,
        cache_path(),
        chrono::Utc::now().timestamp_millis(),
    )
    .await?;
    let prefix = format!("{}.{}", state.stream, state.generation);
    let export = export_base(
        &source.library.endpoint(),
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
        .collect::<std::result::Result<Vec<_>, _>>()?;
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
        .library
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
        .library
        .write_local(key, support::playlist("replacement snapshot"))
        .await?;
    let replacement = source
        .library
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
        chrono::Utc::now().timestamp_millis(),
    )
    .await?;
    let old_time =
        chrono::Utc::now().timestamp_millis() - share::OBJECT_UNREFERENCED_MS as i64 - 10;
    let second_observation = chrono::Utc::now().timestamp_millis();
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
        .library
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
    a.library
        .write_local(key_a.clone(), support::playlist("authored on A"))
        .await?;
    b.library
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
    assert_eq!(c.library.get(&key_a).await?, a.library.get(&key_a).await?);
    assert_eq!(c.library.get(&key_b).await?, b.library.get(&key_b).await?);
    Ok(())
}

#[tokio::test]
async fn relayed_event_is_filtered_from_the_receiving_devices_own_stream() -> Result<()> {
    let namespace = Uuid::new_v4();
    let a = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let b = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    for (library, id) in [(&a, 144), (&b, 145)] {
        library
            .library
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
    a.library
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
        .library
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
        .library
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
        .library
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
        .library
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
            .library
            .get(&key)
            .await?
            .context("pulled playlist missing")?;
        assert_eq!(Some(actual), source.library.get(&key).await?);
    }

    append_segment(&mut published, 3, "incremental").await?;
    pull(&published, &target).await?;
    let actual = target
        .library
        .get(&ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::from_u128(3),
        })
        .await?
        .context("incremental playlist missing")?;
    assert_eq!(
        Some(actual),
        source
            .library
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
        .library
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
        .collect::<std::result::Result<Vec<_>, _>>()?;
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
        .library
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
        .library
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
    let checkpoint = p::ReceiveStore::checkpoint(&target.library.endpoint(), &key).await?;
    let base_key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(10),
    };
    let segment_key = ObjectKey {
        collection: Collection::Playlists,
        id: Uuid::from_u128(11),
    };
    let base_value = target.library.get(&base_key).await?;
    let segment_value = target.library.get(&segment_key).await?;
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
        .library
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
        .library
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
            .library
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
                target.library.get(&key).await?,
                source.library.get(&key).await?,
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
            p::ReceiveStore::checkpoint(&target.library.endpoint(), &checkpoint_key).await?,
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
        .library
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
        .library
        .get(&ObjectKey {
            collection: Collection::Playlists,
            id: Uuid::from_u128(21),
        })
        .await?;
    assert!(before.is_none(), "target unexpectedly has post-floor data");
    pull(&published, &target).await?;
    let recovered = target
        .library
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
        p::ReceiveStore::checkpoint(&target.library.endpoint(), &checkpoint).await?,
        Some(p::ReceiveProgress {
            sequence: published.head.head.seq,
            anchor: published.head.head.anchor,
        })
    );
    Ok(())
}
