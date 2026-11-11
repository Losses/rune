#[path = "support/causal_library.rs"]
mod support;

use anyhow::{Context, Result, ensure};
use database::sync::model::{Collection, ObjectKey, RuneModel};
use database::sync::share::SharePullError;
use sea_orm::{
    ConnectionTrait, ExprTrait,
    sea_query::{Alias, Expr, Query},
};
use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use support::SqlLibrary;
use sync::share::{
    BaseRef, Checkpoint, ConditionalGet, FloorBase, Head, LibraryFile, MemoryObjectStore,
    ObjectKey as ShareObjectKey, ObjectStore, ObjectTag, ObjectValue, Segment, SegmentLink,
    SegmentRef, StoreError, parse_key,
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

async fn publish_base(
    source: &SqlLibrary,
    store: Arc<MemoryObjectStore>,
    covers: BTreeMap<String, Checkpoint>,
) -> Result<Published> {
    let endpoint = source.library.endpoint();
    let identity = SyncPeer::identity(&endpoint).await?;
    let stream = Uuid::new_v5(&identity.replica, b"rune-share");
    let generation = Uuid::new_v4();
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
        writer_session: Uuid::new_v4(),
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
