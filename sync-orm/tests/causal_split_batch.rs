use anyhow::{Context, Result, ensure};
use sea_orm::{
    ConnectionTrait, Database, ExprTrait,
    sea_query::{Alias, Expr, Query},
};
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
        "split-regression"
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
fn limits(rows: u64) -> p::WorkLimits {
    p::WorkLimits {
        max_requests: 128,
        max_bytes: 1024 * 1024,
        max_depth: 32,
        max_rows_scanned: rows,
        max_mutations: 1024,
    }
}
async fn snapshot(endpoint: &CausalEndpoint<Model>) -> Result<p::SnapshotDescriptor<Model>> {
    let scope = p::Scope {
        collections: [1].into_iter().collect(),
    };
    let request = Uuid::new_v4();
    let mut resume = None;
    for _ in 0..10000 {
        match endpoint
            .open_snapshot(request, scope.clone(), limits(128), resume)
            .await?
        {
            p::WorkOutcome::Ready(value) => return Ok(value),
            p::WorkOutcome::Incomplete { cursor } => {
                resume = Some(serde_json::from_slice(&serde_json::to_vec(&cursor)?)?)
            }
        }
    }
    anyhow::bail!("snapshot did not finish")
}
async fn summarize(
    endpoint: &CausalEndpoint<Model>,
    id: Uuid,
    range: p::KeyRange<Model>,
) -> Result<p::RangeSummary<Model>> {
    let mut resume = None;
    for _ in 0..10000 {
        match endpoint
            .summarize(id, range.clone(), limits(1), resume)
            .await?
        {
            p::WorkOutcome::Ready(value) => return Ok(value),
            p::WorkOutcome::Incomplete { cursor } => {
                resume = Some(serde_json::from_slice(&serde_json::to_vec(&cursor)?)?)
            }
        }
    }
    anyhow::bail!("summary did not finish")
}
async fn split(
    endpoint: &CausalEndpoint<Model>,
    db: &sea_orm::DatabaseConnection,
    id: Uuid,
    range: p::KeyRange<Model>,
    expected_phase: &str,
) -> Result<Vec<p::KeyRange<Model>>> {
    let first = endpoint.split(id, range.clone(), limits(1), None).await?;
    let p::WorkOutcome::Incomplete { mut cursor } = first else {
        anyhow::bail!("split did not create work")
    };
    let query = Query::select()
        .column(Alias::new("build_state"))
        .from(Alias::new("sync_leases"))
        .and_where(Expr::col(Alias::new("id")).eq(cursor.operation_id.as_bytes().to_vec()))
        .to_owned();
    let row = db
        .query_one(&query)
        .await?
        .context("missing split checkpoint")?;
    let state: Vec<u8> = row.try_get_by_index(0)?;
    let state: serde_json::Value = serde_json::from_slice(&state)?;
    assert_eq!(state["phase"], expected_phase);
    for _ in 0..10000 {
        cursor = serde_json::from_slice(&serde_json::to_vec(&cursor)?)?;
        match endpoint
            .split(id, range.clone(), limits(1), Some(cursor))
            .await?
        {
            p::WorkOutcome::Ready(value) => return Ok(value),
            p::WorkOutcome::Incomplete { cursor: next } => cursor = next,
        }
    }
    anyhow::bail!("split did not finish")
}

#[tokio::test]
async fn split_reuses_exact_summaries_and_preserves_the_pinned_view() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let namespace = Uuid::new_v4();
    let store = CausalReplica::<Model>::create_empty(db.clone(), namespace, Uuid::new_v4()).await?;
    let endpoint = CausalEndpoint::new(store);
    for id in (10..=80).step_by(10) {
        endpoint
            .write_local(
                p::ObjectKey {
                    collection: 1,
                    id: Uuid::from_u128(id),
                },
                Some(Value(format!("value-{id}"))),
            )
            .await?;
    }
    let cached = snapshot(&endpoint).await?;
    let uncached = snapshot(&endpoint).await?;
    let root = cached.roots[0].range.clone();
    let subrange = p::KeyRange {
        collection: 1,
        lower: Some(Uuid::from_u128(10)),
        upper: Some(Uuid::from_u128(60)),
    };
    let summary = summarize(&endpoint, cached.snapshot_id, subrange.clone()).await?;
    assert_eq!(summary.candidate_count, 5);
    endpoint
        .write_local(
            p::ObjectKey {
                collection: 1,
                id: Uuid::from_u128(30),
            },
            None,
        )
        .await?;
    endpoint
        .write_local(
            p::ObjectKey {
                collection: 1,
                id: Uuid::from_u128(35),
            },
            Some(Value("later".into())),
        )
        .await?;
    let root_split = split(
        &endpoint,
        &db,
        cached.snapshot_id,
        root.clone(),
        "ContextFallback",
    )
    .await?;
    ensure!(root_split.len() == 2, "root did not split");
    let all_values = p::KeyRange {
        collection: 1,
        lower: Some(Uuid::from_u128(1)),
        upper: Some(Uuid::from_u128(100)),
    };
    let fallback_root = split(
        &endpoint,
        &db,
        uncached.snapshot_id,
        all_values,
        "CountCandidates",
    )
    .await?;
    assert_eq!(fallback_root.len(), 2);
    assert_eq!(root_split[0].upper, fallback_root[0].upper);
    let cached_result = split(
        &endpoint,
        &db,
        cached.snapshot_id,
        subrange.clone(),
        "ContextFallback",
    )
    .await?;
    let fallback_result = split(
        &endpoint,
        &db,
        uncached.snapshot_id,
        subrange,
        "CountCandidates",
    )
    .await?;
    assert_eq!(cached_result, fallback_result);
    assert_eq!(cached_result.len(), 2);
    assert_ne!(cached_result[1].lower, Some(Uuid::from_u128(35)));
    for (range, count) in [
        (
            p::KeyRange {
                collection: 1,
                lower: Some(Uuid::from_u128(81)),
                upper: None,
            },
            0,
        ),
        (
            p::KeyRange {
                collection: 1,
                lower: Some(Uuid::from_u128(10)),
                upper: Some(Uuid::from_u128(11)),
            },
            1,
        ),
    ] {
        assert_eq!(
            summarize(&endpoint, cached.snapshot_id, range.clone())
                .await?
                .candidate_count,
            count
        );
        let left = split(
            &endpoint,
            &db,
            cached.snapshot_id,
            range.clone(),
            "ContextFallback",
        )
        .await?;
        let right = split(
            &endpoint,
            &db,
            uncached.snapshot_id,
            range.clone(),
            "CountCandidates",
        )
        .await?;
        assert_eq!(left, right);
        assert_eq!(left, vec![range]);
    }
    Ok(())
}
