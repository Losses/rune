use anyhow::Result;
use sea_orm::{
    ConnectionTrait, Database, ExprTrait, TransactionTrait,
    sea_query::{Alias, Expr, Query},
};
use sync::protocol::{
    Fragment, ManifestKind, ManifestRef, content_digest, fragment_digest, manifest_digest,
};
use sync_orm::{
    causal_manifests::{publish_on, read_on},
    causal_schema,
};
use uuid::Uuid;

fn single(bytes: &[u8]) -> Result<(ManifestRef, Vec<Fragment>)> {
    let mut m = ManifestRef {
        kind: ManifestKind::Context,
        manifest_digest: [0; 32],
        content_digest: content_digest(bytes)?,
        encoded_bytes: bytes.len() as u64,
        item_count: 1,
        fragment_count: 1,
    };
    m.manifest_digest = manifest_digest(&m)?;
    let mut f = Fragment {
        manifest: m.clone(),
        ordinal: 0,
        offset: 0,
        bytes: bytes.to_vec(),
        digest: [0; 32],
    };
    f.digest = fragment_digest(&f)?;
    Ok((m, vec![f]))
}
#[tokio::test]
async fn complete_validated_manifests_publish_idempotently_and_atomically() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
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
            0_i64.into(),
            0_i64.into(),
            0_i64.into(),
            vec![0_u8; 32].into(),
            0_i64.into(),
            0_i64.into(),
        ])
        .to_owned();
    db.execute(&meta).await?;
    let oversized = vec![1_u8; sync::protocol::MAX_PAGE_BYTES as usize + 1];
    let (too_large, oversized_fragment) = single(&oversized)?;
    let tx = db.begin().await?;
    assert!(
        publish_on(
            &tx,
            ns,
            &too_large,
            &oversized_fragment,
            sync::protocol::MAX_LOGICAL_BYTES,
            |_| Ok(vec![])
        )
        .await
        .is_err()
    );
    tx.rollback().await?;
    let (m, fragments) = single(b"causal context")?;
    let tx = db.begin().await?;
    assert!(
        publish_on(&tx, ns, &m, &fragments, 1024, |_| anyhow::bail!(
            "invalid context"
        ))
        .await
        .is_err()
    );
    tx.rollback().await?;
    let q = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sync_manifests"))
        .to_owned();
    assert!(db.query_one(&q).await?.is_none());
    let tx = db.begin().await?;
    assert!(publish_on(&tx, ns, &m, &fragments, 1024, |_| Ok(vec![])).await?);
    tx.commit().await?;
    let tx = db.begin().await?;
    let (loaded, body) = read_on(&tx, ns, m.manifest_digest, 1024, 10).await?;
    assert_eq!(loaded, m);
    assert_eq!(body, b"causal context");
    assert!(!publish_on(&tx, ns, &m, &fragments, 1024, |_| Ok(vec![])).await?);
    tx.commit().await?;
    let (parent, parts) = single(b"parent")?;
    let tx = db.begin().await?;
    assert!(
        publish_on(&tx, ns, &parent, &parts, 1024, |_| Ok(vec![[9; 32]]))
            .await
            .is_err()
    );
    tx.rollback().await?;
    let tx = db.begin().await?;
    assert!(
        publish_on(&tx, ns, &parent, &parts, 1024, |_| Ok(vec![
            m.manifest_digest
        ]))
        .await?
    );
    tx.commit().await?;
    let q = Query::select()
        .expr(Expr::val(1))
        .from(Alias::new("sync_manifest_edges"))
        .and_where(Expr::col(Alias::new("parent_manifest")).eq(parent.manifest_digest.to_vec()))
        .to_owned();
    assert!(db.query_one(&q).await?.is_some());
    Ok(())
}

#[tokio::test]
async fn bounded_fragment_batches_verify_integrity_and_limits() -> Result<()> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use sync_orm::causal_manifests::fragment_body;
    let mut db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
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
                0_i64.into(),
                0_i64.into(),
                0_i64.into(),
                vec![0_u8; 32].into(),
                0_i64.into(),
                0_i64.into(),
            ])
            .to_owned(),
    )
    .await?;
    let body: Vec<u8> = (0..12000).map(|n| (n % 251) as u8).collect();
    let (manifest, fragments) = fragment_body(ManifestKind::Context, &body, 0, 100)?;
    assert_eq!(fragments.len(), 120);
    let (single_manifest, single_fragments) = single(b"one fragment")?;
    let large_body = vec![9_u8; sync::protocol::MAX_PAGE_BYTES as usize + 2];
    let (large_manifest, large_fragments) = fragment_body(
        ManifestKind::Context,
        &large_body,
        0,
        sync::protocol::MAX_PAGE_BYTES as usize / 2 + 1,
    )?;
    assert_eq!(large_fragments.len(), 2);
    let tx = db.begin().await?;
    publish_on(&tx, ns, &manifest, &fragments, 65536, |_| Ok(vec![])).await?;
    publish_on(&tx, ns, &single_manifest, &single_fragments, 65536, |_| {
        Ok(vec![])
    })
    .await?;
    publish_on(
        &tx,
        ns,
        &large_manifest,
        &large_fragments,
        8 * 1024 * 1024,
        |_| Ok(vec![]),
    )
    .await?;
    tx.commit().await?;
    let enabled = Arc::new(AtomicBool::new(false));
    let count = Arc::new(AtomicUsize::new(0));
    let watch = enabled.clone();
    let queries = count.clone();
    db.set_metric_callback(move |info| {
        if watch.load(Ordering::Relaxed)
            && info
                .statement
                .sql
                .to_ascii_lowercase()
                .starts_with("select")
        {
            queries.fetch_add(1, Ordering::Relaxed);
        }
    });
    let tx = db.begin().await?;
    enabled.store(true, Ordering::Relaxed);
    let (loaded, result) = read_on(&tx, ns, manifest.manifest_digest, 65536, 120).await?;
    enabled.store(false, Ordering::Relaxed);
    let batch_queries = count.swap(0, Ordering::Relaxed);
    assert_eq!(loaded, manifest);
    assert_eq!(result, body);
    assert_eq!(batch_queries, 5); // metadata plus two header/body pages
    enabled.store(true, Ordering::Relaxed);
    assert_eq!(
        read_on(&tx, ns, single_manifest.manifest_digest, 1024, 1)
            .await?
            .1,
        b"one fragment"
    );
    enabled.store(false, Ordering::Relaxed);
    let single_queries = count.swap(0, Ordering::Relaxed);
    assert_eq!(single_queries, 2); // metadata and exact-length fragment
    enabled.store(true, Ordering::Relaxed);
    assert_eq!(
        read_on(&tx, ns, large_manifest.manifest_digest, 8 * 1024 * 1024, 2)
            .await?
            .1,
        large_body
    );
    enabled.store(false, Ordering::Relaxed);
    assert_eq!(count.swap(0, Ordering::Relaxed), 5); // body page budget forces two pages
    eprintln!(
        "manifest_fragments=120 select_queries={batch_queries} single_fragment_select_queries={single_queries}"
    );
    assert!(
        read_on(
            &tx,
            ns,
            manifest.manifest_digest,
            body.len() as u64 - 1,
            120
        )
        .await
        .is_err()
    );
    assert!(
        read_on(&tx, ns, manifest.manifest_digest, 65536, 119)
            .await
            .is_err()
    );
    tx.commit().await?;
    for corruption in 0..8 {
        let tx = db.begin().await?;
        let filtered = || {
            Query::update()
                .table(Alias::new("sync_chunks"))
                .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                .and_where(Expr::col(Alias::new("manifest")).eq(manifest.manifest_digest.to_vec()))
                .and_where(Expr::col(Alias::new("ordinal")).eq(0_i64))
                .to_owned()
        };
        match corruption {
            0 => {
                tx.execute(
                    &Query::delete()
                        .from_table(Alias::new("sync_chunks"))
                        .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                        .and_where(
                            Expr::col(Alias::new("manifest")).eq(manifest.manifest_digest.to_vec()),
                        )
                        .and_where(Expr::col(Alias::new("ordinal")).eq(0_i64))
                        .to_owned(),
                )
                .await?;
            }
            1 => {
                let mut q = filtered();
                q.value(Alias::new("bytes"), vec![42_u8; 100]);
                tx.execute(&q).await?;
            }
            2 => {
                let mut q = filtered();
                q.value(
                    Alias::new("bytes"),
                    vec![0_u8; sync::protocol::MAX_PAGE_BYTES as usize + 1],
                );
                tx.execute(&q).await?;
            }
            3 => {
                let mut q = filtered();
                q.value(Alias::new("offset"), 1_i64);
                tx.execute(&q).await?;
            }
            4 => {
                let mut q = filtered();
                q.value(Alias::new("ordinal"), 200_i64);
                tx.execute(&q).await?;
            }
            5 => {
                let mut altered = fragments[0].clone();
                altered.bytes[0] ^= 1;
                altered.digest = fragment_digest(&altered)?;
                let mut q = filtered();
                q.value(Alias::new("bytes"), altered.bytes)
                    .value(Alias::new("digest"), altered.digest.to_vec());
                tx.execute(&q).await?;
            }
            6 => {
                let mut q = filtered();
                q.value(Alias::new("bytes"), vec![0_u8; 99]);
                tx.execute(&q).await?;
            }
            7 => {
                let mut q = filtered();
                q.value(Alias::new("digest"), vec![0_u8; 4 * 1024 * 1024]);
                tx.execute(&q).await?;
            }
            _ => unreachable!(),
        }
        assert!(
            read_on(&tx, ns, manifest.manifest_digest, 65536, 120)
                .await
                .is_err(),
            "corruption case {corruption}"
        );
        tx.rollback().await?;
    }
    for bytes in [
        vec![0_u8; b"one fragment".len()],
        vec![0_u8; sync::protocol::MAX_PAGE_BYTES as usize + 1],
    ] {
        let tx = db.begin().await?;
        tx.execute(
            &Query::update()
                .table(Alias::new("sync_chunks"))
                .value(Alias::new("bytes"), bytes)
                .and_where(Expr::col(Alias::new("namespace")).eq(ns.as_bytes().to_vec()))
                .and_where(
                    Expr::col(Alias::new("manifest")).eq(single_manifest.manifest_digest.to_vec()),
                )
                .and_where(Expr::col(Alias::new("ordinal")).eq(0_i64))
                .to_owned(),
        )
        .await?;
        assert!(
            read_on(&tx, ns, single_manifest.manifest_digest, 1024, 1)
                .await
                .is_err()
        );
        tx.rollback().await?;
    }
    Ok(())
}
