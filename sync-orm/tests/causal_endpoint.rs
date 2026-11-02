use anyhow::{Context, Result, ensure};

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
    let meta = tx
        .query_one(
            &Query::select()
                .columns(["floor_state_seq", "state_seq"].map(Alias::new))
                .from(Alias::new("sync_meta"))
                .to_owned(),
        )
        .await?
        .unwrap();
    assert!(
        sync_orm::causal_context::globally_observed_on(
            &tx,
            sync_orm::causal_context::GlobalObservation {
                namespace,
                collection: b"notes",
                writer: source.store().writer_incarnation(),
                counter: 1,
                floor: meta.try_get_by_index(0)?,
                state_seq: meta.try_get_by_index(1)?,
                max_assertions: 1024
            }
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
    let meta = tx
        .query_one(
            &Query::select()
                .columns(["floor_state_seq", "state_seq"].map(Alias::new))
                .from(Alias::new("sync_meta"))
                .to_owned(),
        )
        .await?
        .unwrap();
    assert!(
        sync_orm::causal_context::globally_observed_on(
            &tx,
            sync_orm::causal_context::GlobalObservation {
                namespace,
                collection: b"notes",
                writer,
                counter: 1,
                floor: meta.try_get_by_index(0)?,
                state_seq: meta.try_get_by_index(1)?,
                max_assertions: 1024
            }
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
        let meta = tx
            .query_one(
                &Query::select()
                    .columns(["floor_state_seq", "state_seq"].map(Alias::new))
                    .from(Alias::new("sync_meta"))
                    .to_owned(),
            )
            .await?
            .unwrap();
        let mut missing = Vec::new();
        for (writer_index, peer) in peers.iter().take(7).enumerate() {
            if !sync_orm::causal_context::globally_observed_on(
                &tx,
                sync_orm::causal_context::GlobalObservation {
                    namespace,
                    collection: b"notes",
                    writer: peer.store().writer_incarnation(),
                    counter: 1,
                    floor: meta.try_get_by_index(0)?,
                    state_seq: meta.try_get_by_index(1)?,
                    max_assertions: 1024,
                },
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
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
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
    let mut after = p::ContextCursor {
        segment_index: 0,
        next_lower: range.lower,
    };
    loop {
        let page = p::SnapshotSource::context_page(
            &source,
            remote.snapshot_id,
            range.clone(),
            after,
            pages,
        )
        .await?;
        endpoint.stage_context_page(session, page.clone()).await?;
        for segment in &page.segments {
            let manifest = segment.context.manifest.clone();
            let stage = endpoint
                .begin_stage(
                    p::StageDescriptor {
                        origin: p::StageOrigin::Snapshot {
                            session,
                            snapshot_id: remote.snapshot_id,
                            range: range.clone(),
                        },
                        manifest: manifest.clone(),
                    },
                    p::StageLimits {
                        max_total_bytes: p::DEFAULT_STAGE_BYTES,
                        max_stages: p::DEFAULT_STAGES,
                        max_event_bytes: p::DEFAULT_LOGICAL_BYTES,
                        max_event_items: p::DEFAULT_LOGICAL_ITEMS,
                        max_fragment_bytes: p::DEFAULT_PAGE_BYTES,
                    },
                )
                .await?;
            for ordinal in stage.next_ordinal..manifest.fragment_count {
                let fragment = p::SnapshotSource::read_fragment(
                    &source,
                    remote.snapshot_id,
                    manifest.clone(),
                    ordinal,
                    65536,
                )
                .await?;
                endpoint.stage_fragment(stage.stage, fragment).await?;
            }
        }
        if page.done {
            break;
        }
        after = page.next;
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
    db.execute_unprepared("CREATE TRIGGER reject_completion BEFORE INSERT ON sync_checkpoints BEGIN SELECT RAISE(ABORT, 'completion fault'); END").await?;
    assert!(
        endpoint
            .commit_complete_range(session, range.clone(), limits())
            .await
            .is_err()
    );
    assert!(endpoint.get(&key).await?.is_none());
    assert!(text(&db).await?.is_none());
    db.execute_unprepared("DROP TRIGGER reject_completion")
        .await?;
    let (coverage, changed) = endpoint
        .commit_complete_range(session, range.clone(), limits())
        .await?;
    assert!(coverage.complete);
    assert_eq!(changed.changes.len(), 1);
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
    assert_eq!(
        endpoint.checkpoint(&checkpoint).await?,
        Some(remote.head.clone())
    );
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
    assert_eq!(
        reopened.checkpoint(&checkpoint).await?,
        Some(remote.head.clone())
    );
    assert_eq!(
        reopened.complete_reconciliation(session).await,
        Err(p::SyncError::StaleWork)
    );
    Ok(())
}

#[tokio::test]
async fn resumable_baseline_respects_pins_and_commits_floor_atomically() -> Result<()> {
    use sync_orm::causal_compaction::{
        BaselineAdvance, advance_baseline_on, prune_history_step_on,
    };
    let namespace = Uuid::new_v4();
    let (db, source, _) = setup(namespace).await?;
    let (_, stale, _) = setup(namespace).await?;
    let key = p::ObjectKey {
        collection: 1,
        id: Uuid::from_u128(1),
    };
    source
        .write_local(key.clone(), Some(Note("remove me".into())))
        .await?;
    synchronize(&source, &stale).await?;
    let scope = p::Scope {
        collections: [1].into_iter().collect(),
    };
    let work = p::WorkLimits {
        max_requests: 64,
        max_bytes: 1024 * 1024,
        max_depth: 32,
        max_rows_scanned: 16,
        max_mutations: 1024,
    };
    let pin = Uuid::new_v4();
    let mut cursor = None;
    loop {
        match p::SnapshotSource::open_snapshot(&source, pin, scope.clone(), work, cursor).await? {
            p::WorkOutcome::Ready(_) => break,
            p::WorkOutcome::Incomplete { cursor: next } => cursor = Some(next),
        }
    }
    source.write_local(key.clone(), None).await?;
    for id in 2..=16 {
        source
            .write_local(
                p::ObjectKey {
                    collection: 1,
                    id: Uuid::from_u128(id),
                },
                Some(Note(format!("value {id}"))),
            )
            .await?;
    }
    let captured = source.head().await?;
    let request = Uuid::new_v4();
    let identity = source.identity().await?;
    let mut cursor = None;
    let mut pinned = false;
    for step in 0..20000 {
        let tx = db.begin().await?;
        let result = advance_baseline_on::<Notes>(
            &tx,
            identity.clone(),
            request,
            chrono::Utc::now().timestamp_millis(),
            work,
            cursor.clone(),
        )
        .await?;
        tx.commit().await?;
        if step == 0 {
            source
                .write_local(
                    p::ObjectKey {
                        collection: 1,
                        id: Uuid::from_u128(100),
                    },
                    Some(Note("after capture".into())),
                )
                .await?;
        }
        match result {
            BaselineAdvance::Incomplete { cursor: next } => cursor = Some(next),
            BaselineAdvance::Pinned { cursor: next } => {
                cursor = Some(next);
                pinned = true;
                break;
            }
            BaselineAdvance::Advanced { .. } => {
                anyhow::bail!("advanced through an older live snapshot")
            }
        }
    }
    ensure!(pinned, "baseline did not reach its protected floor");
    p::SnapshotSource::close_snapshot(&source, pin).await?;
    db.execute_unprepared("CREATE TRIGGER reject_floor BEFORE UPDATE OF baseline_generation ON sync_meta WHEN NEW.baseline_generation != OLD.baseline_generation BEGIN SELECT RAISE(ABORT, 'floor fault'); END").await?;
    let tx = db.begin().await?;
    assert!(
        advance_baseline_on::<Notes>(
            &tx,
            identity.clone(),
            request,
            chrono::Utc::now().timestamp_millis(),
            work,
            cursor.clone()
        )
        .await
        .is_err()
    );
    tx.rollback().await?;
    db.execute_unprepared("DROP TRIGGER reject_floor").await?;
    let author = source.store().author();
    drop(source);
    let source = CausalEndpoint::with_projection(
        CausalReplica::open(db.clone(), namespace, author).await?,
        Arc::new(Projection {
            fail: AtomicBool::new(false),
        }),
    );
    let tx = db.begin().await?;
    let result = advance_baseline_on::<Notes>(
        &tx,
        identity,
        request,
        chrono::Utc::now().timestamp_millis(),
        work,
        cursor,
    )
    .await?;
    tx.commit().await?;
    assert!(matches!(result, BaselineAdvance::Advanced { .. }));
    let checkpoint = p::CheckpointKey {
        namespace,
        source_replica: source.identity().await?.replica,
        source_generation: source.identity().await?.generation,
        scope_id: p::scope_id(&scope)?,
    };
    let status = source
        .history_status(&checkpoint, Some(captured.clone()))
        .await?;
    assert!(
        matches!(status, p::HistoryStatus::Continuous { floor, head } if floor == captured && head.sequence == captured.sequence + 1)
    );
    for _ in 0..100 {
        let tx = db.begin().await?;
        let step = prune_history_step_on(&tx, namespace, 8).await?;
        tx.commit().await?;
        if !step.more {
            break;
        }
    }
    synchronize(&source, &stale).await?;
    assert!(stale.get(&key).await?.is_none());
    let before = source.head().await?;
    synchronize(&stale, &source).await?;
    assert!(source.get(&key).await?.is_none());
    assert_eq!(source.head().await?, before);
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
