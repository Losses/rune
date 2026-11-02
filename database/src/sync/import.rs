//! Bounded traversal of the registered source rows.
use super::{
    SeaOrmSyncStore,
    model::{Collection, ObjectKey, RuneModel},
};
use anyhow::{Context, Result};
use sea_orm::DatabaseTransaction;
use sync::SyncModel;
use sync_orm::registry::PolicyRegistry;
use sync_orm::row_policy::{ScanLimits, ScanPage};

pub async fn batch(
    tx: &DatabaseTransaction,
    store: &SeaOrmSyncStore,
    collection: &Collection,
    after: Option<i64>,
) -> Result<ScanPage<super::model::Payload>> {
    store.lock(tx).await?;
    let page = RuneModel::scan_on(
        tx,
        collection,
        after,
        ScanLimits {
            max_records: 128,
            max_rows: 1024,
            max_bytes: 4 * 1024 * 1024,
            max_mutations: 128,
        },
    )
    .await
    .with_context(|| {
        format!(
            "scan {} after {:?}",
            RuneModel::collection_tag(collection),
            after
        )
    })?;
    for row in &page.records {
        if RuneModel::authoring_owner(&row.payload).is_some_and(|owner| owner != store.author()) {
            continue;
        }
        let key = ObjectKey {
            collection: *collection,
            id: row.id,
        };
        if store.get_on(tx, &key).await?.is_none() {
            store.author_on(tx, key, Some(row.payload.clone())).await?;
        }
    }
    Ok(page)
}
