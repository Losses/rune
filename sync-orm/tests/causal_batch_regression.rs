//! Independent SQL work and captured-state checks for candidate paging.
#[path = "support/fixtures.rs"]
mod fixtures;

use anyhow::{Context, Result};
use sea_orm::{
    ConnectionTrait, ExprTrait, TransactionTrait,
    sea_query::{Alias, Expr, Query},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use sync_orm::causal_candidates::{CandidatePageRequest, page_on};
use uuid::Uuid;

async fn insert_candidate(
    db: &sea_orm::DatabaseConnection,
    namespace: Uuid,
    object: Uuid,
    writer: Uuid,
    counter: i64,
    start: i64,
    body: Vec<u8>,
) -> Result<()> {
    let statement = Query::insert()
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
                "live_until",
            ]
            .map(Alias::new),
        )
        .values_panic([
            namespace.as_bytes().to_vec().into(),
            b"notes".to_vec().into(),
            writer.as_bytes().to_vec().into(),
            counter.into(),
            object.as_bytes().to_vec().into(),
            writer.as_bytes().to_vec().into(),
            counter.into(),
            body.into(),
            vec![7_u8; 32].into(),
            start.into(),
            Some(start).into(),
            Option::<i64>::None.into(),
        ])
        .to_owned();
    db.execute(&statement).await?;
    Ok(())
}
fn request(
    namespace: Uuid,
    state_seq: i64,
    max_items: usize,
    max_bytes: usize,
) -> CandidatePageRequest<'static> {
    CandidatePageRequest {
        namespace,
        collection: b"notes",
        state_seq,
        lower: None,
        upper: None,
        after: None,
        max_items,
        max_bytes,
    }
}

#[tokio::test]
async fn captured_candidate_pages_batch_reads_and_preserve_byte_boundaries() -> Result<()> {
    let mut db = fixtures::fresh_database().await?;
    let namespace = Uuid::from_u128(1);
    let writer = Uuid::from_u128(2);
    const COUNT: usize = 40;
    const BODY: usize = 128;
    for index in 0..COUNT {
        insert_candidate(
            &db,
            namespace,
            Uuid::from_u128(index as u128 + 10),
            writer,
            index as i64 + 1,
            1,
            vec![index as u8; BODY],
        )
        .await?;
    }
    // A later committed replacement must not enter the captured position.
    db.execute(
        &Query::update()
            .table(Alias::new("sync_candidates"))
            .value(Alias::new("live_until"), 11)
            .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("object")).eq(Uuid::from_u128(10).as_bytes().to_vec()))
            .to_owned(),
    )
    .await?;
    insert_candidate(
        &db,
        namespace,
        Uuid::from_u128(10),
        writer,
        1001,
        11,
        vec![201; BODY],
    )
    .await?;

    let selects = Arc::new(AtomicUsize::new(0));
    let measured = selects.clone();
    db.set_metric_callback(move |info| {
        if info.statement.sql.starts_with("SELECT")
            && info.statement.sql.contains("\"sync_candidates\"")
        {
            measured.fetch_add(1, Ordering::SeqCst);
        }
    });
    let tx = db.begin().await?;
    let captured = page_on(&tx, request(namespace, 10, COUNT, COUNT * (BODY + 96))).await?;
    let read_count = selects.swap(0, Ordering::SeqCst);
    eprintln!("captured candidate page SELECTs={read_count} rows={COUNT}");
    assert_eq!(captured.rows.len(), COUNT);
    assert!(captured.done);
    assert_eq!(captured.rows[0].counter, 1);
    assert_eq!(captured.rows[0].body, vec![0; BODY]);
    assert!(
        read_count > 0 && read_count <= 4,
        "{read_count} candidate SELECTs for {COUNT} rows"
    );

    let current = page_on(&tx, request(namespace, 11, COUNT, COUNT * (BODY + 96))).await?;
    assert_eq!(current.rows.len(), COUNT);
    assert!(current.done);
    assert_eq!(current.rows[0].counter, 1001);
    assert_eq!(current.rows[0].body, vec![201; BODY]);

    let first = page_on(&tx, request(namespace, 10, 2, BODY + 96)).await?;
    assert_eq!(first.rows.len(), 1);
    assert!(!first.done);
    let mut second_request = request(namespace, 10, 2, BODY + 96);
    second_request.after = first.next;
    let second = page_on(&tx, second_request).await?;
    assert_eq!(second.rows.len(), 1);
    assert_eq!(second.rows[0].counter, 2);
    assert!(!second.done);
    assert!(
        page_on(&tx, request(namespace, 10, 2, BODY + 95))
            .await
            .is_err()
    );

    let mut final_request = request(namespace, 10, 1, BODY + 96);
    final_request.after = Some(sync_orm::causal_candidates::Cursor {
        object: Uuid::from_u128(48),
        writer,
        counter: 39,
    });
    let last = page_on(&tx, final_request).await?;
    assert_eq!(last.rows.len(), 1);
    assert_eq!(last.rows[0].counter, 40);
    assert!(last.done);
    tx.rollback().await?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Notes;
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Note(String);
impl sync::SyncModel for Notes {
    type Collection = u8;
    type Payload = Note;
    fn model_id() -> &'static str {
        "batch-regression"
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
    fn validate(_: &sync::ObjectKey<Self>, _: &Option<Note>) -> sync::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn pinned_public_snapshot_batches_leaf_reads_across_concurrent_changes() -> Result<()> {
    use std::collections::BTreeMap;
    use sync::{SnapshotSource, SyncStore};
    use sync_orm::{causal_capabilities::CausalEndpoint, causal_store::CausalReplica};
    let mut db = fixtures::fresh_database().await?;
    let candidate_selects = Arc::new(AtomicUsize::new(0));
    let measured = candidate_selects.clone();
    db.set_metric_callback(move |info| {
        if info.statement.sql.starts_with("SELECT")
            && info.statement.sql.contains("\"sync_candidates\"")
        {
            measured.fetch_add(1, Ordering::SeqCst);
        }
    });
    let namespace = Uuid::from_u128(200);
    let endpoint = CausalEndpoint::new(
        CausalReplica::<Notes>::create_empty(db.clone(), namespace, Uuid::from_u128(201)).await?,
    );
    let mut expected = BTreeMap::new();
    for index in 0..40_u128 {
        let id = Uuid::from_u128(index + 10);
        let value = format!(
            "captured-{index:02}-{}",
            "x".repeat((index as usize % 5 + 1) * 19)
        );
        endpoint
            .write_local(
                sync::ObjectKey { collection: 1, id },
                Some(Note(value.clone())),
            )
            .await?;
        expected.insert(id, value);
    }
    let captured_head = endpoint.head().await?;
    let request_id = Uuid::new_v4();
    let scope = sync::Scope {
        collections: [1].into_iter().collect(),
    };
    let work = sync::WorkLimits {
        max_requests: 1,
        max_bytes: 1024 * 1024,
        max_depth: 32,
        max_rows_scanned: 512,
        max_mutations: 65536,
    };
    let cursor = match endpoint
        .open_snapshot(request_id, scope.clone(), work, None)
        .await
        .context("snapshot admission")?
    {
        sync::WorkOutcome::Incomplete { cursor } => cursor,
        _ => anyhow::bail!("fixture did not pause after capturing its state"),
    };
    endpoint
        .write_local(
            sync::ObjectKey {
                collection: 1,
                id: Uuid::from_u128(10),
            },
            Some(Note("replacement".into())),
        )
        .await?;
    endpoint
        .write_local(
            sync::ObjectKey {
                collection: 1,
                id: Uuid::from_u128(11),
            },
            None,
        )
        .await?;
    endpoint
        .write_local(
            sync::ObjectKey {
                collection: 1,
                id: Uuid::from_u128(1000),
            },
            Some(Note("new".into())),
        )
        .await?;
    candidate_selects.store(0, Ordering::SeqCst);
    let mut cursor = Some(cursor);
    let snapshot = loop {
        match endpoint
            .open_snapshot(request_id, scope.clone(), work, cursor)
            .await
            .context("snapshot continuation")?
        {
            sync::WorkOutcome::Incomplete { cursor: next } => cursor = Some(next),
            sync::WorkOutcome::Ready(snapshot) => break snapshot,
        }
    };
    let open_selects = candidate_selects.swap(0, Ordering::SeqCst);
    assert_eq!(snapshot.head, captured_head);
    assert_eq!(snapshot.roots.len(), 1);
    assert_eq!(snapshot.roots[0].candidate_count, 40);
    eprintln!("snapshot candidate SELECTs={open_selects} rows=40");
    assert!(
        open_selects > 0 && open_selects <= 8,
        "{open_selects} candidate SELECTs while building a 40-item snapshot"
    );

    let range = snapshot.roots[0].range.clone();
    let full = endpoint
        .leaf(
            snapshot.snapshot_id,
            range.clone(),
            None,
            sync::PageLimits {
                max_scanned: 40,
                max_bytes: 65536,
            },
        )
        .await
        .context("full snapshot leaf")?;
    let leaf_selects = candidate_selects.swap(0, Ordering::SeqCst);
    eprintln!("snapshot leaf candidate SELECTs={leaf_selects} rows=40");
    assert_eq!(full.candidates.len(), 40);
    assert!(full.done);
    assert!(
        leaf_selects > 0 && leaf_selects <= 4,
        "{leaf_selects} candidate SELECTs for a 40-item leaf"
    );
    let observed: BTreeMap<_, _> = full
        .candidates
        .iter()
        .map(|v| (v.key.id, v.payload.0.clone()))
        .collect();
    assert_eq!(observed, expected);

    let one = endpoint
        .leaf(
            snapshot.snapshot_id,
            range.clone(),
            None,
            sync::PageLimits {
                max_scanned: 1,
                max_bytes: 65536,
            },
        )
        .await?;
    assert_eq!(one.candidates.len(), 1);
    assert!(!one.done);
    let exact_size = serde_json::to_vec(&one)?.len() as u64;
    let exact = endpoint
        .leaf(
            snapshot.snapshot_id,
            range.clone(),
            None,
            sync::PageLimits {
                max_scanned: 1,
                max_bytes: exact_size,
            },
        )
        .await?;
    assert_eq!(exact, one);
    assert!(
        endpoint
            .leaf(
                snapshot.snapshot_id,
                range.clone(),
                None,
                sync::PageLimits {
                    max_scanned: 1,
                    max_bytes: exact_size - 1
                }
            )
            .await
            .is_err()
    );

    let mut cursor = None;
    let mut paged = BTreeMap::new();
    let mut pages = 0;
    loop {
        let limits = sync::PageLimits {
            max_scanned: 8,
            max_bytes: 2048,
        };
        let page = endpoint
            .leaf(snapshot.snapshot_id, range.clone(), cursor.clone(), limits)
            .await
            .context("bounded snapshot leaf")?;
        assert!(serde_json::to_vec(&page)?.len() as u64 <= limits.max_bytes);
        for candidate in &page.candidates {
            assert!(
                paged
                    .insert(candidate.key.id, candidate.payload.0.clone())
                    .is_none()
            );
        }
        pages += 1;
        if page.done {
            break;
        }
        assert_ne!(page.next, cursor);
        cursor = page.next;
        anyhow::ensure!(pages < 100, "leaf did not terminate");
    }
    assert!(pages > 1);
    assert_eq!(paged, expected);

    // Batched reads must still reject corrupt retained data even when its size
    // matches the previously admitted header.
    let object = Uuid::from_u128(12).as_bytes().to_vec();
    let stored = db
        .query_one(
            &Query::select()
                .columns(["body", "digest"].map(Alias::new))
                .from(Alias::new("sync_candidates"))
                .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
                .and_where(Expr::col(Alias::new("object")).eq(object.clone()))
                .to_owned(),
        )
        .await?
        .expect("captured candidate missing");
    let body: Vec<u8> = stored.try_get_by_index(0)?;
    let digest: Vec<u8> = stored.try_get_by_index(1)?;
    let mut malformed = body.clone();
    *malformed.last_mut().expect("empty candidate") ^= 1;
    db.execute(
        &Query::update()
            .table(Alias::new("sync_candidates"))
            .value(Alias::new("body"), malformed)
            .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("object")).eq(object.clone()))
            .to_owned(),
    )
    .await?;
    let limits = sync::PageLimits {
        max_scanned: 40,
        max_bytes: 65536,
    };
    assert!(
        endpoint
            .leaf(snapshot.snapshot_id, range.clone(), None, limits)
            .await
            .is_err()
    );
    let corrupt_request = Uuid::new_v4();
    let corrupt_work = sync::WorkLimits {
        max_requests: 64,
        ..work
    };
    let mut resume = None;
    let mut rejected = false;
    for _ in 0..2000 {
        match endpoint
            .open_snapshot(corrupt_request, scope.clone(), corrupt_work, resume)
            .await
        {
            Ok(sync::WorkOutcome::Incomplete { cursor }) => resume = Some(cursor),
            Ok(sync::WorkOutcome::Ready(_)) => anyhow::bail!("corrupt source produced a snapshot"),
            Err(sync::SyncError::Storage(message)) if message.contains("line 1 column") => {
                rejected = true;
                break;
            }
            Err(error) => {
                let row = db
                    .query_one(
                        &Query::select()
                            .column(Alias::new("build_state"))
                            .from(Alias::new("sync_leases"))
                            .and_where(
                                Expr::col(Alias::new("namespace"))
                                    .eq(namespace.as_bytes().to_vec()),
                            )
                            .and_where(
                                Expr::col(Alias::new("id")).eq(corrupt_request.as_bytes().to_vec()),
                            )
                            .to_owned(),
                    )
                    .await?;
                let state = row
                    .map(|row| row.try_get_by_index::<Option<Vec<u8>>>(0))
                    .transpose()?
                    .flatten();
                anyhow::bail!(
                    "unexpected snapshot build error: {error:?}; state={}",
                    state
                        .map(|v| String::from_utf8_lossy(&v).into_owned())
                        .unwrap_or_default()
                );
            }
        }
    }
    assert!(
        rejected,
        "corrupt candidate was not rejected during source build"
    );
    endpoint.release_request(corrupt_request).await?;
    db.execute(
        &Query::update()
            .table(Alias::new("sync_candidates"))
            .value(Alias::new("body"), body)
            .value(Alias::new("digest"), vec![0_u8; 32])
            .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("object")).eq(object.clone()))
            .to_owned(),
    )
    .await?;
    assert!(
        endpoint
            .leaf(snapshot.snapshot_id, range.clone(), None, limits)
            .await
            .is_err()
    );
    db.execute(
        &Query::update()
            .table(Alias::new("sync_candidates"))
            .value(Alias::new("digest"), digest)
            .and_where(Expr::col(Alias::new("namespace")).eq(namespace.as_bytes().to_vec()))
            .and_where(Expr::col(Alias::new("object")).eq(object))
            .to_owned(),
    )
    .await?;
    assert_eq!(
        endpoint
            .leaf(snapshot.snapshot_id, range, None, limits)
            .await?
            .candidates
            .len(),
        40
    );
    endpoint.release_request(request_id).await?;
    Ok(())
}
