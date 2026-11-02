//! Transactional source projection from Rune's registered row policies.
use super::model::{ObjectKey, Payload, RuneModel};
use anyhow::Result;
use sea_orm::DatabaseTransaction;

pub async fn apply(
    tx: &DatabaseTransaction,
    key: &ObjectKey,
    value: Option<&Payload>,
) -> Result<()> {
    RuneModel::project_on(tx, key, value).await?;
    Ok(())
}
