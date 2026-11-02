use anyhow::Result;
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use sync::causal::{Candidate, CausalMap, Dot, Limits};
use uuid::Uuid;

#[test]
fn every_delivery_order_and_duplicate_converges() -> Result<()> {
    let mut state = CausalMap::<u32, String, u64>::new(Limits::default())?;
    state.assign(
        1,
        Candidate {
            dot: Dot::new(Uuid::from_u128(1), 1)?,
            revision: 1,
            value: "old".into(),
        },
    )?;
    let first = state.clone();
    state.assign(
        1,
        Candidate {
            dot: Dot::new(Uuid::from_u128(2), 1)?,
            revision: 2,
            value: "edited".into(),
        },
    )?;
    let second = state.clone();
    state.remove(&1);
    let removed = state.clone();
    state.assign(
        1,
        Candidate {
            dot: Dot::new(Uuid::from_u128(3), 1)?,
            revision: 3,
            value: "restored".into(),
        },
    )?;
    let versions = [first, second, removed, state.clone()];
    let mut checked = 0;
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for d in 0..4 {
                    let order = [a, b, c, d];
                    if order
                        .into_iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != 4
                    {
                        continue;
                    }
                    let mut merged = CausalMap::<u32, String, u64>::new(Limits::default())?;
                    for index in order.into_iter().chain(order) {
                        merged.merge(&versions[index])?;
                    }
                    assert!(merged.semantic_eq(&state)?);
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 24);
    Ok(())
}

#[tokio::test]
async fn physical_parent_replacement_cascades_existing_relationships() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    db.execute_unprepared("PRAGMA foreign_keys=ON").await?;
    db.execute_unprepared("CREATE TABLE parent(id INTEGER PRIMARY KEY, name TEXT)")
        .await?;
    db.execute_unprepared(
        "CREATE TABLE membership(parent_id INTEGER REFERENCES parent(id) ON DELETE CASCADE)",
    )
    .await?;
    db.execute_unprepared("INSERT INTO parent VALUES(1,'before')")
        .await?;
    db.execute_unprepared("INSERT INTO membership VALUES(1)")
        .await?;
    db.execute_unprepared("DELETE FROM parent WHERE id=1")
        .await?;
    db.execute_unprepared("INSERT INTO parent VALUES(1,'after')")
        .await?;
    let row = db
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT COUNT(*) AS n FROM membership",
        ))
        .await?
        .unwrap();
    assert_eq!(row.try_get::<i64>("", "n")?, 0);
    Ok(())
}
