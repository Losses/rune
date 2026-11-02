use anyhow::Result;
use sea_orm::{
    ConnectionTrait, Database, TransactionTrait,
    sea_query::{Alias, ColumnDef, Query, Table},
};
use serde::{Deserialize, Serialize};
use sync::protocol as p;
use sync_orm::{causal_progress as progress, causal_schema, causal_store::CausalReplica};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Model;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Value(String);
impl p::SyncModel for Model {
    type Collection = u8;
    type Payload = Value;
    fn model_id() -> &'static str {
        "progress-test"
    }
    fn schema_id() -> &'static str {
        "1"
    }
    fn collections() -> Vec<u8> {
        vec![1]
    }
    fn collection_tag(_: &u8) -> &'static str {
        "values"
    }
    fn collection_from_tag(tag: &str) -> Option<u8> {
        (tag == "values").then_some(1)
    }
    fn validate(_: &p::ObjectKey<Self>, _: &Option<Value>) -> p::Result<()> {
        Ok(())
    }
}
fn scope() -> p::Scope<Model> {
    p::Scope {
        collections: [1].into_iter().collect(),
    }
}

#[tokio::test]
async fn checkpoint_and_completion_share_projection_transaction() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    db.execute(
        &Table::create()
            .table(Alias::new("values"))
            .col(ColumnDef::new(Alias::new("id")).integer().primary_key())
            .to_owned(),
    )
    .await?;
    let namespace = Uuid::new_v4();
    let store = CausalReplica::<Model>::create_empty(db.clone(), namespace, Uuid::new_v4()).await?;
    let identity = store.identity_on(&db).await?;
    let key = p::CheckpointKey {
        namespace,
        source_replica: Uuid::new_v4(),
        source_generation: Uuid::new_v4(),
        scope_id: p::scope_id(&scope())?,
    };
    let received = p::ReceiveProgress {
        sequence: 7,
        anchor: [7; 32],
    };
    let tx = db.begin().await?;
    tx.execute(
        &Query::insert()
            .into_table(Alias::new("values"))
            .columns([Alias::new("id")])
            .values_panic([1.into()])
            .to_owned(),
    )
    .await?;
    progress::initialize_checkpoint_on(&tx, &identity, &key, &scope(), &received).await?;
    assert_eq!(
        progress::checkpoint_on::<Model, _>(&tx, &key).await?,
        Some(received.clone())
    );
    tx.rollback().await?;
    assert!(
        progress::checkpoint_on::<Model, _>(&db, &key)
            .await?
            .is_none()
    );
    assert!(
        db.query_one(
            &Query::select()
                .column(Alias::new("id"))
                .from(Alias::new("values"))
                .to_owned()
        )
        .await?
        .is_none()
    );

    let tx = db.begin().await?;
    tx.execute(
        &Query::insert()
            .into_table(Alias::new("values"))
            .columns([Alias::new("id")])
            .values_panic([1.into()])
            .to_owned(),
    )
    .await?;
    progress::initialize_checkpoint_on(&tx, &identity, &key, &scope(), &received).await?;
    tx.commit().await?;
    let completed = db
        .query_one(
            &Query::select()
                .column(Alias::new("completed_at"))
                .from(Alias::new("sync_checkpoints"))
                .to_owned(),
        )
        .await?
        .unwrap()
        .try_get_by_index::<String>(0)?;
    let tx = db.begin().await?;
    let mut wrong = received.clone();
    wrong.anchor = [9; 32];
    let error = progress::finish_session_on::<Model>(&tx, namespace, &key, &wrong)
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<p::SyncError>(),
        Some(&p::SyncError::RetryAt(received.clone()))
    );
    tx.rollback().await?;
    let unchanged = db
        .query_one(
            &Query::select()
                .column(Alias::new("completed_at"))
                .from(Alias::new("sync_checkpoints"))
                .to_owned(),
        )
        .await?
        .unwrap()
        .try_get_by_index::<String>(0)?;
    assert_eq!(completed, unchanged);
    assert_eq!(
        progress::checkpoint_on::<Model, _>(&db, &key).await?,
        Some(received)
    );
    Ok(())
}

#[tokio::test]
async fn continuity_requires_initialized_matching_generation_and_anchor() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let namespace = Uuid::new_v4();
    let store = CausalReplica::<Model>::create_empty(db.clone(), namespace, Uuid::new_v4()).await?;
    let identity = store.identity_on(&db).await?;
    let key = p::CheckpointKey {
        namespace,
        source_replica: identity.replica,
        source_generation: identity.generation,
        scope_id: p::scope_id(&scope())?,
    };
    let genesis = progress::head_on(&db, namespace).await?;
    assert_eq!(
        progress::history_status_on(&db, &identity, &key, None).await?,
        p::HistoryStatus::Uninitialized {
            head: genesis.clone()
        }
    );
    let tx = db.begin().await?;
    store
        .author_on(
            &tx,
            p::ObjectKey {
                collection: 1,
                id: Uuid::new_v4(),
            },
            Some(Value("one".into())),
        )
        .await?;
    tx.commit().await?;
    let head = progress::head_on(&db, namespace).await?;
    assert_eq!(
        progress::history_status_on(&db, &identity, &key, Some(genesis.clone())).await?,
        p::HistoryStatus::Continuous {
            head: head.clone(),
            floor: genesis
        }
    );
    let mut wrong = head.clone();
    wrong.anchor[0] ^= 1;
    assert_eq!(
        progress::history_status_on(&db, &identity, &key, Some(wrong.clone())).await?,
        p::HistoryStatus::AnchorMismatch { checkpoint: wrong }
    );
    let ahead = p::ReceiveProgress {
        sequence: head.sequence + 1,
        anchor: head.anchor,
    };
    assert_eq!(
        progress::history_status_on(&db, &identity, &key, Some(ahead)).await?,
        p::HistoryStatus::HeadBehind {
            head: head.sequence
        }
    );
    let mut old_generation = key;
    old_generation.source_generation = Uuid::new_v4();
    assert_eq!(
        progress::history_status_on(&db, &identity, &old_generation, Some(head)).await?,
        p::HistoryStatus::GenerationChanged {
            current: identity.generation
        }
    );
    Ok(())
}
