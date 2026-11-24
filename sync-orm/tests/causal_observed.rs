#[path = "support/fixtures.rs"]
mod fixtures;

use anyhow::Result;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, ExprTrait, TransactionTrait,
    sea_query::{Alias, Expr, Func, Query},
};
use sync::causal;
use sync::causal::{Context, Dot};
use sync::protocol;
use sync::protocol::SyncError;
use sync_orm::causal_candidates;
use sync_orm::causal_compaction;
use sync_orm::causal_leases;
use sync_orm::causal_observed as observed;
use sync_orm::causal_store;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Notes;
impl protocol::SyncModel for Notes {
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
    fn validate(_: &protocol::ObjectKey<Self>, _: &Option<String>) -> protocol::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn reclaim_preserves_pinned_observed_versions() -> Result<()> {
    let db = fixtures::fresh_database().await?;
    let ns = Uuid::new_v4();
    let writer = Uuid::new_v4();
    causal_store::CausalReplica::<Notes>::create_empty(db.clone(), ns, Uuid::new_v4()).await?;
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
    causal_leases::create_build_on(
        &tx,
        causal_leases::BuildLeaseRequest {
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
        causal_compaction::reclaim_rows_step_on(&tx, ns, 10).await?,
        0
    );
    assert_eq!(
        observed::page_at_on(&tx, ns, b"notes", 1, None, 10).await?,
        vec![(writer, 1, 1), (writer, 3, 3)]
    );
    let current = observed::page_at_on(&tx, ns, b"notes", 2, None, 10).await?;
    assert_eq!(current, vec![(writer, 1, 3)]);
    causal_leases::revoke_on(&tx, ns, pin).await?;
    assert_eq!(
        causal_compaction::reclaim_rows_step_on(&tx, ns, 10).await?,
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
    let db = fixtures::fresh_database().await?;
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
        error.downcast_ref::<protocol::SyncError>(),
        Some(SyncError::WorkLimit)
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
    let db = fixtures::fresh_database().await?;
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
    let db = fixtures::fresh_database().await?;
    let ns = Uuid::new_v4();
    let writer = Uuid::new_v4();
    let tx = db.begin().await?;
    let mut budget = 100;
    for counter in [1, 3, 2, causal::MAX_DOT_COUNTER] {
        let mut context = Context::default();
        context.insert(Dot::new(writer, counter)?)?;
        assert!(observed::insert_on(&tx, ns, b"notes", 1, &context, &mut budget).await?);
    }
    assert_eq!(
        observed::page_at_on(&tx, ns, b"notes", 1, None, 10).await?,
        vec![
            (writer, 1, 3),
            (writer, causal::MAX_DOT_COUNTER, causal::MAX_DOT_COUNTER)
        ]
    );
    assert!(
        observed::contains_on(
            &tx,
            ns,
            b"notes",
            Dot::new(writer, causal::MAX_DOT_COUNTER)?
        )
        .await?
    );
    assert_eq!(observed::reclaim_step_on(&tx, ns, 1, 10).await?, 0);
    tx.commit().await?;
    Ok(())
}

/// The stored state a failed authoring command must leave untouched.
#[derive(Clone, Debug, PartialEq, Eq)]
struct StoredState {
    head: i64,
    state_seq: i64,
    revision_counter: i64,
    journal_rows: i64,
    candidates: Vec<(Vec<u8>, i64, i64, Option<i64>, Option<i64>)>,
}

async fn stored_state(
    tx: &DatabaseTransaction,
    ns: Uuid,
    key: &protocol::ObjectKey<Notes>,
) -> Result<StoredState> {
    let meta = tx
        .query_one(
            &Query::select()
                .columns(["head", "state_seq", "revision_counter"].map(Alias::new))
                .from(Alias::new("sync_meta"))
                .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                .to_owned(),
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing sync_meta row"))?;
    let state_seq: i64 = meta.try_get_by_index(1)?;
    let page = causal_candidates::page_on(
        tx,
        causal_candidates::CandidatePageRequest {
            namespace: ns,
            collection: b"notes",
            state_seq,
            lower: Some(key.id),
            upper: key.id.as_u128().checked_add(1).map(Uuid::from_u128),
            after: None,
            max_items: 1024,
            max_bytes: 4 * 1024 * 1024,
        },
    )
    .await?;
    assert!(page.done, "the state helper reads every live candidate");
    let candidates = page
        .rows
        .into_iter()
        .map(|row| {
            (
                row.writer,
                row.counter,
                row.revision,
                row.live_from,
                row.live_until,
            )
        })
        .collect();
    let journal_rows = tx
        .query_one(
            &Query::select()
                .expr(Func::count(Expr::val(1)))
                .from(Alias::new("sync_journal"))
                .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                .to_owned(),
        )
        .await?
        .map(|row| row.try_get_by_index::<i64>(0))
        .transpose()?
        .unwrap_or(0);
    Ok(StoredState {
        head: meta.try_get_by_index(0)?,
        state_seq,
        revision_counter: meta.try_get_by_index(2)?,
        journal_rows,
        candidates,
    })
}

async fn committed_state(
    db: &DatabaseConnection,
    ns: Uuid,
    key: &protocol::ObjectKey<Notes>,
) -> Result<StoredState> {
    let tx = db.begin().await?;
    let state = stored_state(&tx, ns, key).await?;
    tx.commit().await?;
    Ok(state)
}

#[tokio::test]
async fn sixty_five_concurrent_candidates_hit_the_work_limit_and_roll_back() -> Result<()> {
    let db = fixtures::fresh_database().await?;
    let ns = Uuid::new_v4();
    let key = protocol::ObjectKey {
        collection: 1,
        id: Uuid::new_v4(),
    };
    let replica =
        causal_store::CausalReplica::<Notes>::create_empty(db.clone(), ns, Uuid::new_v4()).await?;
    // Concurrent assignments observe none of the live candidates at their key.
    for index in 1..=65 {
        let tx = db.begin().await?;
        replica
            .write_observed_on(
                &tx,
                key.clone(),
                Some(format!("concurrent {index}")),
                protocol::Observed::Exact(Default::default()),
            )
            .await?;
        tx.commit().await?;
    }
    let before = committed_state(&db, ns, &key).await?;
    assert_eq!(before.candidates.len(), 65, "65 candidates stay live");
    assert_eq!(
        (before.head, before.state_seq, before.revision_counter),
        (65, 65, 65)
    );

    let tx = db.begin().await?;
    let error = replica
        .write_observed_on(
            &tx,
            key.clone(),
            Some("observe all".into()),
            protocol::Observed::AllCurrent,
        )
        .await
        .expect_err("65 observed candidates must exceed the work limit");
    assert!(
        matches!(
            error.downcast_ref::<protocol::SyncError>(),
            Some(SyncError::WorkLimit)
        ),
        "{error:#}"
    );
    assert_eq!(
        stored_state(&tx, ns, &key).await?,
        before,
        "the failed command must not write before returning"
    );
    tx.rollback().await?;
    assert_eq!(
        committed_state(&db, ns, &key).await?,
        before,
        "rollback must restore head, revision and every candidate lifetime"
    );
    Ok(())
}

#[tokio::test]
async fn exhausted_revision_clock_rolls_back_authoring() -> Result<()> {
    let db = fixtures::fresh_database().await?;
    let ns = Uuid::new_v4();
    let key = protocol::ObjectKey {
        collection: 1,
        id: Uuid::new_v4(),
    };
    let replica =
        causal_store::CausalReplica::<Notes>::create_empty(db.clone(), ns, Uuid::new_v4()).await?;
    let tx = db.begin().await?;
    replica
        .write_observed_on(
            &tx,
            key.clone(),
            Some("first".into()),
            protocol::Observed::AllCurrent,
        )
        .await?;
    tx.commit().await?;
    let tx = db.begin().await?;
    assert_eq!(
        tx.execute(
            &Query::update()
                .table(Alias::new("sync_meta"))
                .value(Alias::new("revision_counter"), i64::MAX)
                .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                .to_owned(),
        )
        .await?
        .rows_affected(),
        1
    );
    tx.commit().await?;
    let before = committed_state(&db, ns, &key).await?;
    assert_eq!(before.revision_counter, i64::MAX);

    let tx = db.begin().await?;
    let error = replica
        .write_observed_on(
            &tx,
            key.clone(),
            Some("second".into()),
            protocol::Observed::AllCurrent,
        )
        .await
        .expect_err("an exhausted revision clock must fail the command");
    assert!(
        matches!(
            error.downcast_ref::<protocol::SyncError>(),
            Some(SyncError::ClockExhausted)
        ),
        "{error:#}"
    );
    assert_eq!(
        stored_state(&tx, ns, &key).await?,
        before,
        "clock exhaustion must not consume a dot or publish history"
    );
    tx.rollback().await?;
    assert_eq!(
        committed_state(&db, ns, &key).await?,
        before,
        "rollback must restore the exhausted clock and the live candidate"
    );

    let tx = db.begin().await?;
    assert_eq!(
        replica.get_on(&tx, &key).await?.map(|record| record.value),
        Some(Some("first".into()))
    );
    tx.commit().await?;
    Ok(())
}
