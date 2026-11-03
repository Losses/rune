#[path = "support/fixtures.rs"]
mod fixtures;

use anyhow::Result;
use sea_orm::sea_query::ColumnDef;
use sea_orm::sea_query::ExprTrait;
use sea_orm::sea_query::Table;
use sea_orm::{
    ConnectionTrait, Database, TransactionTrait,
    sea_query::{Alias, Expr, Query},
};
use sync_orm::causal_schema;

#[tokio::test]
async fn fresh_layout_is_complete_and_legacy_tables_are_never_replaced() -> Result<()> {
    let db = fixtures::fresh_database().await?;
    for name in [
        "sync_meta",
        "sync_candidates",
        "sync_journal",
        "sync_checkpoints",
        "sync_observed",
        "sync_manifests",
        "sync_chunks",
        "sync_manifest_edges",
        "sync_leases",
    ] {
        let statement = Query::select()
            .expr(Expr::val(1))
            .from(Alias::new("sqlite_master"))
            .and_where(ExprTrait::eq(Expr::col(Alias::new("name")), name))
            .to_owned();
        assert!(db.query_one(&statement).await?.is_some(), "missing {name}");
    }
    assert!(causal_schema::install_fresh(&db).await.is_err());
    let old = Database::connect("sqlite::memory:").await?;
    old.execute(
        &Table::create()
            .table(Alias::new("sync_meta"))
            .col(ColumnDef::new(Alias::new("unexpected_layout")).integer())
            .to_owned(),
    )
    .await?;
    let notes = Table::create()
        .table(Alias::new("notes"))
        .col(
            ColumnDef::new(Alias::new("id"))
                .integer()
                .not_null()
                .primary_key(),
        )
        .col(ColumnDef::new(Alias::new("text")).text().not_null())
        .to_owned();
    old.execute(&notes).await?;
    let insert = Query::insert()
        .into_table(Alias::new("notes"))
        .columns([Alias::new("id"), Alias::new("text")])
        .values_panic([1.into(), "user data".into()])
        .to_owned();
    old.execute(&insert).await?;
    assert!(causal_schema::install_fresh(&old).await.is_err());
    let persisted = Query::select()
        .column(Alias::new("text"))
        .from(Alias::new("notes"))
        .to_owned();
    assert_eq!(
        old.query_one(&persisted)
            .await?
            .unwrap()
            .try_get_by_index::<String>(0)?,
        "user data"
    );
    let tx = old.begin().await?;
    let before = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sync_meta"))
        .limit(1)
        .to_owned();
    assert!(tx.query_one(&before).await?.is_none());
    tx.rollback().await?;
    // An incompatible reserved table must leave the database untouched.
    let candidates = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sqlite_master"))
        .and_where(ExprTrait::eq(
            Expr::col(Alias::new("name")),
            "sync_candidates",
        ))
        .to_owned();
    assert!(old.query_one(&candidates).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn durable_build_pin_is_visible_to_an_independent_connection() -> Result<()> {
    use sea_orm::sea_query::Query;
    use sync_orm::causal_leases::{
        checkpoint_on, create_build_on, minimum_pins_on, pin_root_on, publish_on, ready_on,
        revoke_expired_on,
    };
    use uuid::Uuid;
    let path = std::env::temp_dir().join(format!("sync-orm-pin-{}.sqlite", Uuid::new_v4()));
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let a = Database::connect(&url).await?;
    causal_schema::install_fresh(&a).await?;
    let b = Database::connect(&url).await?;
    let namespace = Uuid::new_v4();
    let generation = Uuid::new_v4();
    let id = Uuid::new_v4();
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
            namespace.as_bytes().to_vec().into(),
            "notes".into(),
            "1".into(),
            "1".into(),
            Uuid::new_v4().as_bytes().to_vec().into(),
            generation.as_bytes().to_vec().into(),
            5_i64.into(),
            vec![7_u8; 32].into(),
            8_i64.into(),
            0_i64.into(),
            0_i64.into(),
            vec![0_u8; 32].into(),
            0_i64.into(),
            0_i64.into(),
        ])
        .to_owned();
    a.execute(&meta).await?;
    let tx = a.begin().await?;
    let built = create_build_on(
        &tx,
        sync_orm::causal_leases::BuildLeaseRequest {
            namespace,
            id,
            scope: vec![1],
            now_ms: 10,
            expires_at_ms: 100,
            max_active: 1,
            max_ttl_ms: 1000,
        },
    )
    .await?;
    assert_eq!((built.head, built.state_seq), (5, 8));
    tx.commit().await?;
    let tx = b.begin().await?;
    assert_eq!(minimum_pins_on(&tx, namespace).await?, Some((8, 5)));
    assert!(ready_on(&tx, namespace, id, 11).await.is_err());
    checkpoint_on(&tx, namespace, id, None, b"checkpoint-1".to_vec(), 11).await?;
    assert!(
        checkpoint_on(&tx, namespace, id, None, b"stale".to_vec(), 11)
            .await
            .is_err()
    );

    assert!(
        create_build_on(
            &tx,
            sync_orm::causal_leases::BuildLeaseRequest {
                namespace,
                id: Uuid::new_v4(),
                scope: vec![1],
                now_ms: 11,
                expires_at_ms: 99,
                max_active: 1,
                max_ttl_ms: 1000
            }
        )
        .await
        .is_err()
    );
    tx.commit().await?;
    let tx = a.begin().await?;
    checkpoint_on(
        &tx,
        namespace,
        id,
        Some(b"checkpoint-1"),
        b"checkpoint-2".to_vec(),
        11,
    )
    .await?;
    tx.commit().await?;
    let root = [42_u8; 32];
    let tx = a.begin().await?;
    let manifest = Query::insert()
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
            namespace.as_bytes().to_vec().into(),
            root.to_vec().into(),
            1_i64.into(),
            root.to_vec().into(),
            0_i64.into(),
            0_i64.into(),
            0_i64.into(),
            Vec::<u8>::new().into(),
        ])
        .to_owned();
    tx.execute(&manifest).await?;
    pin_root_on(&tx, namespace, id, root, 12).await?;
    publish_on(&tx, namespace, id, vec![9], 12).await?;
    tx.commit().await?;
    let tx = b.begin().await?;
    let ready = ready_on(&tx, namespace, id, 13).await?;
    assert_eq!(minimum_pins_on(&tx, namespace).await?, Some((8, 5)));
    assert_eq!(ready.retained_root, Some(root.to_vec()));
    assert!(!sync_orm::causal_gc::collect_one_on(&tx, namespace, root, 10, 10).await?);
    assert!(publish_on(&tx, namespace, id, vec![8], 13).await.is_err());
    assert!(ready_on(&tx, namespace, id, 100).await.is_err());
    tx.rollback().await?;
    let tx = a.begin().await?;
    assert_eq!(revoke_expired_on(&tx, namespace, 100, 1).await?, 1);
    assert_eq!(minimum_pins_on(&tx, namespace).await?, None);
    tx.commit().await?;
    let tx = b.begin().await?;
    assert!(ready_on(&tx, namespace, id, 14).await.is_err());
    assert!(sync_orm::causal_gc::collect_one_on(&tx, namespace, root, 10, 10).await?);
    tx.commit().await?;
    a.close().await?;
    b.close().await?;
    std::fs::remove_file(path)?;
    Ok(())
}
