use std::{collections::HashMap, fmt::Debug};

use anyhow::Result;
use async_trait::async_trait;
use sea_orm::{ActiveModelBehavior, DatabaseConnection};

use crate::hlc::HLCRecord;

// Maps an FK column name (in the current entity) to the sync_id of the referenced record.
// Option<String> because the FK might be nullable.
pub type FkPayload = HashMap<String, Option<String>>;

#[async_trait]
pub trait ForeignKeyResolver: Send + Sync + Debug {
    /// For a given model instance of a specific entity type, extract the `sync_id`s
    /// of all records it references via foreign keys.
    ///
    /// # Arguments
    /// * `entity_name`: Name of the entity `model` belongs to.
    /// * `model`: The model instance.
    /// * `db`: Database connection for lookups.
    ///
    /// # Returns
    /// A map where keys are FK column names in `model`'s entity,
    /// and values are `Option<sync_id>` of the referenced records.
    async fn extract_foreign_key_sync_ids<M: HLCRecord + Sync>(
        &self,
        entity_name: &str,
        model: &M,
        db: &DatabaseConnection,
    ) -> Result<FkPayload>;

    /// Given an `ActiveModel` and a payload of foreign key `sync_id`s,
    /// this method resolves those `sync_id`s to local primary key `Value`s
    /// and sets them on the `ActiveModel`.
    ///
    /// This method should ideally batch lookups if called with multiple `active_models`
    /// or be prepared to be called within a loop. For simplicity here, we'll assume
    /// it's called per `ActiveModel`, but an implementation might cache/batch.
    /// A more advanced version could take a slice of (ActiveModel, FkPayload).
    ///
    /// # Arguments
    /// * `entity_name`: Name of the entity `active_model` belongs to.
    /// * `active_model`: The active model to be modified.
    /// * `fk_sync_id_payload`: The map of FK column names to referenced `sync_id`s.
    /// * `db`: Database connection for lookups.
    async fn remap_and_set_foreign_keys<AM: ActiveModelBehavior + Send>(
        &self,
        entity_name: &str,
        active_model: &mut AM,
        fk_sync_id_payload: &FkPayload,
        db: &DatabaseConnection,
    ) -> Result<()>;
}
