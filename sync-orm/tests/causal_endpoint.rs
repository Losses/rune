use anyhow::{Context, Result, ensure};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

fn session_limits() -> p::SessionLimits {
    p::SessionLimits {
        pages: p::PageLimits {
            max_scanned: 4,
            max_bytes: 65536,
        },
        stages: p::StageLimits {
            max_total_bytes: p::DEFAULT_STAGE_BYTES,
            max_stages: p::DEFAULT_STAGES,
            max_event_bytes: p::DEFAULT_LOGICAL_BYTES,
            max_event_items: p::DEFAULT_LOGICAL_ITEMS,
            max_fragment_bytes: p::DEFAULT_PAGE_BYTES,
        },
        apply: p::ApplyLimits {
            max_projection_items: 2,
            max_projection_bytes: 65536,
            max_mutations: 1024,
        },
        work: p::WorkLimits {
            max_requests: 32,
            max_bytes: 1024 * 1024,
            max_depth: 32,
            max_rows_scanned: 16,
            max_mutations: 1024,
        },
    }
}
async fn synchronize(
    source: &CausalEndpoint<Notes>,
    target: &CausalEndpoint<Notes>,
) -> Result<usize> {
    let mut coordinator = p::Coordinator::new(
        p::Scope {
            collections: [1].into_iter().collect(),
        },
        session_limits(),
    )?;
    for step in 0..20000 {
        match coordinator
            .advance(target, target, target, source, source)
            .await?
        {
            p::SessionAdvance::Complete(_) => return Ok(step),
            p::SessionAdvance::BudgetExhausted => coordinator.start_pass(),
            p::SessionAdvance::Incomplete => (),
            p::SessionAdvance::Cancelled => anyhow::bail!("unexpected cancellation"),
        }
    }
    anyhow::bail!("SQL reconciliation did not finish")
}

#[tokio::test]
async fn single_key_write_does_not_read_neighboring_candidate_bodies() -> Result<()> {
    use std::sync::atomic::AtomicUsize;
    let namespace = Uuid::new_v4();
    let (mut db, original, projection) = setup(namespace).await?;
    for id in 100..148 {
        original
            .write_local(
                p::ObjectKey {
                    collection: 1,
                    id: Uuid::from_u128(id),
                },
                Some(Note("neighbor".into())),
            )
            .await?;
    }
    let reads = Arc::new(AtomicUsize::new(0));
    let measured = reads.clone();
    db.set_metric_callback(move |info| {
        if info.statement.sql.starts_with("SELECT")
            && info.statement.sql.contains("\"sync_candidates\"")
        {
            measured.fetch_add(1, Ordering::SeqCst);
        }
    });
    let endpoint = CausalEndpoint::with_projection(
        CausalReplica::open(db, namespace, original.store().author()).await?,
        projection,
    );
    endpoint
        .write_local(
            p::ObjectKey {
                collection: 1,
                id: Uuid::from_u128(1),
            },
            Some(Note("before neighbors".into())),
        )
        .await?;
    let lower_reads = reads.swap(0, Ordering::SeqCst);
    endpoint
        .write_local(
            p::ObjectKey {
                collection: 1,
                id: Uuid::from_u128(u128::MAX),
            },
            Some(Note("after neighbors".into())),
        )
        .await?;
    let upper_reads = reads.load(Ordering::SeqCst);
    assert!(
        lower_reads > 0 && upper_reads > 0,
        "database metric callback was not active"
    );
    assert_eq!(
        lower_reads, upper_reads,
        "point assignment scanned unrelated candidates"
    );
    Ok(())
}

#[tokio::test]
async fn removed_prefix_without_live_candidates_survives_relay() -> Result<()> {
    let namespace = Uuid::new_v4();
    let (_, source, _) = setup(namespace).await?;
    let (db, target, _) = setup(namespace).await?;
    let (_, relay, _) = setup(namespace).await?;
    let key = p::ObjectKey {
        collection: 1,
        id: Uuid::from_u128(1),
    };
    source
        .write_local(key.clone(), Some(Note("removed".into())))
        .await?;
    source.write_local(key.clone(), None).await?;
    synchronize(&source, &target).await?;
    assert!(target.get(&key).await?.is_none());
    assert!(
        target.head().await?.sequence > 0,
        "prefix-only observation was discarded"
    );
    let tx = db.begin().await?;
    assert!(
        sync_orm::causal_observed::contains_on(
            &tx,
            namespace,
            b"notes",
            sync::causal::Dot::new(source.store().writer_incarnation(), 1)?
        )
        .await?
    );
    tx.commit().await?;
    synchronize(&target, &relay).await?;
    let stable = relay.head().await?;
    assert!(stable.sequence > 0);
    synchronize(&source, &relay).await?;
    assert_eq!(relay.head().await?, stable);
    Ok(())
}

#[tokio::test]
async fn preexisting_context_manifest_is_indexed_when_accepted() -> Result<()> {
    let namespace = Uuid::new_v4();
    let (_, source, _) = setup(namespace).await?;
    let (db, target, _) = setup(namespace).await?;
    // Establish an incremental checkpoint before any source assignments.
    synchronize(&source, &target).await?;
    let key = p::ObjectKey {
        collection: 1,
        id: Uuid::from_u128(1),
    };
    source
        .write_local(key.clone(), Some(Note("one".into())))
        .await?;
    let writer = source.store().writer_incarnation();
    let context = p::ExactContext {
        writers: vec![p::WriterContext {
            writer_incarnation: writer,
            prefix: 1,
            exceptions: vec![],
        }],
    };
    let body = p::canonical_context(&context)?;
    let (manifest, fragments) =
        sync_orm::causal_manifests::fragment_body(p::ManifestKind::Context, &body, 1, 1024 * 1024)?;
    let tx = db.begin().await?;
    // Valid immutable dependencies may arrive before any assertion uses them.
    sync_orm::causal_manifests::publish_on(
        &tx,
        namespace,
        &manifest,
        &fragments,
        p::MAX_LOGICAL_BYTES,
        |bytes| {
            p::decode_context(bytes)?;
            Ok(vec![])
        },
    )
    .await?;
    tx.commit().await?;
    synchronize(&source, &target).await?;
    let tx = db.begin().await?;
    assert!(
        sync_orm::causal_observed::contains_on(
            &tx,
            namespace,
            b"notes",
            sync::causal::Dot::new(writer, 1)?
        )
        .await?,
        "accepted observation is absent from the interval index"
    );
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
async fn relayed_snapshot_suppresses_obsolete_incremental_assignments() -> Result<()> {
    let namespace = Uuid::new_v4();
    let (_, a, _) = setup(namespace).await?;
    let (_, b, _) = setup(namespace).await?;
    let (_, c, _) = setup(namespace).await?;
    let key = p::ObjectKey {
        collection: 1,
        id: Uuid::from_u128(1),
    };
    a.write_local(key.clone(), Some(Note("common".into())))
        .await?;
    synchronize(&a, &b).await?;
    synchronize(&b, &c).await?;
    b.write_local(key.clone(), Some(Note("intermediate".into())))
        .await?;
    b.write_local(key.clone(), Some(Note("latest".into())))
        .await?;
    for id in 2..=5 {
        b.write_local(
            p::ObjectKey {
                collection: 1,
                id: Uuid::from_u128(id),
            },
            Some(Note(format!("unique-{id}"))),
        )
        .await?;
    }
    synchronize(&b, &a).await?;
    synchronize(&a, &c).await?;
    assert_eq!(
        c.get(&key).await?.unwrap().value,
        Some(Note("latest".into()))
    );
    let stable = c.head().await?;
    synchronize(&b, &c).await?;
    assert_eq!(
        c.head().await?,
        stable,
        "obsolete incremental history changed relayed state"
    );
    Ok(())
}

#[tokio::test]
async fn relay_graph_stability_survives_first_contact_between_leaves() -> Result<()> {
    let namespace = Uuid::new_v4();
    let mut peers = Vec::new();
    let mut databases = Vec::new();
    for _ in 0..8 {
        let (db, peer, _) = setup(namespace).await?;
        databases.push(db);
        peers.push(peer);
    }
    let key = p::ObjectKey {
        collection: 1,
        id: Uuid::from_u128(1),
    };
    peers[0]
        .write_local(key.clone(), Some(Note("common".into())))
        .await?;
    for i in 0..6 {
        synchronize(&peers[i], &peers[i + 1]).await?;
    }
    for (index, peer) in peers.iter().take(7).enumerate() {
        peer.write_local(key.clone(), Some(Note(format!("initial-{index}"))))
            .await?;
        peer.write_local(
            key.clone(),
            (index != 0).then(|| Note(format!("edit-{index}"))),
        )
        .await?;
        peer.write_local(
            p::ObjectKey {
                collection: 1,
                id: Uuid::from_u128(10 + index as u128),
            },
            Some(Note(format!("unique-{index}"))),
        )
        .await?;
    }
    fn edges(shape: usize) -> Vec<(usize, usize)> {
        match shape {
            0 => (0..7).map(|i| (i, i + 1)).collect(),
            1 => (0..8).map(|i| (i, (i + 1) % 8)).collect(),
            2 => (1..8).map(|i| (0, i)).collect(),
            _ => vec![
                (0, 1),
                (1, 2),
                (2, 3),
                (3, 0),
                (3, 4),
                (4, 5),
                (5, 6),
                (6, 4),
                (6, 7),
            ],
        }
    }
    async fn heads(peers: &[CausalEndpoint<Notes>]) -> Result<Vec<p::ReceiveProgress>> {
        let mut result = Vec::new();
        for peer in peers {
            result.push(peer.head().await?);
        }
        Ok(result)
    }
    let mut random = 4_u64;
    for step in 0..24 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let active = edges((2 + step / 8) % 4);
        let (a, b) = active[random as usize % active.len()];
        if a != 7 && b != 7 {
            let (a, b) = if random & 1 == 0 { (a, b) } else { (b, a) };
            synchronize(&peers[a], &peers[b]).await?;
        }
    }
    for stage in 0..3 {
        if stage > 0 {
            peers[0]
                .write_local(key.clone(), (stage == 2).then(|| Note("restored".into())))
                .await?;
        }
        for round in 0..40 {
            let before = heads(&peers).await?;
            for (a, b) in edges(2) {
                synchronize(&peers[a], &peers[b]).await?;
                synchronize(&peers[b], &peers[a]).await?;
            }
            if heads(&peers).await? == before {
                break;
            }
            ensure!(round < 39, "relay graph did not settle");
        }
    }
    let stable = heads(&peers).await?;
    for (index, db) in databases.iter().enumerate() {
        let tx = db.begin().await?;
        let mut missing = Vec::new();
        for (writer_index, peer) in peers.iter().take(7).enumerate() {
            if !sync_orm::causal_observed::contains_on(
                &tx,
                namespace,
                b"notes",
                sync::causal::Dot::new(peer.store().writer_incarnation(), 1)?,
            )
            .await?
            {
                missing.push(writer_index);
            }
        }
        assert!(
            missing.is_empty(),
            "settled peer {index} lacks observations from writers {missing:?}"
        );
        tx.commit().await?;
    }
    for a in 0..8 {
        for b in a + 1..8 {
            for (source, target) in [(a, b), (b, a)] {
                let before = peers[target].head().await?;
                synchronize(&peers[source], &peers[target]).await?;
                let after = peers[target].head().await?;
                if before != after {
                    let page = p::SyncStore::read_page(
                        &peers[target],
                        p::PageRequest {
                            source: peers[target].identity().await?,
                            scope: p::Scope {
                                collections: [1].into_iter().collect(),
                            },
                            after: before,
                            through: after,
                            limits: p::PageLimits {
                                max_scanned: 16,
                                max_bytes: 65536,
                            },
                        },
                    )
                    .await?;
                    anyhow::bail!("first contact {source}->{target} republished state: {page:?}");
                }
            }
        }
    }
    assert_eq!(heads(&peers).await?, stable);
    Ok(())
}

#[tokio::test]
async fn sql_coordinator_reconciles_split_ranges_then_journal_and_reaches_stability() -> Result<()>
{
    let namespace = Uuid::new_v4();
    let (_, source, _) = setup(namespace).await?;
    let (target_db, target, _) = setup(namespace).await?;
    let keys: Vec<_> = (1..=32)
        .map(|id| p::ObjectKey {
            collection: 1,
            id: Uuid::from_u128(id),
        })
        .collect();
    for (index, key) in keys.iter().enumerate() {
        source
            .write_local(key.clone(), Some(Note(format!("note {index}"))))
            .await?;
    }
    assert!(synchronize(&source, &target).await? > 32);
    for key in &keys {
        assert_eq!(
            source.get(key).await?.unwrap().value,
            target.get(key).await?.unwrap().value
        );
    }
    source.write_local(keys[0].clone(), None).await?;
    source
        .write_local(keys[1].clone(), Some(Note("changed".into())))
        .await?;
    synchronize(&source, &target).await?;
    assert!(
        target
            .get(&keys[0])
            .await?
            .is_none_or(|row| row.value.is_none())
    );
    assert_eq!(
        target.get(&keys[1]).await?.unwrap().value,
        Some(Note("changed".into()))
    );
    let head = target.head().await?;
    synchronize(&source, &target).await?;
    assert_eq!(target.head().await?, head);
    let count = target_db
        .query_one(
            &Query::select()
                .expr(sea_orm::sea_query::Func::count(Expr::val(1)))
                .from(Alias::new("notes"))
                .to_owned(),
        )
        .await?
        .unwrap()
        .try_get_by_index::<i64>(0)?;
    assert_eq!(count, 31);
    Ok(())
}

#[tokio::test]
async fn sql_snapshot_build_resumes_after_restart_and_cancels_after_lost_reply() -> Result<()> {
    use p::SnapshotSource;
    let namespace = Uuid::new_v4();
    let (db, endpoint, projection) = setup(namespace).await?;
    let key = p::ObjectKey {
        collection: 1,
        id: Uuid::from_u128(12),
    };
    endpoint
        .write_local(key.clone(), Some(Note("captured".into())))
        .await?;
    let scope = p::Scope {
        collections: [1].into_iter().collect(),
    };
    let mut work = session_limits().work;
    work.max_rows_scanned = 1;
    let request = Uuid::new_v4();
    let cursor = match endpoint
        .open_snapshot(request, scope.clone(), work, None)
        .await?
    {
        p::WorkOutcome::Incomplete { cursor } => cursor,
        _ => anyhow::bail!("expected resumable build"),
    };
    let author = endpoint.store().author();
    drop(endpoint);
    let endpoint = CausalEndpoint::with_projection(
        CausalReplica::open(db.clone(), namespace, author).await?,
        projection,
    );
    endpoint
        .write_local(key.clone(), Some(Note("later".into())))
        .await?;
    let mut resume = Some(cursor.clone());
    let mut completed = None;
    for _ in 0..2000 {
        match endpoint
            .open_snapshot(request, scope.clone(), work, resume)
            .await?
        {
            p::WorkOutcome::Incomplete { cursor } => resume = Some(cursor),
            p::WorkOutcome::Ready(descriptor) => {
                completed = Some(descriptor);
                break;
            }
        }
    }
    let descriptor = completed.context("snapshot did not finish")?;
    let page = endpoint
        .leaf(
            descriptor.snapshot_id,
            descriptor.roots[0].range.clone(),
            None,
            session_limits().pages,
        )
        .await?;
    assert_eq!(page.candidates[0].payload, Note("captured".into()));
    endpoint.release_request(request).await?;
    endpoint.release_request(request).await?;
    assert!(
        endpoint
            .leaf(
                descriptor.snapshot_id,
                descriptor.roots[0].range.clone(),
                None,
                session_limits().pages
            )
            .await
            .is_err()
    );

    let request = Uuid::new_v4();
    let cursor = match endpoint
        .open_snapshot(request, scope.clone(), work, None)
        .await?
    {
        p::WorkOutcome::Incomplete { cursor } => cursor,
        _ => anyhow::bail!("expected resumable build"),
    };
    let _lost_reply = endpoint
        .open_snapshot(request, scope, work, Some(cursor.clone()))
        .await?;
    let mut forged = cursor.clone();
    forged.generation = Uuid::new_v4();
    assert_eq!(
        endpoint.cancel_snapshot_build(forged).await,
        Err(p::SyncError::StaleWork)
    );
    endpoint.cancel_snapshot_build(cursor).await?;
    Ok(())
}
use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait, Database, DatabaseTransaction, ExprTrait, TransactionTrait,
    sea_query::{Alias, ColumnDef, Expr, OnConflict, Query, Table},
};
use serde::{Deserialize, Serialize};
use sync::protocol::{self as p, ReceiveStore, SnapshotSink, SyncStore};
use sync_orm::{
    causal_capabilities::{CausalEndpoint, TransactionProjection},
    causal_progress, causal_schema,
    causal_store::CausalReplica,
};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Notes;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Note(String);
impl p::SyncModel for Notes {
    type Collection = u8;
    type Payload = Note;
    fn model_id() -> &'static str {
        "endpoint-notes"
    }
    fn schema_id() -> &'static str {
        "1"
    }
    fn collections() -> Vec<u8> {
        vec![1]
    }
    fn collection_tag(_: &u8) -> &'static str {
        "notes"
    }
    fn collection_from_tag(value: &str) -> Option<u8> {
        (value == "notes").then_some(1)
    }
    fn validate(_: &p::ObjectKey<Self>, _: &Option<Note>) -> p::Result<()> {
        Ok(())
    }
}
#[derive(Default)]
struct Projection {
    fail: AtomicBool,
}
#[async_trait]
impl TransactionProjection<Notes> for Projection {
    fn mutation_ceiling(&self, changes: &p::ProjectionBatch<Notes>) -> Result<u64> {
        Ok(changes.changes.len() as u64)
    }
    async fn apply(
        &self,
        tx: &DatabaseTransaction,
        changes: &p::ProjectionBatch<Notes>,
    ) -> Result<()> {
        for change in &changes.changes {
            if let Some(candidate) = &change.after {
                tx.execute(
                    &Query::insert()
                        .into_table(Alias::new("notes"))
                        .columns(["id", "text"].map(Alias::new))
                        .values_panic([
                            change.key.id.as_bytes().to_vec().into(),
                            candidate.payload.0.clone().into(),
                        ])
                        .on_conflict(
                            OnConflict::column(Alias::new("id"))
                                .update_column(Alias::new("text"))
                                .to_owned(),
                        )
                        .to_owned(),
                )
                .await?;
            } else {
                tx.execute(
                    &Query::delete()
                        .from_table(Alias::new("notes"))
                        .and_where(
                            Expr::col(Alias::new("id")).eq(change.key.id.as_bytes().to_vec()),
                        )
                        .to_owned(),
                )
                .await?;
            }
        }
        ensure!(
            !self.fail.load(Ordering::SeqCst),
            "injected projection failure after source mutation"
        );
        Ok(())
    }
}
async fn setup(
    namespace: Uuid,
) -> Result<(
    sea_orm::DatabaseConnection,
    CausalEndpoint<Notes>,
    Arc<Projection>,
)> {
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    db.execute(
        &Table::create()
            .table(Alias::new("notes"))
            .col(ColumnDef::new(Alias::new("id")).binary().primary_key())
            .col(ColumnDef::new(Alias::new("text")).text().not_null())
            .to_owned(),
    )
    .await?;
    let store = CausalReplica::create_empty(db.clone(), namespace, Uuid::new_v4()).await?;
    let projection = Arc::new(Projection::default());
    Ok((
        db,
        CausalEndpoint::with_projection(store, projection.clone()),
        projection,
    ))
}
async fn text(db: &sea_orm::DatabaseConnection) -> Result<Option<String>> {
    Ok(db
        .query_one(
            &Query::select()
                .column(Alias::new("text"))
                .from(Alias::new("notes"))
                .to_owned(),
        )
        .await?
        .map(|row| row.try_get_by_index(0))
        .transpose()?)
}
fn limits() -> p::ApplyLimits {
    p::ApplyLimits {
        max_projection_items: 16,
        max_projection_bytes: 65536,
        max_mutations: 1024,
    }
}

#[tokio::test]
async fn endpoint_projects_lower_survivor_and_rolls_back_failed_absence() -> Result<()> {
    let (db, endpoint, projection) = setup(Uuid::new_v4()).await?;
    let key = p::ObjectKey {
        collection: 1,
        id: Uuid::new_v4(),
    };
    endpoint
        .write_local(key.clone(), Some(Note("lower".into())))
        .await?;
    endpoint
        .write_observed(
            key.clone(),
            Some(Note("higher".into())),
            p::Observed::Exact(Default::default()),
        )
        .await?;
    let candidates = endpoint
        .candidates(
            &key,
            None,
            p::PageLimits {
                max_scanned: 8,
                max_bytes: 65536,
            },
        )
        .await?;
    assert_eq!(candidates.values.len(), 2);
    let higher = candidates
        .values
        .iter()
        .find(|candidate| candidate.payload.0 == "higher")
        .unwrap()
        .dot;
    endpoint
        .write_observed(
            key.clone(),
            None,
            p::Observed::Exact([higher].into_iter().collect()),
        )
        .await?;
    assert_eq!(text(&db).await?.as_deref(), Some("lower"));
    let head = endpoint.head().await?;
    projection.fail.store(true, Ordering::SeqCst);
    assert!(endpoint.write_local(key.clone(), None).await.is_err());
    assert_eq!(endpoint.head().await?, head);
    assert_eq!(
        endpoint.get(&key).await?.unwrap().value,
        Some(Note("lower".into()))
    );
    assert_eq!(text(&db).await?.as_deref(), Some("lower"));
    projection.fail.store(false, Ordering::SeqCst);
    endpoint.write_local(key.clone(), None).await?;
    assert!(endpoint.get(&key).await?.is_none());
    assert!(text(&db).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn failed_receive_projection_preserves_checkpoint_completion_and_history() -> Result<()> {
    let namespace = Uuid::new_v4();
    let (db, endpoint, projection) = setup(namespace).await?;
    let source = p::MemorySyncStore::<Notes>::new(
        p::StoreIdentity {
            namespace,
            model: p::ModelId {
                model: "endpoint-notes".into(),
                schema: "1".into(),
            },
            replica: Uuid::new_v4(),
            generation: Uuid::new_v4(),
        },
        p::LocalAuthor {
            author: Uuid::new_v4(),
            writer_incarnation: Uuid::new_v4(),
        },
    )?;
    let scope = p::Scope {
        collections: [1].into_iter().collect(),
    };
    let identity = source.identity().await?;
    let key = p::CheckpointKey {
        namespace,
        source_replica: identity.replica,
        source_generation: identity.generation,
        scope_id: p::scope_id(&scope)?,
    };
    let genesis = source.head().await?;
    let local_identity = endpoint.identity().await?;
    let tx = db.begin().await?;
    causal_progress::initialize_checkpoint_on(&tx, &local_identity, &key, &scope, &genesis).await?;
    tx.commit().await?;
    let object = p::ObjectKey {
        collection: 1,
        id: Uuid::new_v4(),
    };
    source
        .write_local(object.clone(), Some(Note("received".into())))
        .await?;
    let p::PageRead::Page(page) = source
        .read_page(p::PageRequest {
            source: identity,
            scope,
            after: genesis.clone(),
            through: source.head().await?,
            limits: p::PageLimits {
                max_scanned: 8,
                max_bytes: 65536,
            },
        })
        .await?
    else {
        anyhow::bail!("small event fragmented")
    };
    let local_head = endpoint.head().await?;
    let completed_query = Query::select()
        .column(Alias::new("completed_at"))
        .from(Alias::new("sync_checkpoints"))
        .to_owned();
    let before = db
        .query_one(&completed_query)
        .await?
        .unwrap()
        .try_get_by_index::<String>(0)?;
    projection.fail.store(true, Ordering::SeqCst);
    assert!(endpoint.apply_page(&page, limits()).await.is_err());
    assert_eq!(endpoint.head().await?, local_head);
    assert_eq!(endpoint.checkpoint(&key).await?, Some(genesis));
    assert_eq!(
        db.query_one(&completed_query)
            .await?
            .unwrap()
            .try_get_by_index::<String>(0)?,
        before
    );
    assert!(text(&db).await?.is_none());
    assert!(endpoint.get(&object).await?.is_none());
    projection.fail.store(false, Ordering::SeqCst);
    endpoint.apply_page(&page, limits()).await?;
    assert_eq!(text(&db).await?.as_deref(), Some("received"));
    assert_eq!(endpoint.checkpoint(&key).await?, Some(page.next.clone()));
    let committed = endpoint.head().await?;
    endpoint.apply_page(&page, limits()).await?;
    assert_eq!(endpoint.head().await?, committed);
    Ok(())
}

#[tokio::test]
async fn committed_range_survives_receiver_restart_before_completion() -> Result<()> {
    let namespace = Uuid::new_v4();
    let (db, endpoint, projection) = setup(namespace).await?;
    let source = p::MemorySyncStore::<Notes>::new(
        p::StoreIdentity {
            namespace,
            model: p::ModelId {
                model: "endpoint-notes".into(),
                schema: "1".into(),
            },
            replica: Uuid::new_v4(),
            generation: Uuid::new_v4(),
        },
        p::LocalAuthor {
            author: Uuid::new_v4(),
            writer_incarnation: Uuid::new_v4(),
        },
    )?;
    let key = p::ObjectKey {
        collection: 1,
        id: Uuid::new_v4(),
    };
    source
        .write_local(key.clone(), Some(Note("accepted range".into())))
        .await?;
    let scope = p::Scope {
        collections: [1].into_iter().collect(),
    };
    let work = p::WorkLimits {
        max_requests: 1024,
        max_bytes: 1024 * 1024,
        max_depth: 32,
        max_rows_scanned: 1024,
        max_mutations: 65536,
    };
    let request = Uuid::new_v4();
    let mut resume = None;
    let remote = loop {
        match p::SnapshotSource::open_snapshot(&source, request, scope.clone(), work, resume)
            .await?
        {
            p::WorkOutcome::Ready(descriptor) => break descriptor,
            p::WorkOutcome::Incomplete { cursor } => resume = Some(cursor),
        }
    };
    let local_identity = endpoint.identity().await?;
    let tx = db.begin().await?;
    let local = sync_orm::causal_snapshot::open_bounded_on(
        &tx,
        local_identity,
        scope.clone(),
        chrono::Utc::now().timestamp_millis(),
        600000,
        work,
    )
    .await?;
    tx.commit().await?;
    let session = endpoint
        .begin_reconciliation(Uuid::new_v4(), remote.clone(), local)
        .await?;
    let range = remote.roots[0].range.clone();
    let pages = p::PageLimits {
        max_scanned: 128,
        max_bytes: 65536,
    };
    for context in &remote.contexts {
        let manifest = context.context.manifest.clone();
        let stage = endpoint
            .begin_stage(
                p::StageDescriptor {
                    origin: p::StageOrigin::SnapshotContext {
                        session,
                        snapshot_id: remote.snapshot_id,
                        collection: context.collection,
                    },
                    manifest: manifest.clone(),
                },
                session_limits().stages,
            )
            .await?;
        for ordinal in 0..manifest.fragment_count {
            let fragment = p::SnapshotSource::read_fragment(
                &source,
                remote.snapshot_id,
                manifest.clone(),
                ordinal,
                pages.max_bytes,
            )
            .await?;
            endpoint.stage_fragment(stage.stage, fragment).await?;
        }
    }
    let mut after = None;
    loop {
        let page =
            p::SnapshotSource::leaf(&source, remote.snapshot_id, range.clone(), after, pages)
                .await?;
        endpoint.stage_leaf_page(session, page.clone()).await?;
        if page.done {
            break;
        }
        after = page.next;
    }
    projection.fail.store(true, Ordering::SeqCst);
    assert!(
        endpoint
            .commit_complete_range(session, range.clone(), limits())
            .await
            .is_err()
    );
    assert!(endpoint.get(&key).await?.is_none());
    assert!(text(&db).await?.is_none());
    projection.fail.store(false, Ordering::SeqCst);
    let (coverage, changed) = endpoint
        .commit_complete_range(session, range.clone(), limits())
        .await?;
    assert!(coverage.complete);
    assert_eq!(changed.changes.len(), 1);
    db.execute_unprepared("CREATE TRIGGER reject_completion BEFORE INSERT ON sync_checkpoints BEGIN SELECT RAISE(ABORT, 'completion fault'); END").await?;
    while !endpoint.promote_step(session, limits()).await?.done {}
    assert!(endpoint.complete_reconciliation(session).await.is_err());
    assert!(endpoint.get(&key).await?.is_some());
    db.execute_unprepared("DROP TRIGGER reject_completion")
        .await?;

    let committed_head = endpoint.head().await?;
    let retry = endpoint
        .commit_complete_range(session, range, limits())
        .await?;
    assert!(retry.1.changes.is_empty());
    assert_eq!(endpoint.head().await?, committed_head);
    let checkpoint = p::CheckpointKey {
        namespace,
        source_replica: remote.source.replica,
        source_generation: remote.source.generation,
        scope_id: p::scope_id(&scope)?,
    };
    assert_eq!(endpoint.checkpoint(&checkpoint).await?, None);
    let author = endpoint.store().author();
    drop(endpoint);
    let reopened = CausalEndpoint::with_projection(
        CausalReplica::open(db.clone(), namespace, author).await?,
        projection,
    );
    assert_eq!(
        reopened.get(&key).await?.unwrap().value,
        Some(Note("accepted range".into()))
    );
    assert_eq!(text(&db).await?.as_deref(), Some("accepted range"));
    assert_eq!(reopened.head().await?, committed_head);
    assert_eq!(reopened.checkpoint(&checkpoint).await?, None);
    assert_eq!(
        reopened.complete_reconciliation(session).await,
        Err(p::SyncError::StaleWork)
    );
    Ok(())
}

#[tokio::test]
async fn floor_respects_pins_and_commits_atomically() -> Result<()> {
    use sync_orm::causal_compaction::{FloorStep, maintenance_step_on};
    let ns = Uuid::new_v4();
    let (db, source, _) = setup(ns).await?;
    let key = p::ObjectKey {
        collection: 1,
        id: Uuid::from_u128(1),
    };
    source
        .write_local(key.clone(), Some(Note("old".into())))
        .await?;
    let pin = Uuid::new_v4();
    let tx = db.begin().await?;
    sync_orm::causal_leases::create_build_on(
        &tx,
        sync_orm::causal_leases::BuildLeaseRequest {
            namespace: ns,
            id: pin,
            scope: vec![1],
            now_ms: 0,
            expires_at_ms: 100,
            max_active: 4,
            max_ttl_ms: 1000,
        },
    )
    .await?;
    tx.commit().await?;
    source.write_local(key, None).await?;
    let tx = db.begin().await?;
    assert_eq!(
        maintenance_step_on::<Notes>(&tx, ns, 64, 64, 65536).await?,
        FloorStep::Advanced { state_seq: 1 }
    );
    tx.commit().await?;
    let tx = db.begin().await?;
    assert_eq!(
        maintenance_step_on::<Notes>(&tx, ns, 64, 64, 65536).await?,
        FloorStep::Pinned
    );
    sync_orm::causal_leases::revoke_on(&tx, ns, pin).await?;
    tx.commit().await?;
    db.execute_unprepared("CREATE TRIGGER reject_floor BEFORE UPDATE OF history_floor ON sync_meta WHEN NEW.history_floor != OLD.history_floor BEGIN SELECT RAISE(ABORT, 'floor fault'); END").await?;
    let tx = db.begin().await?;
    assert!(
        maintenance_step_on::<Notes>(&tx, ns, 64, 64, 65536)
            .await
            .is_err()
    );
    tx.rollback().await?;
    db.execute_unprepared("DROP TRIGGER reject_floor").await?;
    let tx = db.begin().await?;
    assert_eq!(
        maintenance_step_on::<Notes>(&tx, ns, 64, 64, 65536).await?,
        FloorStep::Advanced { state_seq: 2 }
    );
    tx.commit().await?;
    assert_eq!(source.head().await?.sequence, 2);
    Ok(())
}

#[tokio::test]
async fn maintenance_recovers_abandoned_and_competing_builds_without_reviving_removals()
-> Result<()> {
    let namespace = Uuid::new_v4();
    let (db, source, _) = setup(namespace).await?;
    let (_, stale, _) = setup(namespace).await?;
    let removed = p::ObjectKey {
        collection: 1,
        id: Uuid::from_u128(1),
    };
    source
        .write_local(removed.clone(), Some(Note("remove after relay".into())))
        .await?;
    synchronize(&source, &stale).await?;
    source.write_local(removed.clone(), None).await?;
    for index in 2..=24 {
        source
            .write_local(
                p::ObjectKey {
                    collection: 1,
                    id: Uuid::from_u128(index),
                },
                Some(Note(format!("retained {index}"))),
            )
            .await?;
    }
    let expected_head = source.head().await?;
    let identity = source.identity().await?;
    let mut now = chrono::Utc::now().timestamp_millis();
    let mut first = sync_orm::maintenance::Maintenance::default();
    first.advance::<Notes>(&db, identity.clone(), now).await?;
    first.advance::<Notes>(&db, identity.clone(), now).await?;
    // A process restart loses the RAM cursor; its persisted build expires.
    first = sync_orm::maintenance::Maintenance::default();
    now += p::DEFAULT_SNAPSHOT_TTL_MS as i64 + 1;
    let mut second = sync_orm::maintenance::Maintenance::default();
    let mut complete = false;
    for _ in 0..2000 {
        first.advance::<Notes>(&db, identity.clone(), now).await?;
        second.advance::<Notes>(&db, identity.clone(), now).await?;
        let row = db
            .query_one(
                &Query::select()
                    .columns(["head", "history_floor"].map(Alias::new))
                    .from(Alias::new("sync_meta"))
                    .to_owned(),
            )
            .await?
            .context("namespace missing")?;
        let floor = row.try_get_by_index::<i64>(1)?;
        let count = db
            .query_one(
                &Query::select()
                    .expr(sea_orm::sea_query::Func::count(Expr::val(1)))
                    .from(Alias::new("sync_journal"))
                    .to_owned(),
            )
            .await?
            .unwrap()
            .try_get_by_index::<i64>(0)?;
        if floor == row.try_get_by_index::<i64>(0)? && count == 0 {
            complete = true;
            break;
        }
    }
    if !complete {
        let meta = db
            .query_one(
                &Query::select()
                    .columns(
                        ["head", "history_floor", "state_seq", "floor_state_seq"].map(Alias::new),
                    )
                    .from(Alias::new("sync_meta"))
                    .to_owned(),
            )
            .await?
            .unwrap();
        eprintln!(
            "maintenance meta {:?}",
            (0..4)
                .map(|i| meta.try_get_by_index::<i64>(i))
                .collect::<Vec<_>>()
        );
        for lease in db
            .query_all(
                &Query::select()
                    .columns(["status", "state_seq", "head", "expires_at_ms"].map(Alias::new))
                    .from(Alias::new("sync_leases"))
                    .to_owned(),
            )
            .await?
        {
            eprintln!(
                "maintenance lease {:?}",
                (0..4)
                    .map(|i| lease.try_get_by_index::<i64>(i))
                    .collect::<Vec<_>>()
            );
        }
    }
    ensure!(
        complete,
        "maintenance remained pinned or stuck on an obsolete continuation"
    );
    for _ in 0..20 {
        first.advance::<Notes>(&db, identity.clone(), now).await?;
        second.advance::<Notes>(&db, identity.clone(), now).await?;
    }
    assert_eq!(source.head().await?, expected_head);
    synchronize(&source, &stale).await?;
    assert!(stale.get(&removed).await?.is_none());
    synchronize(&stale, &source).await?;
    assert!(source.get(&removed).await?.is_none());
    let stable = source.head().await?;
    synchronize(&stale, &source).await?;
    assert_eq!(source.head().await?, stable);
    Ok(())
}

struct CountingSource<'a> {
    inner: &'a CausalEndpoint<Notes>,
    calls: AtomicUsize,
    candidates: AtomicUsize,
}
impl p::SnapshotSource<Notes> for CountingSource<'_> {
    fn release_request(&self, id: Uuid) -> p::BoxFuture<'_, ()> {
        p::SnapshotSource::release_request(self.inner, id)
    }
    fn close_snapshot(&self, id: Uuid) -> p::BoxFuture<'_, ()> {
        p::SnapshotSource::close_snapshot(self.inner, id)
    }
    fn cancel_snapshot_build(&self, cursor: p::WorkCursor) -> p::BoxFuture<'_, ()> {
        p::SnapshotSource::cancel_snapshot_build(self.inner, cursor)
    }
    fn open_snapshot(
        &self,
        request: Uuid,
        scope: p::Scope<Notes>,
        limits: p::WorkLimits,
        resume: Option<p::WorkCursor>,
    ) -> p::BoxFuture<'_, p::WorkOutcome<p::SnapshotDescriptor<Notes>>> {
        p::SnapshotSource::open_snapshot(self.inner, request, scope, limits, resume)
    }
    fn summarize(
        &self,
        snapshot: Uuid,
        range: p::KeyRange<Notes>,
        limits: p::WorkLimits,
        resume: Option<p::WorkCursor>,
    ) -> p::BoxFuture<'_, p::WorkOutcome<p::RangeSummary<Notes>>> {
        p::SnapshotSource::summarize(self.inner, snapshot, range, limits, resume)
    }
    fn split(
        &self,
        snapshot: Uuid,
        range: p::KeyRange<Notes>,
        limits: p::WorkLimits,
        resume: Option<p::WorkCursor>,
    ) -> p::BoxFuture<'_, p::WorkOutcome<Vec<p::KeyRange<Notes>>>> {
        p::SnapshotSource::split(self.inner, snapshot, range, limits, resume)
    }
    fn leaf(
        &self,
        snapshot: Uuid,
        range: p::KeyRange<Notes>,
        after: Option<p::CandidateCursor<Notes>>,
        limits: p::PageLimits,
    ) -> p::BoxFuture<'_, p::LeafPage<Notes>> {
        Box::pin(async move {
            let page = p::SnapshotSource::leaf(self.inner, snapshot, range, after, limits).await?;
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.candidates
                .fetch_add(page.candidates.len(), Ordering::SeqCst);
            Ok(page)
        })
    }
    fn read_fragment(
        &self,
        snapshot: Uuid,
        manifest: p::ManifestRef,
        ordinal: u32,
        max_bytes: u64,
    ) -> p::BoxFuture<'_, p::Fragment> {
        p::SnapshotSource::read_fragment(self.inner, snapshot, manifest, ordinal, max_bytes)
    }
}

#[tokio::test]
async fn one_write_snapshot_transfers_only_changed_leaf() -> Result<()> {
    let ns = Uuid::new_v4();
    let (_, a, _) = setup(ns).await?;
    let (_, b, _) = setup(ns).await?;
    let (_, c, _) = setup(ns).await?;
    let key = |n| p::ObjectKey {
        collection: 1,
        id: Uuid::from_u128(n),
    };
    for n in 1..=1024 {
        a.write_local(key(n), Some(Note(n.to_string()))).await?;
    }
    let mut limits = session_limits();
    limits.pages.max_scanned = 128;
    limits.apply.max_projection_items = 128;
    limits.apply.max_mutations = 4096;
    limits.work.max_rows_scanned = 1024;
    limits.work.max_mutations = 4096;
    async fn run(
        dst: &CausalEndpoint<Notes>,
        src: &CausalEndpoint<Notes>,
        source: &dyn p::SnapshotSource<Notes>,
        limits: p::SessionLimits,
    ) -> Result<()> {
        let mut coordinator = p::Coordinator::new(
            p::Scope {
                collections: [1].into_iter().collect(),
            },
            limits,
        )?;
        for _ in 0..20000 {
            match coordinator.advance(dst, dst, dst, src, source).await? {
                p::SessionAdvance::Complete(_) => return Ok(()),
                p::SessionAdvance::BudgetExhausted => coordinator.start_pass(),
                _ => (),
            }
        }
        anyhow::bail!("snapshot session did not finish")
    }
    run(&c, &a, &a, limits).await?;
    run(&b, &c, &c, limits).await?;
    a.write_local(key(500), Some(Note("9999".into()))).await?;
    let head = b.head().await?.sequence;
    let counting = CountingSource {
        inner: &a,
        calls: 0.into(),
        candidates: 0.into(),
    };
    run(&b, &a, &counting, limits).await?;
    let calls = counting.calls.load(Ordering::SeqCst);
    let candidates = counting.candidates.load(Ordering::SeqCst);
    let growth = b.head().await?.sequence - head;
    eprintln!("SQL leaf calls: {calls}, candidates: {candidates}, head growth: {growth}");
    assert_eq!(
        b.get(&key(500)).await?.unwrap().value,
        Some(Note("9999".into()))
    );
    assert!(calls <= 4 && candidates <= 256 && growth <= 2);
    Ok(())
}

async fn open_sql(endpoint: &CausalEndpoint<Notes>) -> Result<p::SnapshotDescriptor<Notes>> {
    use p::SnapshotSource;
    let request = Uuid::new_v4();
    let mut resume = None;
    loop {
        match endpoint
            .open_snapshot(
                request,
                p::Scope {
                    collections: [1].into_iter().collect(),
                },
                session_limits().work,
                resume,
            )
            .await?
        {
            p::WorkOutcome::Ready(descriptor) => return Ok(descriptor),
            p::WorkOutcome::Incomplete { cursor } => resume = Some(cursor),
        }
    }
}
async fn stage_sql_contexts(
    source: &CausalEndpoint<Notes>,
    target: &CausalEndpoint<Notes>,
    session: Uuid,
    remote: &p::SnapshotDescriptor<Notes>,
) -> Result<()> {
    for context in &remote.contexts {
        let manifest = context.context.manifest.clone();
        let stage = target
            .begin_stage(
                p::StageDescriptor {
                    origin: p::StageOrigin::SnapshotContext {
                        session,
                        snapshot_id: remote.snapshot_id,
                        collection: context.collection,
                    },
                    manifest: manifest.clone(),
                },
                session_limits().stages,
            )
            .await?;
        for ordinal in 0..manifest.fragment_count {
            let fragment = p::SnapshotSource::read_fragment(
                source,
                remote.snapshot_id,
                manifest.clone(),
                ordinal,
                p::DEFAULT_PAGE_BYTES,
            )
            .await?;
            target.stage_fragment(stage.stage, fragment).await?;
        }
    }
    Ok(())
}
#[tokio::test]
async fn post_snapshot_arrival_is_removed_by_sql_promotion() -> Result<()> {
    let ns = Uuid::new_v4();
    let (_, a, _) = setup(ns).await?;
    let (_, b, _) = setup(ns).await?;
    let (_, stale, _) = setup(ns).await?;
    let key = p::ObjectKey {
        collection: 1,
        id: Uuid::from_u128(42),
    };
    a.write_local(key.clone(), Some(Note("dead at source".into())))
        .await?;
    synchronize(&a, &stale).await?;
    a.write_local(key.clone(), None).await?;
    let remote = open_sql(&a).await?;
    let local = open_sql(&b).await?;
    let session = b
        .begin_reconciliation(Uuid::new_v4(), remote.clone(), local)
        .await?;
    stage_sql_contexts(&a, &b, session, &remote).await?;
    b.accept_equal_range(session, remote.roots[0].clone())
        .await?;
    synchronize(&stale, &b).await?;
    assert!(b.get(&key).await?.is_some());
    while !b.promote_step(session, limits()).await?.done {}
    assert!(b.get(&key).await?.is_none());
    assert_eq!(b.complete_reconciliation(session).await?, remote.head);
    Ok(())
}
#[tokio::test]
async fn sql_promotion_chunks_resume_and_are_idempotent() -> Result<()> {
    let ns = Uuid::new_v4();
    let (db, a, _) = setup(ns).await?;
    let (_, b, _) = setup(ns).await?;
    let writer = Uuid::new_v4();
    let mut context = sync::causal::Context::default();
    for n in 1..=3000 {
        context.insert(sync::causal::Dot::new(writer, 2 * n)?)?;
    }
    let mut apply = limits();
    apply.max_mutations = 16384;
    let tx = db.begin().await?;
    a.store()
        .apply_merge_on(
            &tx,
            &p::MergeDelta {
                collection: 1,
                added: vec![],
                observed: p::exact_context(&context),
            },
            apply,
            &mut 16384,
        )
        .await?;
    tx.commit().await?;
    let remote = open_sql(&a).await?;
    let local = open_sql(&b).await?;
    let session = b
        .begin_reconciliation(Uuid::new_v4(), remote.clone(), local)
        .await?;
    stage_sql_contexts(&a, &b, session, &remote).await?;
    b.accept_equal_range(session, remote.roots[0].clone())
        .await?;
    apply.max_mutations = 400;
    let mut steps = 0;
    loop {
        let progress = b.promote_step(session, apply).await?;
        steps += 1;
        if progress.done {
            break;
        }
        assert!(matches!(
            b.complete_reconciliation(session).await,
            Err(p::SyncError::IncompleteCoverage)
        ));
    }
    assert!(steps >= 31);
    let head = b.head().await?;
    assert_eq!(
        b.promote_step(session, apply).await?,
        p::PromoteProgress {
            done: true,
            causal_changes: 0
        }
    );
    assert_eq!(b.head().await?, head);
    assert_eq!(b.complete_reconciliation(session).await?, remote.head);
    let snapshot = open_sql(&b).await?;
    assert_eq!(snapshot.contexts, remote.contexts);
    Ok(())
}

async fn merge_sql_root(
    source: &CausalEndpoint<Notes>,
    target: &CausalEndpoint<Notes>,
    session: Uuid,
    remote: &p::SnapshotDescriptor<Notes>,
) -> Result<()> {
    let range = remote.roots[0].range.clone();
    let mut after = None;
    loop {
        let page = p::SnapshotSource::leaf(
            source,
            remote.snapshot_id,
            range.clone(),
            after,
            session_limits().pages,
        )
        .await?;
        target.stage_leaf_page(session, page.clone()).await?;
        if page.done {
            break;
        }
        after = page.next;
    }
    target
        .commit_complete_range(session, range, limits())
        .await?;
    Ok(())
}

#[tokio::test]
async fn sql_live_writes_and_journal_arrivals_survive_promotion() -> Result<()> {
    let ns = Uuid::new_v4();
    let (_, a, _) = setup(ns).await?;
    let (_, b, _) = setup(ns).await?;
    let key = |id| p::ObjectKey {
        collection: 1,
        id: Uuid::from_u128(id),
    };
    a.write_local(key(42), Some(Note("original".into())))
        .await?;
    a.write_local(key(43), Some(Note("remove".into()))).await?;
    synchronize(&a, &b).await?;
    let remote = open_sql(&a).await?;
    let local = open_sql(&b).await?;
    let session = b
        .begin_reconciliation(Uuid::new_v4(), remote.clone(), local)
        .await?;
    stage_sql_contexts(&a, &b, session, &remote).await?;
    merge_sql_root(&a, &b, session, &remote).await?;
    b.write_local(key(44), Some(Note("local new".into())))
        .await?;
    b.write_local(key(42), Some(Note("local update".into())))
        .await?;
    a.write_local(key(45), Some(Note("source new".into())))
        .await?;
    a.write_local(key(43), None).await?;
    synchronize(&a, &b).await?;
    while !b.promote_step(session, limits()).await?.done {}
    assert!(matches!(
        b.complete_reconciliation(session).await,
        Err(p::SyncError::RetryAt(_))
    ));
    for (id, value) in [
        (42, Some("local update")),
        (43, None),
        (44, Some("local new")),
        (45, Some("source new")),
    ] {
        assert_eq!(
            b.get(&key(id)).await?.and_then(|r| r.value).map(|v| v.0),
            value.map(str::to_owned)
        );
    }
    synchronize(&b, &a).await?;
    let heads = (a.head().await?, b.head().await?);
    synchronize(&a, &b).await?;
    synchronize(&b, &a).await?;
    assert_eq!((a.head().await?, b.head().await?), heads);
    Ok(())
}

async fn causal_rows(db: &sea_orm::DatabaseConnection, table: &str) -> Result<Vec<Vec<String>>> {
    let (columns, binary, nullable): (&[&str], &[usize], &[usize]) = match table {
        "sync_observed" => (
            &[
                "namespace",
                "collection",
                "writer",
                "first",
                "last",
                "added_seq",
                "removed_seq",
            ],
            &[0, 1, 2],
            &[6],
        ),
        "sync_candidates" => (
            &[
                "namespace",
                "collection",
                "writer",
                "counter",
                "object",
                "author",
                "revision",
                "body",
                "digest",
                "retention_seq",
                "live_from",
                "live_until",
            ],
            &[0, 1, 2, 4, 5, 7, 8],
            &[10, 11],
        ),
        _ => anyhow::bail!("unexpected table"),
    };
    let rows = db
        .query_all(
            &Query::select()
                .columns(columns.iter().map(|name| Alias::new(*name)))
                .from(Alias::new(table))
                .to_owned(),
        )
        .await?;
    let mut values = Vec::new();
    for row in rows {
        let mut fields = Vec::new();
        for index in 0..columns.len() {
            fields.push(if binary.contains(&index) {
                format!("{:?}", row.try_get_by_index::<Vec<u8>>(index)?)
            } else if nullable.contains(&index) {
                format!("{:?}", row.try_get_by_index::<Option<i64>>(index)?)
            } else {
                format!("{:?}", row.try_get_by_index::<i64>(index)?)
            });
        }
        values.push(fields);
    }
    values.sort();
    Ok(values)
}

#[tokio::test]
async fn sql_promotion_failure_rolls_back_and_reopens() -> Result<()> {
    let ns = Uuid::new_v4();
    let (_, a, _) = setup(ns).await?;
    let (db, b, projection) = setup(ns).await?;
    let (_, stale, _) = setup(ns).await?;
    let keys: Vec<_> = (1..=8)
        .map(|id| p::ObjectKey {
            collection: 1,
            id: Uuid::from_u128(id),
        })
        .collect();
    for key in &keys {
        a.write_local(key.clone(), Some(Note("old".into()))).await?;
    }
    synchronize(&a, &stale).await?;
    for key in &keys {
        a.write_local(key.clone(), None).await?;
    }
    let remote = open_sql(&a).await?;
    let local = open_sql(&b).await?;
    let session = b
        .begin_reconciliation(Uuid::new_v4(), remote.clone(), local)
        .await?;
    stage_sql_contexts(&a, &b, session, &remote).await?;
    b.accept_equal_range(session, remote.roots[0].clone())
        .await?;
    let mut apply = limits();
    apply.max_mutations = 400;
    let first = b.promote_step(session, apply).await?;
    assert!(!first.done);
    synchronize(&stale, &b).await?;
    let head = b.head().await?;
    let observed = causal_rows(&db, "sync_observed").await?;
    let candidates = causal_rows(&db, "sync_candidates").await?;
    projection.fail.store(true, Ordering::SeqCst);
    let failure = b.promote_step(session, apply).await.unwrap_err();
    assert!(
        matches!(failure, p::SyncError::Storage(ref message) if message.contains("injected projection failure"))
    );
    assert_eq!(b.head().await?, head);
    assert_eq!(causal_rows(&db, "sync_observed").await?, observed);
    assert_eq!(causal_rows(&db, "sync_candidates").await?, candidates);
    let author = b.store().author();
    drop(b);
    projection.fail.store(false, Ordering::SeqCst);
    let b = CausalEndpoint::with_projection(CausalReplica::open(db, ns, author).await?, projection);
    let fresh = open_sql(&a).await?;
    let local = open_sql(&b).await?;
    let session = b
        .begin_reconciliation(Uuid::new_v4(), fresh.clone(), local)
        .await?;
    stage_sql_contexts(&a, &b, session, &fresh).await?;
    merge_sql_root(&a, &b, session, &fresh).await?;
    while !b.promote_step(session, apply).await?.done {}
    assert_eq!(b.complete_reconciliation(session).await?, fresh.head);
    for key in &keys {
        assert_eq!(
            a.get(key).await?.and_then(|r| r.value),
            b.get(key).await?.and_then(|r| r.value)
        );
    }
    Ok(())
}

#[tokio::test]
async fn expired_sql_snapshot_recovers_after_partial_reconciliation() -> Result<()> {
    let ns = Uuid::new_v4();
    let (db, source, _) = setup(ns).await?;
    let (_, target, _) = setup(ns).await?;
    let key = |id| p::ObjectKey {
        collection: 1,
        id: Uuid::from_u128(id),
    };
    for id in 1..=32 {
        source
            .write_local(key(id), Some(Note(format!("{id}"))))
            .await?;
    }
    let remote = open_sql(&source).await?;
    let local = open_sql(&target).await?;
    let session = target
        .begin_reconciliation(Uuid::new_v4(), remote.clone(), local)
        .await?;
    stage_sql_contexts(&source, &target, session, &remote).await?;
    let range = remote.roots[0].range.clone();
    let mut cursor = None;
    let children = loop {
        match p::SnapshotSource::split(
            &source,
            remote.snapshot_id,
            range.clone(),
            session_limits().work,
            cursor,
        )
        .await?
        {
            p::WorkOutcome::Ready(children) => break children,
            p::WorkOutcome::Incomplete { cursor: next } => cursor = Some(next),
        }
    };
    assert_eq!(children.len(), 2);
    let mut summaries = Vec::new();
    for child in &children {
        let mut cursor = None;
        loop {
            match p::SnapshotSource::summarize(
                &source,
                remote.snapshot_id,
                child.clone(),
                session_limits().work,
                cursor,
            )
            .await?
            {
                p::WorkOutcome::Ready(summary) => {
                    summaries.push(summary);
                    break;
                }
                p::WorkOutcome::Incomplete { cursor: next } => cursor = Some(next),
            }
        }
    }
    target
        .split_range(session, range.clone(), summaries)
        .await?;
    let mut after = None;
    loop {
        let page = p::SnapshotSource::leaf(
            &source,
            remote.snapshot_id,
            children[0].clone(),
            after,
            session_limits().pages,
        )
        .await?;
        target.stage_leaf_page(session, page.clone()).await?;
        if page.done {
            break;
        }
        after = page.next;
    }
    let mut apply = limits();
    apply.max_projection_items = 64;
    apply.max_mutations = 65536;
    let (coverage, _) = target
        .commit_complete_range(session, children[0].clone(), apply)
        .await?;
    assert!(!coverage.complete);
    let now = chrono::Utc::now().timestamp_millis() + p::DEFAULT_SNAPSHOT_TTL_MS as i64 + 1;
    let mut maintenance = sync_orm::maintenance::Maintenance::default();
    for _ in 0..100 {
        maintenance
            .advance::<Notes>(&db, source.identity().await?, now)
            .await?;
    }
    source.write_local(key(1), None).await?;
    source
        .write_local(key(33), Some(Note("new".into())))
        .await?;
    let expired = p::SnapshotSource::leaf(
        &source,
        remote.snapshot_id,
        range,
        None,
        session_limits().pages,
    )
    .await;
    assert!(
        matches!(expired, Err(p::SyncError::SnapshotExpired)),
        "{expired:?}"
    );
    synchronize(&source, &target).await?;
    for id in 1..=33 {
        assert_eq!(
            source.get(&key(id)).await?.and_then(|r| r.value),
            target.get(&key(id)).await?.and_then(|r| r.value)
        );
    }
    for _ in 0..100 {
        maintenance
            .advance::<Notes>(&db, source.identity().await?, now)
            .await?;
    }
    let rows = db
        .query_all(
            &Query::select()
                .column(Alias::new("id"))
                .from(Alias::new("sync_leases"))
                .cond_where(
                    sea_orm::sea_query::Condition::any()
                        .add(Expr::col(Alias::new("id")).eq(remote.snapshot_id.as_bytes().to_vec()))
                        .add(
                            Expr::col(Alias::new("parent_lease"))
                                .eq(remote.snapshot_id.as_bytes().to_vec()),
                        ),
                )
                .to_owned(),
        )
        .await?;
    assert!(rows.is_empty());
    Ok(())
}

#[tokio::test]
async fn oversize_leaf_reports_work_limit_and_splits_when_allowed() -> Result<()> {
    let ns = Uuid::new_v4();
    let (_, source, _) = setup(ns).await?;
    let (_, limited, _) = setup(ns).await?;
    let (_, split, _) = setup(ns).await?;
    let key = |n| p::ObjectKey {
        collection: 1,
        id: Uuid::from_u128(n),
    };
    let (db, crowded, _) = setup(ns).await?;
    let tx = db.begin().await?;
    let writer = Uuid::new_v4();
    for counter in 1..=1025_i64 {
        tx.execute(
            &Query::insert()
                .into_table(Alias::new("sync_candidates"))
                .columns(
                    [
                        "namespace",
                        "collection",
                        "writer",
                        "counter",
                        "object",
                        "author",
                        "revision",
                        "body",
                        "digest",
                        "retention_seq",
                        "live_from",
                    ]
                    .map(Alias::new),
                )
                .values_panic([
                    ns.as_bytes().to_vec().into(),
                    b"notes".to_vec().into(),
                    writer.as_bytes().to_vec().into(),
                    counter.into(),
                    key(1).id.as_bytes().to_vec().into(),
                    writer.as_bytes().to_vec().into(),
                    counter.into(),
                    vec![1_u8].into(),
                    vec![0_u8; 32].into(),
                    1_i64.into(),
                    1_i64.into(),
                ])
                .to_owned(),
        )
        .await?;
    }
    tx.commit().await?;
    assert_eq!(crowded.get(&key(1)).await, Err(p::SyncError::WorkLimit));
    for n in 1..=3000 {
        source
            .write_local(key(n), Some(Note(n.to_string())))
            .await?;
    }
    let mut limits = session_limits();
    limits.pages.max_scanned = 128;
    limits.apply.max_projection_items = 128;
    limits.apply.max_mutations = 4096;
    limits.work.max_rows_scanned = 1024;
    limits.work.max_mutations = 4096;
    limits.work.max_depth = 1;
    async fn run(
        src: &CausalEndpoint<Notes>,
        dst: &CausalEndpoint<Notes>,
        limits: p::SessionLimits,
    ) -> p::Result<()> {
        let mut coordinator = p::Coordinator::new(
            p::Scope {
                collections: [1].into_iter().collect(),
            },
            limits,
        )?;
        for _ in 0..100000 {
            match coordinator.advance(dst, dst, dst, src, src).await? {
                p::SessionAdvance::Complete(_) => return Ok(()),
                p::SessionAdvance::BudgetExhausted => coordinator.start_pass(),
                p::SessionAdvance::Incomplete => (),
                p::SessionAdvance::Cancelled => {
                    return Err(p::SyncError::Invalid("cancelled".into()));
                }
            }
        }
        Err(p::SyncError::Invalid("session did not finish".into()))
    }
    assert_eq!(
        run(&source, &limited, limits).await,
        Err(p::SyncError::WorkLimit)
    );
    limits.work.max_depth = 32;
    run(&source, &split, limits).await?;
    for n in 1..=3000 {
        assert_eq!(
            source.get(&key(n)).await?,
            split.get(&key(n)).await?,
            "key {n}"
        );
    }
    Ok(())
}
