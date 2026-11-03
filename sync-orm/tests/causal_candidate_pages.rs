#[path = "support/fixtures.rs"]
mod fixtures;

use anyhow::Result;
use sea_orm::{
    ConnectionTrait, TransactionTrait,
    sea_query::{Alias, Query},
};
use sync_orm::causal_candidates::page_on;
use uuid::Uuid;

#[tokio::test]
async fn candidate_lifetimes_reconstruct_old_positions_with_keyset_pages() -> Result<()> {
    let db = fixtures::fresh_database().await?;
    let ns = Uuid::from_u128(1);
    let a = Uuid::from_u128(10);
    let b = Uuid::from_u128(20);
    let writer = Uuid::from_u128(30);
    for (object, counter, start, end) in [
        (a, 1_i64, 1_i64, Some(3_i64)),
        (a, 2, 3, None),
        (b, 3, 4, None),
    ] {
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
                b"notes".to_vec().into(),
                writer.as_bytes().to_vec().into(),
                counter.into(),
                object.as_bytes().to_vec().into(),
                writer.as_bytes().to_vec().into(),
                counter.into(),
                vec![1_u8; 16].into(),
                vec![2_u8; 32].into(),
                start.into(),
                Some(start).into(),
                end.into(),
            ])
            .to_owned();
        db.execute(&insert).await?;
    }
    let tx = db.begin().await?;
    let old = page_on(
        &tx,
        sync_orm::causal_candidates::CandidatePageRequest {
            namespace: ns,
            collection: b"notes",
            state_seq: 2,
            lower: None,
            upper: None,
            after: None,
            max_items: 1,
            max_bytes: 1024,
        },
    )
    .await?;
    assert!(old.done);
    assert_eq!(old.rows.len(), 1);
    assert_eq!(old.rows[0].counter, 1);
    let first = page_on(
        &tx,
        sync_orm::causal_candidates::CandidatePageRequest {
            namespace: ns,
            collection: b"notes",
            state_seq: 4,
            lower: None,
            upper: None,
            after: None,
            max_items: 1,
            max_bytes: 1024,
        },
    )
    .await?;
    assert!(!first.done);
    assert_eq!(first.rows[0].counter, 2);
    let last = page_on(
        &tx,
        sync_orm::causal_candidates::CandidatePageRequest {
            namespace: ns,
            collection: b"notes",
            state_seq: 4,
            lower: None,
            upper: None,
            after: first.next,
            max_items: 1,
            max_bytes: 1024,
        },
    )
    .await?;
    assert!(last.done);
    assert_eq!(last.rows[0].counter, 3);
    assert!(
        page_on(
            &tx,
            sync_orm::causal_candidates::CandidatePageRequest {
                namespace: ns,
                collection: b"notes",
                state_seq: 4,
                lower: None,
                upper: None,
                after: None,
                max_items: 1,
                max_bytes: 1
            }
        )
        .await
        .is_err()
    );
    tx.rollback().await?;
    Ok(())
}
