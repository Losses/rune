use anyhow::Result;
use sea_orm::{
    ConnectionTrait, Database, TransactionTrait,
    sea_query::{Alias, Query},
};
use sync_orm::{
    causal_context::{ContextScan, ScanResult, scan_on},
    causal_schema,
};
use uuid::Uuid;

#[tokio::test]
async fn qualified_context_preserves_gaps_and_budgeted_continuation() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
    let writer = Uuid::new_v4();
    let manifest = vec![1_u8; 32];
    let meta = Query::insert()
        .into_table(Alias::new("sync_meta"))
        .columns(
            [
                "namespace",
                "model",
                "schema",
                "codec",
                "replica",
                "generation",
                "head",
                "head_anchor",
                "state_seq",
                "revision_counter",
                "history_floor",
                "floor_anchor",
                "floor_state_seq",
                "retained_bytes",
            ]
            .map(Alias::new),
        )
        .values_panic([
            ns.as_bytes().to_vec().into(),
            "notes".into(),
            "1".into(),
            "1".into(),
            Uuid::new_v4().as_bytes().to_vec().into(),
            Uuid::new_v4().as_bytes().to_vec().into(),
            0_i64.into(),
            vec![0_u8; 32].into(),
            2_i64.into(),
            0_i64.into(),
            0_i64.into(),
            vec![0_u8; 32].into(),
            0_i64.into(),
            0_i64.into(),
        ])
        .to_owned();
    db.execute(&meta).await?;
    let insert = Query::insert()
        .into_table(Alias::new("sync_manifests"))
        .columns(
            [
                "namespace",
                "digest",
                "kind",
                "content_digest",
                "encoded_bytes",
                "item_count",
                "fragment_count",
                "metadata",
            ]
            .map(Alias::new),
        )
        .values_panic([
            ns.as_bytes().to_vec().into(),
            manifest.clone().into(),
            1_i64.into(),
            manifest.clone().into(),
            0_i64.into(),
            2_i64.into(),
            0_i64.into(),
            Vec::<u8>::new().into(),
        ])
        .to_owned();
    db.execute(&insert).await?;
    for (v, lo, hi) in [(1_i64, vec![0], vec![1]), (2_i64, vec![1], vec![2])] {
        let row = Query::insert()
            .into_table(Alias::new("sync_context_ranges"))
            .columns(
                [
                    "namespace",
                    "collection",
                    "state_seq",
                    "ordinal",
                    "lower_bound",
                    "upper_bound",
                    "context_manifest",
                ]
                .map(Alias::new),
            )
            .values_panic([
                ns.as_bytes().to_vec().into(),
                b"notes".to_vec().into(),
                v.into(),
                0_i64.into(),
                lo.into(),
                hi.into(),
                manifest.clone().into(),
            ])
            .to_owned();
        db.execute(&row).await?;
    }
    for (first, last) in [(1_i64, 2_i64), (4, 4)] {
        let row = Query::insert()
            .into_table(Alias::new("sync_context_intervals"))
            .columns(["namespace", "context_manifest", "writer", "first", "last"].map(Alias::new))
            .values_panic([
                ns.as_bytes().to_vec().into(),
                manifest.clone().into(),
                writer.as_bytes().to_vec().into(),
                first.into(),
                last.into(),
            ])
            .to_owned();
        db.execute(&row).await?;
    }
    let tx = db.begin().await?;
    let make =
        |counter| ContextScan::new(ns, b"notes".to_vec(), vec![1, 42], writer, counter, 0, 2);
    // An unpublished generation may coexist at the same state position; it
    // must not enter active reads even if its contents are invalid.
    let staged = Query::insert()
        .into_table(Alias::new("sync_context_ranges"))
        .columns(
            [
                "namespace",
                "collection",
                "basis_generation",
                "state_seq",
                "ordinal",
                "lower_bound",
                "upper_bound",
                "context_manifest",
            ]
            .map(Alias::new),
        )
        .values_panic([
            ns.as_bytes().to_vec().into(),
            b"notes".to_vec().into(),
            Uuid::new_v4().as_bytes().to_vec().into(),
            1_i64.into(),
            0_i64.into(),
            vec![0_u8].into(),
            vec![2_u8].into(),
            vec![9_u8].into(),
        ])
        .to_owned();
    tx.execute(&staged).await?;
    let ScanResult::Incomplete(next) = scan_on(&tx, make(4)?, 1).await? else {
        panic!("first range cannot decide");
    };
    assert!(matches!(scan_on(&tx, next, 1).await?, ScanResult::Observed));
    let ScanResult::Incomplete(next) = scan_on(&tx, make(3)?, 1).await? else {
        panic!("first range cannot decide");
    };
    let ScanResult::Incomplete(next) = scan_on(&tx, next, 1).await? else {
        panic!("second range does not observe gap");
    };
    assert!(matches!(scan_on(&tx, next, 1).await?, ScanResult::Absent));
    let ScanResult::Incomplete(stale_generation) = scan_on(&tx, make(4)?, 1).await? else {
        panic!("expected generation-bound continuation");
    };
    tx.execute(
        &Query::update()
            .table(Alias::new("sync_meta"))
            .value(
                Alias::new("baseline_generation"),
                Uuid::new_v4().as_bytes().to_vec(),
            )
            .to_owned(),
    )
    .await?;
    assert!(scan_on(&tx, stale_generation, 1).await.is_err());
    tx.execute(
        &Query::update()
            .table(Alias::new("sync_meta"))
            .value(
                Alias::new("baseline_generation"),
                Uuid::nil().as_bytes().to_vec(),
            )
            .to_owned(),
    )
    .await?;
    tx.execute(
        &Query::update()
            .table(Alias::new("sync_meta"))
            .value(Alias::new("floor_state_seq"), 1_i64)
            .to_owned(),
    )
    .await?;
    let missing_baseline = ContextScan::new(ns, b"notes".to_vec(), vec![1, 42], writer, 4, 1, 2)?;
    let ScanResult::Incomplete(next) = scan_on(&tx, missing_baseline, 1).await? else {
        panic!("baseline scan must continue");
    };
    assert!(scan_on(&tx, next, 1).await.is_err());
    tx.rollback().await?;
    // A resumable probe cannot keep its old cursor after a floor switch.
    let tx = db.begin().await?;
    let ScanResult::Incomplete(stale) = scan_on(&tx, make(4)?, 1).await? else {
        panic!("expected continuation");
    };
    tx.commit().await?;
    let tx = db.begin().await?;
    tx.execute(
        &Query::delete()
            .from_table(Alias::new("sync_context_ranges"))
            .to_owned(),
    )
    .await?;
    let baseline = Query::insert()
        .into_table(Alias::new("sync_context_ranges"))
        .columns(
            [
                "namespace",
                "collection",
                "state_seq",
                "ordinal",
                "lower_bound",
                "upper_bound",
                "context_manifest",
            ]
            .map(Alias::new),
        )
        .values_panic([
            ns.as_bytes().to_vec().into(),
            b"notes".to_vec().into(),
            2_i64.into(),
            0_i64.into(),
            vec![0_u8].into(),
            vec![2_u8].into(),
            manifest.clone().into(),
        ])
        .to_owned();
    tx.execute(&baseline).await?;
    tx.execute(
        &Query::update()
            .table(Alias::new("sync_meta"))
            .value(Alias::new("floor_state_seq"), 2_i64)
            .to_owned(),
    )
    .await?;
    tx.commit().await?;
    let tx = db.begin().await?;
    assert!(scan_on(&tx, stale, 1).await.is_err());
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn bounded_context_page_preserves_order_and_detects_corruption() -> Result<()> {
    use sea_orm::ExprTrait;
    use sea_orm::sea_query::Expr;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    let mut db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
    let writer = Uuid::new_v4();
    let manifest = vec![7_u8; 32];
    db.execute(
        &Query::insert()
            .into_table(Alias::new("sync_meta"))
            .columns(
                [
                    "namespace",
                    "model",
                    "schema",
                    "codec",
                    "replica",
                    "generation",
                    "head",
                    "head_anchor",
                    "state_seq",
                    "revision_counter",
                    "history_floor",
                    "floor_anchor",
                    "floor_state_seq",
                    "retained_bytes",
                ]
                .map(Alias::new),
            )
            .values_panic([
                ns.as_bytes().to_vec().into(),
                "notes".into(),
                "1".into(),
                "1".into(),
                Uuid::new_v4().as_bytes().to_vec().into(),
                Uuid::new_v4().as_bytes().to_vec().into(),
                0_i64.into(),
                vec![0_u8; 32].into(),
                50_i64.into(),
                0_i64.into(),
                0_i64.into(),
                vec![0_u8; 32].into(),
                0_i64.into(),
                0_i64.into(),
            ])
            .to_owned(),
    )
    .await?;
    db.execute(
        &Query::insert()
            .into_table(Alias::new("sync_manifests"))
            .columns(
                [
                    "namespace",
                    "digest",
                    "kind",
                    "content_digest",
                    "encoded_bytes",
                    "item_count",
                    "fragment_count",
                    "metadata",
                ]
                .map(Alias::new),
            )
            .values_panic([
                ns.as_bytes().to_vec().into(),
                manifest.clone().into(),
                2_i64.into(),
                manifest.clone().into(),
                0_i64.into(),
                1_i64.into(),
                0_i64.into(),
                Vec::<u8>::new().into(),
            ])
            .to_owned(),
    )
    .await?;
    db.execute(
        &Query::insert()
            .into_table(Alias::new("sync_context_intervals"))
            .columns(["namespace", "context_manifest", "writer", "first", "last"].map(Alias::new))
            .values_panic([
                ns.as_bytes().to_vec().into(),
                manifest.clone().into(),
                writer.as_bytes().to_vec().into(),
                1_i64.into(),
                1_i64.into(),
            ])
            .to_owned(),
    )
    .await?;
    for state in 1_i64..=50 {
        db.execute(
            &Query::insert()
                .into_table(Alias::new("sync_context_ranges"))
                .columns(
                    [
                        "namespace",
                        "collection",
                        "state_seq",
                        "ordinal",
                        "lower_bound",
                        "upper_bound",
                        "context_manifest",
                    ]
                    .map(Alias::new),
                )
                .values_panic([
                    ns.as_bytes().to_vec().into(),
                    b"notes".to_vec().into(),
                    state.into(),
                    0_i64.into(),
                    vec![0_u8].into(),
                    vec![2_u8].into(),
                    manifest.clone().into(),
                ])
                .to_owned(),
        )
        .await?;
    }
    let active = Arc::new(AtomicBool::new(false));
    let query_count = Arc::new(AtomicUsize::new(0));
    let callback_active = active.clone();
    let callback_count = query_count.clone();
    db.set_metric_callback(move |_| {
        if callback_active.load(Ordering::Relaxed) {
            callback_count.fetch_add(1, Ordering::Relaxed);
        }
    });
    let make =
        |counter| ContextScan::new(ns, b"notes".to_vec(), vec![1, 42], writer, counter, 0, 50);
    let tx = db.begin().await?;
    active.store(true, Ordering::Relaxed);
    let first = scan_on(&tx, make(2)?, 40).await?;
    active.store(false, Ordering::Relaxed);
    let first_queries = query_count.swap(0, Ordering::Relaxed);
    let ScanResult::Incomplete(next) = first else {
        panic!("expected bounded continuation")
    };
    assert_eq!(first_queries, 2);
    active.store(true, Ordering::Relaxed);
    assert!(matches!(
        scan_on(&tx, next.clone(), 40).await?,
        ScanResult::Absent
    ));
    active.store(false, Ordering::Relaxed);
    let second_queries = query_count.swap(0, Ordering::Relaxed);
    assert_eq!(second_queries, 2);
    eprintln!(
        "context_assertions=50 first_page=40 first_queries={first_queries} resume_queries={second_queries}"
    );
    let global = |counter, max_assertions| sync_orm::causal_context::GlobalObservation {
        namespace: ns,
        collection: b"notes",
        writer,
        counter,
        floor: 0,
        state_seq: 50,
        max_assertions,
    };
    active.store(true, Ordering::Relaxed);
    assert!(sync_orm::causal_context::globally_observed_on(&tx, global(1, 50)).await?);
    active.store(false, Ordering::Relaxed);
    assert_eq!(query_count.swap(0, Ordering::Relaxed), 2);
    assert!(!sync_orm::causal_context::globally_observed_on(&tx, global(2, 50)).await?);
    assert!(
        sync_orm::causal_context::globally_observed_on(&tx, global(1, 40))
            .await
            .is_err()
    );
    // A non-covering assertion need not have a retained manifest for this
    // key probe; global observation still checks every assertion.
    tx.execute(
        &Query::update()
            .table(Alias::new("sync_context_ranges"))
            .value(Alias::new("upper_bound"), vec![1_u8])
            .value(Alias::new("context_manifest"), vec![8_u8; 32])
            .and_where(Expr::col(Alias::new("state_seq")).eq(1_i64))
            .to_owned(),
    )
    .await?;
    assert!(matches!(
        scan_on(&tx, make(1)?, 50).await?,
        ScanResult::Observed
    ));
    assert!(
        sync_orm::causal_context::globally_observed_on(&tx, global(1, 50))
            .await
            .is_err()
    );
    tx.execute(
        &Query::update()
            .table(Alias::new("sync_context_ranges"))
            .value(Alias::new("upper_bound"), vec![2_u8])
            .value(Alias::new("context_manifest"), manifest.clone())
            .and_where(Expr::col(Alias::new("state_seq")).eq(1_i64))
            .to_owned(),
    )
    .await?;
    // Later invalid rows must not change an earlier observed result.
    tx.execute(
        &Query::update()
            .table(Alias::new("sync_context_ranges"))
            .value(Alias::new("context_manifest"), vec![3_u8])
            .and_where(Expr::col(Alias::new("state_seq")).eq(50_i64))
            .to_owned(),
    )
    .await?;
    assert!(matches!(
        scan_on(&tx, make(1)?, 50).await?,
        ScanResult::Observed
    ));
    assert!(scan_on(&tx, make(2)?, 50).await.is_err());
    tx.execute(
        &Query::update()
            .table(Alias::new("sync_context_ranges"))
            .value(Alias::new("context_manifest"), manifest.clone())
            .and_where(Expr::col(Alias::new("state_seq")).eq(50_i64))
            .to_owned(),
    )
    .await?;
    tx.execute(
        &Query::delete()
            .from_table(Alias::new("sync_manifests"))
            .to_owned(),
    )
    .await?;
    assert!(scan_on(&tx, make(2)?, 40).await.is_err());
    tx.rollback().await?;
    let tx = db.begin().await?;
    tx.execute(
        &Query::update()
            .table(Alias::new("sync_context_intervals"))
            .value(Alias::new("first"), 0_i64)
            .value(Alias::new("last"), 0_i64)
            .to_owned(),
    )
    .await?;
    assert!(scan_on(&tx, make(2)?, 40).await.is_err());
    tx.rollback().await?;
    let tx = db.begin().await?;
    tx.execute(
        &Query::update()
            .table(Alias::new("sync_meta"))
            .value(
                Alias::new("baseline_generation"),
                Uuid::new_v4().as_bytes().to_vec(),
            )
            .to_owned(),
    )
    .await?;
    assert!(scan_on(&tx, next, 40).await.is_err());
    tx.rollback().await?;
    let tx = db.begin().await?;
    tx.execute(
        &Query::update()
            .table(Alias::new("sync_meta"))
            .value(Alias::new("floor_state_seq"), 1_i64)
            .to_owned(),
    )
    .await?;
    tx.execute(
        &Query::insert()
            .into_table(Alias::new("sync_context_ranges"))
            .columns(
                [
                    "namespace",
                    "collection",
                    "state_seq",
                    "ordinal",
                    "lower_bound",
                    "upper_bound",
                    "context_manifest",
                ]
                .map(Alias::new),
            )
            .values_panic([
                ns.as_bytes().to_vec().into(),
                b"notes".to_vec().into(),
                1_i64.into(),
                1_i64.into(),
                vec![0_u8].into(),
                vec![2_u8].into(),
                manifest.into(),
            ])
            .to_owned(),
    )
    .await?;
    let overlap = ContextScan::new(ns, b"notes".to_vec(), vec![1, 42], writer, 2, 1, 50)?;
    assert!(scan_on(&tx, overlap, 40).await.is_err());
    tx.rollback().await?;
    Ok(())
}
