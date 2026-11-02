//! Explicit, paged diagnostics over the registered source UUID references.
use super::{
    SeaOrmSyncStore,
    accepted::AcceptedState,
    model::{Collection, ObjectKey, RuneModel},
};
use anyhow::{Result, ensure};
use sea_orm::{DatabaseConnection, TransactionTrait};
use uuid::Uuid;

#[derive(Debug)]
pub struct MissingReference {
    pub owner: ObjectKey,
    pub target: ObjectKey,
}

#[derive(Debug)]
pub struct ReferencePage {
    pub missing: Vec<MissingReference>,
    pub next: Option<Uuid>,
    pub done: bool,
}

/// One short read transaction; each call observes the current source rows.
pub async fn page(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    collection: Collection,
    after: Option<Uuid>,
    max_records: u32,
) -> Result<ReferencePage> {
    ensure!(
        (1..=32).contains(&max_records),
        "reference page requires 1..=32 records"
    );
    let tx = db.begin().await?;
    let records = AcceptedState::new(&tx, store)
        .page(
            collection,
            after,
            sync::PageLimits {
                max_scanned: max_records,
                max_bytes: 4 * 1024 * 1024,
            },
        )
        .await?;
    let mut missing = Vec::new();
    for record in records.records {
        if let Some(value) = record.value {
            for target in RuneModel::missing_references_on(&tx, &value, 256).await? {
                missing.push(MissingReference {
                    owner: record.key.clone(),
                    target,
                });
            }
        }
    }
    ensure!(
        records.done || records.next != after,
        "reference page made no progress"
    );
    tx.commit().await?;
    Ok(ReferencePage {
        missing,
        next: records.next,
        done: records.done,
    })
}

/// Collect an explicitly requested diagnostic, releasing the transaction between pages.
pub(crate) async fn collect(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    collections: impl IntoIterator<Item = Collection>,
) -> Result<Vec<(ObjectKey, String)>> {
    let mut missing = Vec::new();
    for collection in collections {
        let mut after = None;
        loop {
            let page = page(db, store, collection, after, 32).await?;
            missing.extend(page.missing.into_iter().map(|reference| {
                (
                    reference.owner,
                    format!("missing source row: {:?}", reference.target),
                )
            }));
            if page.done {
                break;
            }
            after = page.next;
        }
    }
    Ok(missing)
}
