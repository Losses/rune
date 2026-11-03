#[path = "support/fixtures.rs"]
mod fixtures;

use anyhow::Result;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, TransactionTrait};
use sync::protocol::{self as p, ReceiveStore, SyncStore};
use sync_orm::maintenance::Maintenance;
use sync_orm::{causal_capabilities::CausalEndpoint, causal_schema, causal_store::CausalReplica};
use uuid::Uuid;
#[derive(Clone, Debug, PartialEq, Eq)]
struct Model;
impl p::SyncModel for Model {
    type Collection = u8;
    type Payload = String;
    fn model_id() -> &'static str {
        "lock-test"
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
    fn collection_from_tag(tag: &str) -> Option<u8> {
        (tag == "notes").then_some(1)
    }
    fn validate(_: &p::ObjectKey<Self>, _: &Option<String>) -> p::Result<()> {
        Ok(())
    }
}
#[tokio::test]
async fn write_transaction_survives_concurrent_commit() -> Result<()> {
    let path = std::env::temp_dir().join(format!("sync-wal-{}.sqlite", Uuid::new_v4()));
    let mut options = ConnectOptions::new(format!("sqlite://{}?mode=rwc", path.display()));
    options.max_connections(2).min_connections(2);
    let db = Database::connect(options).await?;
    db.execute_unprepared("PRAGMA journal_mode=WAL").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
    let store = CausalReplica::<Model>::create_empty(db.clone(), ns, Uuid::new_v4()).await?;
    let endpoint = CausalEndpoint::new(store.clone());
    let key = |n| p::ObjectKey {
        collection: 1,
        id: Uuid::from_u128(n),
    };
    // BEGIN is deferred: the held connection has made no statement yet.
    let tx = db.begin().await?;
    endpoint
        .write_local(key(1), Some("other connection".into()))
        .await?;
    // Authoring must acquire the writer lock before reading the new snapshot.
    store
        .author_on(&tx, key(2), Some("held connection".into()))
        .await?;
    tx.commit().await?;
    assert_eq!(endpoint.head().await?.sequence, 2);
    assert_eq!(
        endpoint.get(&key(1)).await?.unwrap().value.as_deref(),
        Some("other connection")
    );
    assert_eq!(
        endpoint.get(&key(2)).await?.unwrap().value.as_deref(),
        Some("held connection")
    );
    drop(endpoint);
    drop(store);
    db.close().await?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[tokio::test]
async fn receive_transaction_locks_before_reading_progress() -> Result<()> {
    use p::ReceiveStore;
    use std::sync::{Arc, Mutex};
    let mut db = fixtures::fresh_database().await?;
    let ns = Uuid::new_v4();
    let store = CausalReplica::<Model>::create_empty(db.clone(), ns, Uuid::new_v4()).await?;
    let statements = Arc::new(Mutex::new(Vec::new()));
    let captured = statements.clone();
    db.set_metric_callback(move |info| {
        captured.lock().unwrap().push(info.statement.sql.clone());
    });
    let endpoint = CausalEndpoint::new(CausalReplica::<Model>::open(db, ns, store.author()).await?);
    statements.lock().unwrap().clear();
    let mut identity = endpoint.identity().await?;
    identity.replica = Uuid::new_v4();
    identity.generation = Uuid::new_v4();
    let head = p::ReceiveProgress {
        sequence: 0,
        anchor: p::initial_anchor(&identity)?,
    };
    let page = p::ChangePage {
        request: p::PageRequest {
            source: identity,
            scope: p::Scope {
                collections: [1].into_iter().collect(),
            },
            after: head.clone(),
            through: head.clone(),
            limits: p::PageLimits {
                max_scanned: 10,
                max_bytes: 65536,
            },
        },
        next: head,
        done: true,
        links: vec![],
    };
    statements.lock().unwrap().clear();
    assert!(
        endpoint
            .apply_page(
                &page,
                p::ApplyLimits {
                    max_projection_items: 10,
                    max_projection_bytes: 65536,
                    max_mutations: 1024
                }
            )
            .await
            .is_err()
    );
    let sql = statements.lock().unwrap();
    assert!(
        sql.first()
            .is_some_and(|q| q.starts_with("UPDATE \"sync_meta\"")),
        "first statement: {:?}",
        sql.first()
    );
    Ok(())
}

#[tokio::test]
async fn author_and_endpoint_lock_before_reading() -> Result<()> {
    use std::sync::{Arc, Mutex};
    let mut db = fixtures::fresh_database().await?;
    let ns = Uuid::new_v4();
    let store = CausalReplica::<Model>::create_empty(db.clone(), ns, Uuid::new_v4()).await?;
    let captured = Arc::new(Mutex::new(Vec::new()));
    let statements = captured.clone();
    db.set_metric_callback(move |info| statements.lock().unwrap().push(info.statement.sql.clone()));
    let store = CausalReplica::<Model>::open(db.clone(), ns, store.author()).await?;
    let endpoint = CausalEndpoint::new(store.clone());
    let key = |n| p::ObjectKey {
        collection: 1,
        id: Uuid::from_u128(n),
    };
    let tx = db.begin().await?;
    captured.lock().unwrap().clear();
    store.author_on(&tx, key(1), Some("one".into())).await?;
    assert!(
        captured
            .lock()
            .unwrap()
            .first()
            .is_some_and(|s| s.starts_with("UPDATE \"sync_meta\""))
    );
    tx.commit().await?;
    captured.lock().unwrap().clear();
    endpoint.write_local(key(2), Some("two".into())).await?;
    assert!(
        captured
            .lock()
            .unwrap()
            .first()
            .is_some_and(|s| s.starts_with("UPDATE \"sync_meta\""))
    );
    Ok(())
}

#[tokio::test]
async fn maintenance_locks_before_reading() -> Result<()> {
    use std::sync::{Arc, Mutex};
    let mut db = fixtures::fresh_database().await?;
    let ns = Uuid::new_v4();
    let store = CausalReplica::<Model>::create_empty(db.clone(), ns, Uuid::new_v4()).await?;
    let endpoint = CausalEndpoint::new(store);
    let identity = endpoint.identity().await?;
    let captured = Arc::new(Mutex::new(Vec::new()));
    let statements = captured.clone();
    db.set_metric_callback(move |info| statements.lock().unwrap().push(info.statement.sql.clone()));
    captured.lock().unwrap().clear();
    Maintenance::default()
        .advance::<Model>(&db, identity, 0)
        .await?;
    assert!(
        captured
            .lock()
            .unwrap()
            .first()
            .is_some_and(|s| s.starts_with("UPDATE \"sync_meta\""))
    );
    Ok(())
}

async fn snapshot<S: p::SnapshotSource<Model>>(source: &S) -> Result<p::SnapshotDescriptor<Model>> {
    let request = Uuid::new_v4();
    let mut cursor = None;
    loop {
        match source
            .open_snapshot(
                request,
                p::Scope {
                    collections: [1].into_iter().collect(),
                },
                work(),
                cursor,
            )
            .await?
        {
            p::WorkOutcome::Ready(snapshot) => return Ok(snapshot),
            p::WorkOutcome::Incomplete { cursor: next } => cursor = Some(next),
        }
    }
}
fn work() -> p::WorkLimits {
    p::WorkLimits {
        max_requests: 1024,
        max_bytes: 1024 * 1024,
        max_depth: 32,
        max_rows_scanned: 1024,
        max_mutations: 65536,
    }
}
fn apply_limits() -> p::ApplyLimits {
    p::ApplyLimits {
        max_projection_items: 10,
        max_projection_bytes: 65536,
        max_mutations: 1024,
    }
}
async fn prepared_session() -> Result<(
    CausalEndpoint<Model>,
    Uuid,
    p::KeyRange<Model>,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
)> {
    use p::{SnapshotSink, SnapshotSource};
    use std::sync::{Arc, Mutex};
    let mut db = fixtures::fresh_database().await?;
    let ns = Uuid::new_v4();
    let original = CausalReplica::<Model>::create_empty(db.clone(), ns, Uuid::new_v4()).await?;
    let captured = Arc::new(Mutex::new(Vec::new()));
    let statements = captured.clone();
    db.set_metric_callback(move |info| statements.lock().unwrap().push(info.statement.sql.clone()));
    let target =
        CausalEndpoint::new(CausalReplica::<Model>::open(db, ns, original.author()).await?);
    let source = p::MemorySyncStore::<Model>::new(
        p::StoreIdentity {
            namespace: ns,
            model: p::ModelId {
                model: "lock-test".into(),
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
    source
        .write_local(
            p::ObjectKey {
                collection: 1,
                id: Uuid::new_v4(),
            },
            Some("source".into()),
        )
        .await?;
    let remote = snapshot(&source).await?;
    let local = snapshot(&target).await?;
    let session = target
        .begin_reconciliation(Uuid::new_v4(), remote.clone(), local)
        .await?;
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
                p::StageLimits {
                    max_total_bytes: p::DEFAULT_STAGE_BYTES,
                    max_stages: p::DEFAULT_STAGES,
                    max_event_bytes: p::DEFAULT_LOGICAL_BYTES,
                    max_event_items: p::DEFAULT_LOGICAL_ITEMS,
                    max_fragment_bytes: p::DEFAULT_PAGE_BYTES,
                },
            )
            .await?;
        for ordinal in 0..manifest.fragment_count {
            let fragment = p::SnapshotSource::read_fragment(
                &source,
                remote.snapshot_id,
                manifest.clone(),
                ordinal,
                p::DEFAULT_PAGE_BYTES,
            )
            .await?;
            target.stage_fragment(stage.stage, fragment).await?;
        }
    }
    let range = remote.roots[0].range.clone();
    let page = source
        .leaf(
            remote.snapshot_id,
            range.clone(),
            None,
            p::PageLimits {
                max_scanned: 128,
                max_bytes: 65536,
            },
        )
        .await?;
    target.stage_leaf_page(session, page).await?;
    Ok((target, session, range, captured))
}

#[tokio::test]
async fn merge_locks_before_reading() -> Result<()> {
    use p::SnapshotSink;
    let (target, session, range, statements) = prepared_session().await?;
    statements.lock().unwrap().clear();
    target
        .commit_complete_range(session, range, apply_limits())
        .await?;
    assert!(
        statements
            .lock()
            .unwrap()
            .first()
            .is_some_and(|s| s.starts_with("UPDATE \"sync_meta\""))
    );
    Ok(())
}

#[tokio::test]
async fn promotion_locks_before_reading() -> Result<()> {
    use p::SnapshotSink;
    let (target, session, range, statements) = prepared_session().await?;
    target
        .commit_complete_range(session, range, apply_limits())
        .await?;
    statements.lock().unwrap().clear();
    target.promote_step(session, apply_limits()).await?;
    assert!(
        statements
            .lock()
            .unwrap()
            .first()
            .is_some_and(|s| s.starts_with("UPDATE \"sync_meta\""))
    );
    Ok(())
}
