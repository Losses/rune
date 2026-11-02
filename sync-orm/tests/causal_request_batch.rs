use anyhow::{Context, Result, ensure};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection};
use serde::{Deserialize, Serialize};
use sync::protocol::{self as p, SnapshotSource, SyncStore};
use sync_orm::{causal_capabilities::CausalEndpoint, causal_schema, causal_store::CausalReplica};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Model;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Value(String);
impl p::SyncModel for Model {
    type Collection = u8;
    type Payload = Value;
    fn model_id() -> &'static str {
        "request-batch-test"
    }
    fn schema_id() -> &'static str {
        "1"
    }
    fn collections() -> Vec<u8> {
        vec![1]
    }
    fn collection_tag(_: &u8) -> &'static str {
        "items"
    }
    fn collection_from_tag(tag: &str) -> Option<u8> {
        (tag == "items").then_some(1)
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
fn limits(rows: u64) -> p::WorkLimits {
    p::WorkLimits {
        max_requests: 64,
        max_bytes: 1024 * 1024,
        max_depth: 32,
        max_rows_scanned: rows,
        max_mutations: 65536,
    }
}
fn key(id: u128) -> p::ObjectKey<Model> {
    p::ObjectKey {
        collection: 1,
        id: Uuid::from_u128(id),
    }
}
async fn setup(db: DatabaseConnection) -> Result<(CausalEndpoint<Model>, Uuid, Uuid)> {
    causal_schema::install_fresh(&db).await?;
    let namespace = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let endpoint = CausalEndpoint::new(CausalReplica::create_empty(db, namespace, actor).await?);
    for id in 1..=20 {
        endpoint
            .write_local(key(id), Some(Value(format!("captured-{id}"))))
            .await?;
    }
    Ok((endpoint, namespace, actor))
}
async fn finish(
    endpoint: &CausalEndpoint<Model>,
    request: Uuid,
    work: p::WorkLimits,
    mut cursor: Option<p::WorkCursor>,
) -> Result<(p::SnapshotDescriptor<Model>, usize)> {
    for calls in 1..=5000 {
        match endpoint
            .open_snapshot(request, scope(), work, cursor)
            .await?
        {
            p::WorkOutcome::Ready(snapshot) => return Ok((snapshot, calls)),
            p::WorkOutcome::Incomplete { cursor: next } => cursor = Some(next),
        }
    }
    anyhow::bail!("snapshot did not finish")
}
async fn verify_values(
    endpoint: &CausalEndpoint<Model>,
    snapshot: &p::SnapshotDescriptor<Model>,
) -> Result<()> {
    assert_eq!(snapshot.roots.len(), 1);
    let mut after = None;
    let mut values = std::collections::BTreeMap::new();
    loop {
        let page = endpoint
            .leaf(
                snapshot.snapshot_id,
                snapshot.roots[0].range.clone(),
                after,
                p::PageLimits {
                    max_scanned: 100,
                    max_bytes: 1024 * 1024,
                },
            )
            .await?;
        for candidate in page.candidates {
            values.insert(candidate.key.id, candidate.payload.0);
        }
        if page.done {
            break;
        }
        after = page.next;
        ensure!(after.is_some(), "incomplete page did not advance");
    }
    let expected = (1..=20)
        .map(|id| (Uuid::from_u128(id), format!("captured-{id}")))
        .collect();
    assert_eq!(values, expected);
    Ok(())
}

#[tokio::test]
async fn wide_budget_completes_with_fewer_requests_and_same_snapshot() -> Result<()> {
    let (endpoint, _, _) = setup(Database::connect("sqlite::memory:").await?).await?;
    let (low, low_calls) = finish(&endpoint, Uuid::new_v4(), limits(1), None).await?;
    let (wide, wide_calls) = finish(&endpoint, Uuid::new_v4(), limits(1024), None).await?;
    eprintln!("snapshot requests: low={low_calls}, wide={wide_calls}");
    assert_eq!(wide.head, low.head);
    verify_values(&endpoint, &low).await?;
    verify_values(&endpoint, &wide).await?;
    assert!(
        wide_calls * 2 < low_calls,
        "wide={wide_calls}, low={low_calls}"
    );
    Ok(())
}

#[tokio::test]
async fn lost_reply_reopen_concurrent_retry_and_cancel_preserve_admitted_view() -> Result<()> {
    let path = std::env::temp_dir().join(format!("rune-request-batch-{}.sqlite", Uuid::new_v4()));
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let db = Database::connect(&url).await?;
    let (endpoint, namespace, actor) = setup(db.clone()).await?;
    let captured_head = endpoint.head().await?;
    let work = limits(1);
    let request = Uuid::new_v4();
    let cursor = match endpoint.open_snapshot(request, scope(), work, None).await? {
        p::WorkOutcome::Incomplete { cursor } => cursor,
        _ => anyhow::bail!("expected admitted work"),
    };
    let _lost = endpoint
        .open_snapshot(request, scope(), work, Some(cursor.clone()))
        .await?;
    drop(endpoint);
    drop(db);
    let reopened = Database::connect(&url).await?;
    let endpoint = CausalEndpoint::new(
        CausalReplica::<Model>::open(reopened.clone(), namespace, actor).await?,
    );
    endpoint
        .write_local(key(1), Some(Value("changed".into())))
        .await?;
    endpoint.write_local(key(2), None).await?;
    endpoint
        .write_local(key(21), Some(Value("new".into())))
        .await?;
    let other = CausalEndpoint::new(
        CausalReplica::<Model>::open(reopened.clone(), namespace, actor).await?,
    );
    let (first, second) = tokio::join!(
        endpoint.open_snapshot(request, scope(), work, Some(cursor.clone())),
        other.open_snapshot(request, scope(), work, Some(cursor.clone())),
    );
    assert!(
        first.is_ok() || second.is_ok(),
        "both concurrent attempts failed: {first:?}, {second:?}"
    );
    let (snapshot, _) = finish(&endpoint, request, work, Some(cursor.clone())).await?;
    assert_eq!(snapshot.head, captured_head);
    verify_values(&endpoint, &snapshot).await?;
    let (retry, _) = finish(&other, request, work, Some(cursor)).await?;
    assert_eq!(retry, snapshot);

    let cancel_id = Uuid::new_v4();
    let cancel_cursor = match endpoint
        .open_snapshot(cancel_id, scope(), work, None)
        .await?
    {
        p::WorkOutcome::Incomplete { cursor } => cursor,
        _ => anyhow::bail!("expected cancellable admitted work"),
    };
    let _ = endpoint
        .open_snapshot(cancel_id, scope(), work, Some(cancel_cursor.clone()))
        .await?;
    endpoint
        .cancel_snapshot_build(cancel_cursor.clone())
        .await?;
    assert!(
        endpoint
            .open_snapshot(cancel_id, scope(), work, Some(cancel_cursor))
            .await
            .is_err()
    );
    drop(other);
    drop(endpoint);
    drop(reopened);
    std::fs::remove_file(path).context("remove temporary database")?;
    Ok(())
}

#[tokio::test]
async fn storage_error_after_committed_prefix_is_reported_and_retryable() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    let (endpoint, _, _) = setup(db.clone()).await?;
    db.execute_unprepared(
        "CREATE TABLE request_batch_fault (committed INTEGER NOT NULL, fail_at INTEGER NOT NULL)",
    )
    .await?;
    db.execute_unprepared("INSERT INTO request_batch_fault VALUES (0, 3)")
        .await?;
    db.execute_unprepared("CREATE TRIGGER request_batch_reject BEFORE UPDATE OF build_state ON sync_leases WHEN (SELECT committed >= fail_at FROM request_batch_fault) BEGIN SELECT RAISE(ABORT, 'injected checkpoint fault'); END").await?;
    db.execute_unprepared("CREATE TRIGGER request_batch_count AFTER UPDATE OF build_state ON sync_leases BEGIN UPDATE request_batch_fault SET committed = committed + 1; END").await?;
    let work = limits(1024);
    let request = Uuid::new_v4();
    let mut cursor = None;
    let mut faulted = false;
    for _ in 0..1000 {
        let before: i64 = db
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT committed FROM request_batch_fault".to_owned(),
            ))
            .await?
            .context("fault counter")?
            .try_get_by_index(0)?;
        db.execute_unprepared(&format!(
            "UPDATE request_batch_fault SET fail_at = {}",
            before + 1
        ))
        .await?;
        match endpoint
            .open_snapshot(request, scope(), work, cursor.clone())
            .await
        {
            Ok(p::WorkOutcome::Incomplete { cursor: next }) => cursor = Some(next),
            Ok(p::WorkOutcome::Ready(_)) => {
                anyhow::bail!("fixture never encountered a batched checkpoint")
            }
            Err(error) => {
                assert!(
                    error.to_string().contains("injected checkpoint fault"),
                    "{error:?}"
                );
                let after: i64 = db
                    .query_one_raw(sea_orm::Statement::from_string(
                        sea_orm::DatabaseBackend::Sqlite,
                        "SELECT committed FROM request_batch_fault".to_owned(),
                    ))
                    .await?
                    .context("fault counter")?
                    .try_get_by_index(0)?;
                assert_eq!(
                    after - before,
                    1,
                    "the failed call must have committed a prefix"
                );
                faulted = true;
                break;
            }
        }
    }
    assert!(faulted, "injected failure was not reached");
    db.execute_unprepared("DROP TRIGGER request_batch_reject")
        .await?;
    let (snapshot, _) = finish(&endpoint, request, work, cursor).await?;
    verify_values(&endpoint, &snapshot).await?;
    Ok(())
}

#[tokio::test]
async fn one_request_never_advances_more_than_its_transaction_cap() -> Result<()> {
    let (endpoint, _, _) = setup(Database::connect("sqlite::memory:").await?).await?;
    for cap in [1_u32, 2, 128] {
        let work = p::WorkLimits {
            max_requests: cap,
            ..limits(1024)
        };
        let request = Uuid::new_v4();
        let mut previous = None::<p::WorkCursor>;
        let mut largest_delta = 0;
        let mut completed = false;
        for _ in 0..5000 {
            match endpoint
                .open_snapshot(request, scope(), work, previous.clone())
                .await?
            {
                p::WorkOutcome::Ready(snapshot) => {
                    verify_values(&endpoint, &snapshot).await?;
                    completed = true;
                    break;
                }
                p::WorkOutcome::Incomplete { cursor } => {
                    if let Some(old) = previous {
                        let delta = cursor
                            .position
                            .checked_sub(old.position)
                            .context("cursor moved backwards")?;
                        assert!(delta <= u64::from(cap.min(64)), "cap={cap}, delta={delta}");
                        largest_delta = largest_delta.max(delta);
                    }
                    previous = Some(cursor);
                }
            }
        }
        assert!(completed, "snapshot did not finish for cap={cap}");
        if cap > 1 {
            assert!(largest_delta > 1, "cap={cap} never combined steps");
        }
        eprintln!("snapshot transaction cap={cap}, largest cursor advance={largest_delta}");
    }
    Ok(())
}
