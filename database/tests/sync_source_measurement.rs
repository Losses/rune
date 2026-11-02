#[path = "support/causal_library.rs"]
mod support;
use anyhow::Result;
use database::sync::{LibrarySyncStore, model::*};
use sea_orm::{
    ConnectionTrait, ExprTrait,
    sea_query::{Alias, Expr, Query},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use support::{SqlLibrary, key};
use sync::{SnapshotSource, WorkOutcome};
use uuid::Uuid;

// Classify only known relations; raw SQL and bound values are never retained.
const TABLES: &[&str] = &[
    "sync_context_intervals",
    "sync_context_ranges",
    "sync_candidates",
    "sync_manifests",
    "sync_manifest_edges",
    "sync_chunks",
    "sync_leases",
    "sync_meta",
    "sync_baseline_segments",
    "sync_journal",
    "sync_writers",
    "sync_work_roots",
    "sync_checkpoint",
    "sync_gc",
];

#[derive(Default)]
struct QueryTotal {
    count: usize,
    elapsed: Duration,
    failed: usize,
}

#[derive(Default)]
struct QueryStats {
    active: Option<String>,
    by_operation: BTreeMap<(String, &'static str, &'static str), QueryTotal>,
}

#[tokio::test]
#[ignore = "manual SQL source operation measurement"]
async fn source_build_measurement() -> Result<()> {
    let namespace = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let library = SqlLibrary::new(namespace, actor).await?;
    let records = std::env::var("SYNC_SOURCE_RECORDS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1);
    for index in 0..records {
        library
            .peer
            .write_local(
                key(Collection::Playlists, Uuid::new_v4()),
                support::playlist(&format!("record {index}")),
            )
            .await?;
    }
    // Connection clones retain their own callback, so reopen the endpoint from
    // the instrumented connection before measuring. Fixture writes above use
    // the same database and actor as the uninstrumented measurement.
    let stats = Arc::new(Mutex::new(QueryStats::default()));
    let callback_stats = stats.clone();
    let mut measured_db = library.db.clone();
    measured_db.set_metric_callback(move |info| {
        let mut stats = callback_stats.lock().unwrap();
        if let Some(phase) = stats.active.clone() {
            let sql = info.statement.sql.to_ascii_lowercase();
            let operation = match sql.split_whitespace().next() {
                Some("select") => "select",
                Some("insert") => "insert",
                Some("update") => "update",
                Some("delete") => "delete",
                _ => "other",
            };
            let table = TABLES
                .iter()
                .copied()
                .find(|table| sql.contains(table))
                .unwrap_or("other");
            let entry = stats
                .by_operation
                .entry((phase, operation, table))
                .or_default();
            entry.count += 1;
            entry.elapsed += info.elapsed;
            entry.failed += usize::from(info.failed);
        }
    });
    let measured = LibrarySyncStore::open(measured_db, actor, Some(namespace)).await?;
    let source = measured.endpoint();
    let request = Uuid::new_v4();
    let mut cursor = None;
    let mut by_phase: BTreeMap<String, (usize, Duration, usize)> = BTreeMap::new();
    let mut phase = "admit".to_owned();
    let total = Instant::now();
    for _ in 0..20000 {
        stats.lock().unwrap().active = Some(phase.clone());
        let start = Instant::now();
        let outcome = source
            .open_snapshot(request, support::scope(), support::limits().work, cursor)
            .await;
        let elapsed = start.elapsed();
        stats.lock().unwrap().active = None;
        let outcome = outcome?;
        let row = library
            .db
            .query_one(
                &Query::select()
                    .column(Alias::new("build_state"))
                    .from(Alias::new("sync_leases"))
                    .and_where(Expr::col(Alias::new("id")).eq(request.as_bytes().to_vec()))
                    .to_owned(),
            )
            .await?
            .unwrap();
        let data = row
            .try_get_by_index::<Option<Vec<u8>>>(0)?
            .unwrap_or_default();
        let entry = by_phase.entry(phase.clone()).or_default();
        entry.0 += 1;
        entry.1 += elapsed;
        entry.2 = entry.2.max(data.len());
        match outcome {
            WorkOutcome::Incomplete { cursor: next } => cursor = Some(next),
            WorkOutcome::Ready(_) => {
                eprintln!(
                    "environment={} {} profile={} store=sqlite::memory: collections={} records={} elapsed_ms={}",
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                    if cfg!(debug_assertions) {
                        "debug"
                    } else {
                        "release"
                    },
                    support::scope().collections.len(),
                    records,
                    total.elapsed().as_millis()
                );
                for (name, (steps, time, bytes)) in by_phase {
                    eprintln!(
                        "phase={name} steps={steps} time_ms={} max_checkpoint_bytes={bytes}",
                        time.as_millis()
                    );
                }
                {
                    let stats = stats.lock().unwrap();
                    for ((phase, operation, table), value) in &stats.by_operation {
                        eprintln!(
                            "query phase={phase} operation={operation} table={table} count={} elapsed_us={} failed={}",
                            value.count,
                            value.elapsed.as_micros(),
                            value.failed,
                        );
                    }
                }
                source.close_snapshot(request).await?;
                return Ok(());
            }
        }
        let state: serde_json::Value = serde_json::from_slice(&data)?;
        phase = if state["collection_index"].as_u64().unwrap()
            < support::scope().collections.len() as u64
        {
            if state["union"]["phase"].is_string() {
                state["union"]["phase"].as_str().unwrap()
            } else if state["awaiting_union"].as_bool().unwrap_or(false) {
                "context-begin"
            } else if state["pending_context"].is_object()
                || state["collection_end"].as_bool().unwrap_or(false)
            {
                "context-segment"
            } else {
                "context-boundary"
            }
        } else if !state["reversal"]["done"].as_bool().unwrap_or(false) {
            "segment-order"
        } else if state["partition_index"].as_u64().unwrap()
            < support::scope().collections.len() as u64
        {
            "partitions"
        } else if !state["root_promoted"].as_bool().unwrap_or(false) {
            "root"
        } else if !state["staging"]["done"].as_bool().unwrap_or(false) {
            "staging"
        } else {
            "summaries"
        }
        .to_owned();
    }
    anyhow::bail!("source build did not finish")
}
