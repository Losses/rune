//! Transactional source projection from Rune's registered row policies.
use super::model::{Collection, ObjectKey, Payload, RuneModel};
use anyhow::Result;
use sea_orm::DatabaseTransaction;

use crate::actions::resolved_metadata;

pub async fn apply(
    tx: &DatabaseTransaction,
    key: &ObjectKey,
    value: Option<&Payload>,
) -> Result<()> {
    RuneModel::project_on(tx, key, value).await?;
    if key.collection == Collection::Tracks && value.is_none() {
        resolved_metadata::reconcile_recordings(tx).await?;
    }
    Ok(())
}
