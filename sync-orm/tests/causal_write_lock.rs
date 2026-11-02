use anyhow::Result;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, TransactionTrait};
use sync::protocol::{self as p, SyncStore};
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
    let mut db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
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
