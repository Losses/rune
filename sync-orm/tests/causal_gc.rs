use anyhow::Result;
use sea_orm::{
    ConnectionTrait, Database, TransactionTrait,
    sea_query::{Alias, Expr, Query},
};
use sync_orm::{causal_gc::collect_one_on, causal_schema};
use uuid::Uuid;

#[tokio::test]
async fn graph_reclamation_respects_roots_edges_and_budgets() -> Result<()> {
    let path = std::env::temp_dir().join(format!("rune-gc-{}.sqlite", Uuid::new_v4()));
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let db = Database::connect(&url).await?;
    causal_schema::install_fresh(&db).await?;
    let other = Database::connect(&url).await?;
    let n = Uuid::new_v4();
    let parent = vec![1_u8; 32];
    let child = vec![2_u8; 32];
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
            n.as_bytes().to_vec().into(),
            "notes".into(),
            "1".into(),
            "1".into(),
            Uuid::new_v4().as_bytes().to_vec().into(),
            Uuid::new_v4().as_bytes().to_vec().into(),
            0_i64.into(),
            vec![0_u8; 32].into(),
            0_i64.into(),
            0_i64.into(),
            0_i64.into(),
            vec![0_u8; 32].into(),
            0_i64.into(),
            0_i64.into(),
        ])
        .to_owned();
    db.execute(&meta).await?;
    for digest in [&parent, &child] {
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
                n.as_bytes().to_vec().into(),
                digest.clone().into(),
                1_i64.into(),
                digest.clone().into(),
                5_i64.into(),
                1_i64.into(),
                1_i64.into(),
                Vec::<u8>::new().into(),
            ])
            .to_owned();
        db.execute(&insert).await?;
    }
    let edge = Query::insert()
        .into_table(Alias::new("sync_manifest_edges"))
        .columns(["namespace", "parent_manifest", "child_manifest"].map(Alias::new))
        .values_panic([
            n.as_bytes().to_vec().into(),
            parent.clone().into(),
            child.clone().into(),
        ])
        .to_owned();
    db.execute(&edge).await?;
    let root_lease = Uuid::new_v4();
    let tx = db.begin().await?;
    sync_orm::causal_leases::create_build_on(
        &tx,
        sync_orm::causal_leases::BuildLeaseRequest {
            namespace: n,
            id: root_lease,
            scope: vec![1],
            now_ms: 0,
            expires_at_ms: 100,
            max_active: 10,
            max_ttl_ms: 1000,
        },
    )
    .await?;
    sync_orm::causal_leases::pin_root_on(&tx, n, root_lease, [1; 32], 1).await?;
    tx.commit().await?;
    let tx = db.begin().await?;
    assert!(!collect_one_on(&tx, n, [1; 32], 10, 10).await?);
    assert!(!collect_one_on(&tx, n, [2; 32], 10, 10).await?);
    tx.commit().await?;
    let tx = db.begin().await?;
    sync_orm::causal_leases::revoke_on(&tx, n, root_lease).await?;
    tx.commit().await?;
    let tx = db.begin().await?;
    assert!(collect_one_on(&tx, n, [1; 32], 1, 10).await.is_err());
    tx.rollback().await?;
    let tx = db.begin().await?;
    let first = sync_orm::causal_gc::step_on(&tx, n, None, 1, 10, 256).await?;
    assert!(first.reclaimed && !first.done);
    tx.commit().await?;
    let tx = db.begin().await?;
    let second = sync_orm::causal_gc::step_on(&tx, n, first.next, 1, 10, 256).await?;
    assert!(second.reclaimed && !second.done);
    tx.commit().await?;
    let tx = db.begin().await?;
    let end = sync_orm::causal_gc::step_on(&tx, n, second.next, 1, 10, 256).await?;
    assert!(end.done && !end.reclaimed);
    tx.commit().await?;
    let query = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sync_manifests"))
        .to_owned();
    assert!(db.query_one(&query).await?.is_none());
    // A partially reclaimed manifest remains marked across transactions; a
    // stale builder cannot reattach it, and each step removes at most one row.
    let reclaim_digest = [17_u8; 32];
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
            n.as_bytes().to_vec().into(),
            reclaim_digest.to_vec().into(),
            7_i64.into(),
            vec![18_u8; 32].into(),
            5_i64.into(),
            0_i64.into(),
            5_i64.into(),
            Vec::<u8>::new().into(),
        ])
        .to_owned();
    db.execute(&insert).await?;
    for ordinal in 0..5_i64 {
        let insert = Query::insert()
            .into_table(Alias::new("sync_chunks"))
            .columns(
                [
                    "namespace",
                    "manifest",
                    "ordinal",
                    "offset",
                    "digest",
                    "bytes",
                ]
                .map(Alias::new),
            )
            .values_panic([
                n.as_bytes().to_vec().into(),
                reclaim_digest.to_vec().into(),
                ordinal.into(),
                ordinal.into(),
                vec![19_u8; 32].into(),
                vec![1_u8].into(),
            ])
            .to_owned();
        db.execute(&insert).await?;
    }
    let first_lease = Uuid::new_v4();
    let tx = other.begin().await?;
    sync_orm::causal_leases::create_build_on(
        &tx,
        sync_orm::causal_leases::BuildLeaseRequest {
            namespace: n,
            id: first_lease,
            scope: vec![1],
            now_ms: 0,
            expires_at_ms: 100,
            max_active: 10,
            max_ttl_ms: 1000,
        },
    )
    .await?;
    sync_orm::causal_leases::pin_root_on(&tx, n, first_lease, reclaim_digest, 1).await?;
    tx.commit().await?;
    let tx = db.begin().await?;
    assert_eq!(
        sync_orm::causal_gc::collect_incremental_on(&tx, n, reclaim_digest, 1, 256).await?,
        sync_orm::causal_gc::IncrementalReclaim::LiveOrMissing
    );
    tx.commit().await?;
    let tx = other.begin().await?;
    sync_orm::causal_leases::revoke_on(&tx, n, first_lease).await?;
    tx.commit().await?;
    let tx = db.begin().await?;
    assert_eq!(
        sync_orm::causal_gc::collect_incremental_on(&tx, n, reclaim_digest, 1, 256).await?,
        sync_orm::causal_gc::IncrementalReclaim::More
    );
    tx.commit().await?;
    let tx = other.begin().await?;
    let stale_lease = Uuid::new_v4();
    sync_orm::causal_leases::create_build_on(
        &tx,
        sync_orm::causal_leases::BuildLeaseRequest {
            namespace: n,
            id: stale_lease,
            scope: vec![1],
            now_ms: 0,
            expires_at_ms: 100,
            max_active: 10,
            max_ttl_ms: 1000,
        },
    )
    .await?;
    assert!(
        sync_orm::causal_leases::pin_root_on(&tx, n, stale_lease, reclaim_digest, 1)
            .await
            .is_err()
    );
    tx.rollback().await?;
    let mut steps = 0;
    loop {
        let tx = db.begin().await?;
        let result =
            sync_orm::causal_gc::collect_incremental_on(&tx, n, reclaim_digest, 1, 256).await?;
        tx.commit().await?;
        steps += 1;
        if result == sync_orm::causal_gc::IncrementalReclaim::Deleted {
            break;
        }
        assert_eq!(result, sync_orm::causal_gc::IncrementalReclaim::More);
        assert!(steps < 20);
    }
    assert_eq!(steps, 6);
    db.close().await?;
    other.close().await?;
    std::fs::remove_file(path)?;
    Ok(())
}
