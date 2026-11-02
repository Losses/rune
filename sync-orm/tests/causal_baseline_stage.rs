use anyhow::Result;
use sea_orm::{Database, TransactionTrait};
use serde::{Deserialize, Serialize};
use sync::protocol::{self, ContextRef, ContextSegment, KeyRange, ObjectKey, SyncModel};
use sync_orm::{
    causal_compaction, causal_leases, causal_manifests, causal_schema, causal_store::CausalReplica,
};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Model;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Collection {
    Items,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Value(String);
impl SyncModel for Model {
    type Collection = Collection;
    type Payload = Value;
    fn model_id() -> &'static str {
        "items"
    }
    fn schema_id() -> &'static str {
        "1"
    }
    fn collections() -> Vec<Collection> {
        vec![Collection::Items]
    }
    fn collection_tag(_: &Collection) -> &'static str {
        "items"
    }
    fn collection_from_tag(tag: &str) -> Option<Collection> {
        (tag == "items").then_some(Collection::Items)
    }
    fn validate(_: &ObjectKey<Self>, _: &Option<Value>) -> protocol::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn staged_baseline_is_idempotent_contiguous_and_pinned() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    assert!(
        sync_orm::causal_store::list_namespace_ids_on(&db, 2)
            .await?
            .is_empty()
    );
    let ns = Uuid::new_v4();
    let _replica = CausalReplica::<Model>::create_empty(db.clone(), ns, Uuid::new_v4()).await?;
    assert_eq!(
        sync_orm::causal_store::list_namespace_ids_on(&db, 2).await?,
        vec![ns]
    );
    assert!(
        sync_orm::causal_store::list_namespace_ids_on(&db, 3)
            .await
            .is_err()
    );
    let lease = Uuid::new_v4();
    let output = Uuid::new_v4();
    let tx = db.begin().await?;
    causal_leases::create_build_on(
        &tx,
        sync_orm::causal_leases::BuildLeaseRequest {
            namespace: ns,
            id: lease,
            scope: vec![1],
            now_ms: 0,
            expires_at_ms: 100,
            max_active: 10,
            max_ttl_ms: 1000,
        },
    )
    .await?;
    causal_leases::claim_output_generation_on(&tx, ns, lease, output, 1).await?;
    let exact = protocol::ExactContext { writers: vec![] };
    let body = protocol::canonical_context(&exact)?;
    let (manifest, parts) =
        causal_manifests::fragment_body(protocol::ManifestKind::Context, &body, 0, 1024)?;
    causal_manifests::publish_on(&tx, ns, &manifest, &parts, 1024, |body| {
        protocol::decode_context(body)?;
        Ok(vec![])
    })
    .await?;
    let split = Uuid::new_v4();
    let first = ContextSegment {
        range: KeyRange {
            collection: Collection::Items,
            lower: None,
            upper: Some(split),
        },
        context: ContextRef {
            digest: protocol::context_digest(&exact)?,
            manifest: manifest.clone(),
        },
    };
    let mut other = sync::causal::Context::default();
    other.insert(sync::causal::Dot::new(Uuid::new_v4(), 1)?)?;
    let other = protocol::exact_context(&other);
    let other_body = protocol::canonical_context(&other)?;
    let (other_manifest, other_parts) =
        causal_manifests::fragment_body(protocol::ManifestKind::Context, &other_body, 1, 1024)?;
    causal_manifests::publish_on(&tx, ns, &other_manifest, &other_parts, 1024, |body| {
        protocol::decode_context(body)?;
        Ok(vec![])
    })
    .await?;
    let second = ContextSegment {
        range: KeyRange {
            collection: Collection::Items,
            lower: Some(split),
            upper: None,
        },
        context: ContextRef {
            digest: protocol::context_digest(&other)?,
            manifest: other_manifest,
        },
    };
    assert!(
        causal_compaction::stage_baseline_segment_on::<Model>(
            &tx,
            ns,
            lease,
            1,
            1,
            &second,
            causal_compaction::BaselineStageBudget {
                max_bytes: 1024,
                max_mutations: 10
            }
        )
        .await
        .is_err()
    );
    causal_compaction::stage_baseline_segment_on::<Model>(
        &tx,
        ns,
        lease,
        1,
        0,
        &first,
        causal_compaction::BaselineStageBudget {
            max_bytes: 1024,
            max_mutations: 10,
        },
    )
    .await?;
    let mut equal_neighbor = second.clone();
    equal_neighbor.context = first.context.clone();
    assert!(
        causal_compaction::stage_baseline_segment_on::<Model>(
            &tx,
            ns,
            lease,
            1,
            1,
            &equal_neighbor,
            causal_compaction::BaselineStageBudget {
                max_bytes: 1024,
                max_mutations: 10
            }
        )
        .await
        .is_err()
    );
    causal_compaction::stage_baseline_segment_on::<Model>(
        &tx,
        ns,
        lease,
        1,
        0,
        &first,
        causal_compaction::BaselineStageBudget {
            max_bytes: 1024,
            max_mutations: 10,
        },
    )
    .await?;
    causal_compaction::stage_baseline_segment_on::<Model>(
        &tx,
        ns,
        lease,
        1,
        1,
        &second,
        causal_compaction::BaselineStageBudget {
            max_bytes: 1024,
            max_mutations: 10,
        },
    )
    .await?;
    let gc = sync_orm::causal_gc::reclaim_basis_step_on(&tx, ns, None, 10).await?;
    assert_eq!(gc.removed, 0);
    tx.commit().await?;
    let tx = db.begin().await?;
    causal_leases::revoke_on(&tx, ns, lease).await?;
    let gc = sync_orm::causal_gc::reclaim_basis_step_on(&tx, ns, None, 10).await?;
    assert_eq!(gc.removed, 2);
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn history_pruning_is_bounded_and_preserves_rows_above_floor() -> Result<()> {
    use sea_orm::{
        ConnectionTrait,
        sea_query::{Alias, Expr, Func, Query},
    };
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
    let replica = CausalReplica::<Model>::create_empty(db.clone(), ns, Uuid::new_v4()).await?;
    let identity = replica.identity_on(&db).await?;
    let root = vec![9_u8; 32];
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
            root.clone().into(),
            5_i64.into(),
            vec![8_u8; 32].into(),
            1_i64.into(),
            1_i64.into(),
            1_i64.into(),
            vec![1_u8].into(),
        ])
        .to_owned();
    db.execute(&insert).await?;
    let update = Query::update()
        .table(Alias::new("sync_meta"))
        .value(Alias::new("head"), 3_i64)
        .value(Alias::new("state_seq"), 3_i64)
        .value(Alias::new("history_floor"), 2_i64)
        .value(Alias::new("floor_state_seq"), 2_i64)
        .value(Alias::new("baseline_manifest"), root)
        .value(
            Alias::new("baseline_generation"),
            Uuid::new_v4().as_bytes().to_vec(),
        )
        .to_owned();
    db.execute(&update).await?;
    for sequence in [2_i64, 3] {
        let insert = Query::insert()
            .into_table(Alias::new("sync_journal"))
            .columns(
                [
                    "namespace",
                    "generation",
                    "sequence",
                    "state_seq",
                    "collection",
                    "event_digest",
                    "event_manifest",
                    "anchor_after",
                ]
                .map(Alias::new),
            )
            .values_panic([
                ns.as_bytes().to_vec().into(),
                identity.generation.as_bytes().to_vec().into(),
                sequence.into(),
                sequence.into(),
                b"items".to_vec().into(),
                vec![1_u8; 32].into(),
                vec![2_u8; 32].into(),
                vec![3_u8; 32].into(),
            ])
            .to_owned();
        db.execute(&insert).await?;
        let insert = Query::insert()
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
                b"items".to_vec().into(),
                sequence.into(),
                0_i64.into(),
                vec![0_u8].into(),
                vec![2_u8].into(),
                vec![4_u8; 32].into(),
            ])
            .to_owned();
        db.execute(&insert).await?;
        let insert = Query::insert()
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
                ns.as_bytes().to_vec().into(),
                b"items".to_vec().into(),
                Uuid::new_v4().as_bytes().to_vec().into(),
                1_i64.into(),
                Uuid::new_v4().as_bytes().to_vec().into(),
                Uuid::new_v4().as_bytes().to_vec().into(),
                1_i64.into(),
                vec![1_u8].into(),
                vec![5_u8; 32].into(),
                1_i64.into(),
                Some(1_i64).into(),
                Some(sequence).into(),
            ])
            .to_owned();
        db.execute(&insert).await?;
    }
    let mut removed = 0;
    loop {
        let tx = db.begin().await?;
        let step = causal_compaction::prune_history_step_on(&tx, ns, 1).await?;
        assert!(step.removed <= 1);
        removed += step.removed;
        tx.commit().await?;
        if !step.more {
            break;
        }
    }
    assert_eq!(removed, 3);
    for table in ["sync_journal", "sync_context_ranges", "sync_candidates"] {
        let query = Query::select()
            .expr(Func::count(Expr::val(1)))
            .from(Alias::new(table))
            .to_owned();
        assert_eq!(
            db.query_one(&query)
                .await?
                .unwrap()
                .try_get_by_index::<i64>(0)?,
            1
        );
    }
    Ok(())
}

#[tokio::test]
async fn author_rejects_unrelayable_large_observed_set_before_mutation() -> Result<()> {
    use sea_orm::{
        ConnectionTrait,
        sea_query::{Alias, Expr, Query},
    };
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
    let replica = CausalReplica::<Model>::create_empty(db.clone(), ns, Uuid::new_v4()).await?;
    let key = ObjectKey {
        collection: Collection::Items,
        id: Uuid::new_v4(),
    };
    let tx = db.begin().await?;
    for _ in 0..65 {
        let revision = protocol::Revision {
            actor: Uuid::new_v4(),
            counter: 1,
        };
        let candidate = protocol::Candidate {
            model: protocol::ModelId {
                model: "items".into(),
                schema: "1".into(),
            },
            namespace: ns,
            key: key.clone(),
            dot: protocol::Dot {
                writer_incarnation: Uuid::new_v4(),
                counter: 1,
            },
            revision,
            payload: Value("live".into()),
        };
        let delta = protocol::KeyDelta {
            key: key.clone(),
            author: revision,
            removed: vec![],
            added: vec![candidate],
        };
        replica.apply_key_on(&tx, &delta).await?;
    }
    tx.commit().await?;
    let before = Query::select()
        .columns(["head", "revision_counter"].map(Alias::new))
        .from(Alias::new("sync_meta"))
        .to_owned();
    let expected = db.query_one(&before).await?.unwrap();
    let tx = db.begin().await?;
    let error = replica
        .author_on(&tx, key, Some(Value("next".into())))
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<protocol::SyncError>(),
        Some(protocol::SyncError::WorkLimit)
    ));
    tx.rollback().await?;
    let actual = db.query_one(&before).await?.unwrap();
    assert_eq!(
        actual.try_get_by_index::<i64>(0)?,
        expected.try_get_by_index::<i64>(0)?
    );
    assert_eq!(
        actual.try_get_by_index::<i64>(1)?,
        expected.try_get_by_index::<i64>(1)?
    );
    let count = Query::select()
        .expr(sea_orm::sea_query::Func::count(Expr::val(1)))
        .from(Alias::new("sync_candidates"))
        .to_owned();
    assert_eq!(
        db.query_one(&count)
            .await?
            .unwrap()
            .try_get_by_index::<i64>(0)?,
        65
    );
    let tx = db.begin().await?;
    tx.execute(
        &Query::update()
            .table(Alias::new("sync_meta"))
            .value(Alias::new("revision_counter"), i64::MAX)
            .to_owned(),
    )
    .await?;
    let error = replica
        .author_on(
            &tx,
            ObjectKey {
                collection: Collection::Items,
                id: Uuid::new_v4(),
            },
            Some(Value("exhausted".into())),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<protocol::SyncError>(),
        Some(protocol::SyncError::ClockExhausted)
    ));
    tx.rollback().await?;
    Ok(())
}
