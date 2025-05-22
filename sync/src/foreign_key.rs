use std::{collections::HashMap, fmt::Debug};

use anyhow::Result;
use async_trait::async_trait;
use sea_orm::{ActiveModelBehavior, DatabaseConnection, DatabaseTransaction};
use serde::Serialize;

use crate::hlc::HLCRecord;

// Maps an FK column name (in the current entity) to the sync_id of the referenced record.
// Option<String> because the FK might be nullable.
pub type FkPayload = HashMap<String, Option<String>>;

pub trait DatabaseExecutor: Send + Sync {}

impl DatabaseExecutor for DatabaseConnection {}
impl DatabaseExecutor for DatabaseTransaction {}

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
    async fn extract_foreign_key_sync_ids<M: HLCRecord + Sync + Serialize, E>(
        &self,
        entity_name: &str,
        model: &M,
        db: &E,
    ) -> Result<FkPayload>
    where
        E: DatabaseExecutor + sea_orm::ConnectionTrait;

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
    async fn remap_and_set_foreign_keys<AM: ActiveModelBehavior + Send, E>(
        &self,
        entity_name: &str,
        active_model: &mut AM,
        fk_sync_id_payload: &FkPayload,
        db: &E,
    ) -> Result<()>
    where
        E: DatabaseExecutor + sea_orm::ConnectionTrait;

    /// Constructs an FkPayload from a model instance that is assumed to have come
    /// from a remote source, where its foreign key fields already represent sync_ids.
    ///
    /// # Arguments
    /// * `entity_name`: Name of the entity `model` belongs to.
    /// * `remote_model_with_sync_id_fks`: The model instance. Its FK fields should be sync_ids.
    ///
    /// # Returns
    /// An FkPayload.
    ///
    /// This method typically involves knowing the structure of M and which fields
    /// correspond to logical foreign keys. The implementation will likely involve
    /// matching on `entity_name` and casting `remote_model_with_sync_id_fks`.
    fn extract_sync_ids_from_remote_model<M: HLCRecord + Send + Sync + Serialize>(
        &self,
        entity_name: &str,
        remote_model_with_sync_id_fks: &M,
    ) -> Result<FkPayload>;
}
