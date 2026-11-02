use anyhow::Result;
use sea_orm::{
    ConnectionTrait, Database, ExprTrait, TransactionTrait,
    sea_query::{Alias, Expr, Query},
};
use sync::causal::{Context, Dot};
use sync_orm::{causal_observed as observed, causal_schema};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Notes;
impl sync::protocol::SyncModel for Notes {
    type Collection = u8;
    type Payload = String;
    fn model_id() -> &'static str {
        "observed-test"
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
    fn validate(
        _: &sync::protocol::ObjectKey<Self>,
        _: &Option<String>,
    ) -> sync::protocol::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn reclaim_preserves_pinned_observed_versions() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
    let writer = Uuid::new_v4();
    sync_orm::causal_store::CausalReplica::<Notes>::create_empty(db.clone(), ns, Uuid::new_v4())
        .await?;
    let tx = db.begin().await?;
    let mut budget = 100;
    let mut original = Context::default();
    for n in [1, 3] {
        original.insert(Dot::new(writer, n)?)?;
    }
    observed::insert_on(&tx, ns, b"notes", 1, &original, &mut budget).await?;
    tx.execute(
        &Query::update()
            .table(Alias::new("sync_meta"))
            .value(Alias::new("state_seq"), 1)
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
            .to_owned(),
    )
    .await?;
    let pin = Uuid::new_v4();
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
    let mut middle = Context::default();
    middle.insert(Dot::new(writer, 2)?)?;
    observed::insert_on(&tx, ns, b"notes", 2, &middle, &mut budget).await?;
    tx.execute(
        &Query::update()
            .table(Alias::new("sync_meta"))
            .value(Alias::new("state_seq"), 2)
            .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
            .to_owned(),
    )
    .await?;
    assert_eq!(
        sync_orm::causal_compaction::reclaim_rows_step_on(&tx, ns, 10).await?,
        0
    );
    assert_eq!(
        observed::page_at_on(&tx, ns, b"notes", 1, None, 10).await?,
        vec![(writer, 1, 1), (writer, 3, 3)]
    );
    let current = observed::page_at_on(&tx, ns, b"notes", 2, None, 10).await?;
    assert_eq!(current, vec![(writer, 1, 3)]);
    sync_orm::causal_leases::revoke_on(&tx, ns, pin).await?;
    assert_eq!(
        sync_orm::causal_compaction::reclaim_rows_step_on(&tx, ns, 10).await?,
        2
    );
    assert!(
        observed::page_at_on(&tx, ns, b"notes", 1, None, 10)
            .await?
            .is_empty()
    );
    assert_eq!(
        observed::page_at_on(&tx, ns, b"notes", 2, None, 10).await?,
        current
    );
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
async fn insert_budget_rolls_back_a_two_interval_merge() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
    let writer = Uuid::new_v4();
    let tx = db.begin().await?;
    let mut original = Context::default();
    for n in [1, 3] {
        original.insert(Dot::new(writer, n)?)?;
    }
    observed::insert_on(&tx, ns, b"notes", 1, &original, &mut 100).await?;
    tx.commit().await?;
    let tx = db.begin().await?;
    let mut middle = Context::default();
    middle.insert(Dot::new(writer, 2)?)?;
    let error = observed::insert_on(&tx, ns, b"notes", 2, &middle, &mut 1)
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<sync::protocol::SyncError>(),
        Some(sync::protocol::SyncError::WorkLimit)
    ));
    tx.rollback().await?;
    let tx = db.begin().await?;
    assert_eq!(
        observed::page_at_on(&tx, ns, b"notes", 2, None, 10).await?,
        vec![(writer, 1, 1), (writer, 3, 3)]
    );
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
async fn observed_rows_are_versioned_and_canonical() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
    let writer = Uuid::new_v4();
    let tx = db.begin().await?;
    let mut context = Context::default();
    context.insert(Dot::new(writer, 1)?)?;
    context.insert(Dot::new(writer, 3)?)?;
    let mut budget = 100;
    assert!(observed::insert_on(&tx, ns, b"notes", 1, &context, &mut budget).await?);
    assert!(!observed::contains_on(&tx, ns, b"notes", Dot::new(writer, 2)?).await?);
    let mut middle = Context::default();
    middle.insert(Dot::new(writer, 2)?)?;
    assert!(observed::insert_on(&tx, ns, b"notes", 2, &middle, &mut budget).await?);
    assert_eq!(
        observed::page_at_on(&tx, ns, b"notes", 1, None, 10).await?,
        vec![(writer, 1, 1), (writer, 3, 3)]
    );
    assert_eq!(
        observed::page_at_on(&tx, ns, b"notes", 2, None, 10).await?,
        vec![(writer, 1, 3)]
    );
    assert!(observed::contains_on(&tx, ns, b"notes", Dot::new(writer, 2)?).await?);
    assert!(!observed::insert_on(&tx, ns, b"notes", 2, &context, &mut budget).await?);
    assert_eq!(
        observed::unobserved_on(&tx, ns, b"notes", writer, 1, 5).await?,
        vec![(4, 5)]
    );
    assert_eq!(observed::reclaim_step_on(&tx, ns, 2, 10).await?, 2);
    assert_eq!(
        observed::page_at_on(&tx, ns, b"notes", 2, None, 10).await?,
        vec![(writer, 1, 3)]
    );
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
async fn observed_terminal_interval_and_same_state_union() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
    let writer = Uuid::new_v4();
    let tx = db.begin().await?;
    let mut budget = 100;
    for counter in [1, 3, 2, sync::causal::MAX_DOT_COUNTER] {
        let mut context = Context::default();
        context.insert(Dot::new(writer, counter)?)?;
        assert!(observed::insert_on(&tx, ns, b"notes", 1, &context, &mut budget).await?);
    }
    assert_eq!(
        observed::page_at_on(&tx, ns, b"notes", 1, None, 10).await?,
        vec![
            (writer, 1, 3),
            (
                writer,
                sync::causal::MAX_DOT_COUNTER,
                sync::causal::MAX_DOT_COUNTER
            )
        ]
    );
    assert!(
        observed::contains_on(
            &tx,
            ns,
            b"notes",
            Dot::new(writer, sync::causal::MAX_DOT_COUNTER)?
        )
        .await?
    );
    assert_eq!(observed::reclaim_step_on(&tx, ns, 1, 10).await?, 0);
    tx.commit().await?;
    Ok(())
}
