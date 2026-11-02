use anyhow::Result;
use sea_orm::{Database, TransactionTrait};
use sync::causal::{Context, Dot};
use sync_orm::{causal_observed as observed, causal_schema};
use uuid::Uuid;

#[tokio::test]
async fn observed_rows_are_versioned_and_canonical() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
    let writer = Uuid::new_v4();
    let tx = db.begin().await?;
    let mut context = Context::default();
    context.insert(Dot::new(writer, 1)?)?;
    context.insert(Dot::new(writer, 3)?)?;
    let mut budget = 100;
    assert!(observed::insert_on(&tx, ns, b"notes", 1, &context, &mut budget).await?);
    assert!(!observed::contains_on(&tx, ns, b"notes", Dot::new(writer, 2)?).await?);
    let mut middle = Context::default();
    middle.insert(Dot::new(writer, 2)?)?;
    assert!(observed::insert_on(&tx, ns, b"notes", 2, &middle, &mut budget).await?);
    assert_eq!(
        observed::page_at_on(&tx, ns, b"notes", 1, None, 10).await?,
        vec![(writer, 1, 1), (writer, 3, 3)]
    );
    assert_eq!(
        observed::page_at_on(&tx, ns, b"notes", 2, None, 10).await?,
        vec![(writer, 1, 3)]
    );
    assert!(observed::contains_on(&tx, ns, b"notes", Dot::new(writer, 2)?).await?);
    assert!(!observed::insert_on(&tx, ns, b"notes", 2, &context, &mut budget).await?);
    assert_eq!(
        observed::unobserved_on(&tx, ns, b"notes", writer, 1, 5).await?,
        vec![(4, 5)]
    );
    assert_eq!(observed::reclaim_step_on(&tx, ns, 2, 10).await?, 2);
    assert_eq!(
        observed::page_at_on(&tx, ns, b"notes", 2, None, 10).await?,
        vec![(writer, 1, 3)]
    );
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
async fn observed_terminal_interval_and_same_state_union() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    causal_schema::install_fresh(&db).await?;
    let ns = Uuid::new_v4();
    let writer = Uuid::new_v4();
    let tx = db.begin().await?;
    let mut budget = 100;
    for counter in [1, 3, 2, sync::causal::MAX_DOT_COUNTER] {
        let mut context = Context::default();
        context.insert(Dot::new(writer, counter)?)?;
        assert!(observed::insert_on(&tx, ns, b"notes", 1, &context, &mut budget).await?);
    }
    assert_eq!(
        observed::page_at_on(&tx, ns, b"notes", 1, None, 10).await?,
        vec![
            (writer, 1, 3),
            (
                writer,
                sync::causal::MAX_DOT_COUNTER,
                sync::causal::MAX_DOT_COUNTER
            )
        ]
    );
    assert!(
        observed::contains_on(
            &tx,
            ns,
            b"notes",
            Dot::new(writer, sync::causal::MAX_DOT_COUNTER)?
        )
        .await?
    );
    assert_eq!(observed::reclaim_step_on(&tx, ns, 1, 10).await?, 0);
    tx.commit().await?;
    Ok(())
}
