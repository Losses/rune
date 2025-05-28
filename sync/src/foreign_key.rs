use std::{collections::HashMap, fmt::Debug};

use anyhow::Result;
use async_trait::async_trait;
use sea_orm::{ActiveModelBehavior, DatabaseConnection, DatabaseTransaction};
use serde::Serialize;

use crate::{chunking::ChunkFkMapping, hlc::HLCRecord};

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

    /// Extracts FkPayload from a remote model (usually obtained via get_remote_records_in_hlc_range).
    /// Now it requires access to the ChunkFkMapping associated with these models.
    ///
    /// # Arguments
    /// * `entity_name`: The name of the entity to which the model belongs.
    /// * `remote_model_with_sync_id_fks`: An instance of the remote model.
    /// * `chunk_fk_map`: Option<&ChunkFkMapping> containing FK mappings for the current context.
    ///                  Should be provided if the model comes from a DataChunk carrying mappings.
    ///                  May be None for individually fetched models, requiring fallback or specific logic.
    fn extract_sync_ids_from_remote_model_with_mapping<M: HLCRecord + Send + Sync + Serialize>(
        &self,
        entity_name: &str,
        remote_model_with_sync_id_fks: &M,
        chunk_fk_map: Option<&ChunkFkMapping>,
    ) -> Result<FkPayload>;

    /// Generate a mapping from foreign keys to sync_ids for a batch of models belonging to a specific entity type.
    /// This method is primarily used on the data provider side (e.g., when the server generates a DataChunk).
    ///
    /// # Arguments
    /// * `entity_name`: The name of the entity to which the models belong.
    /// * `models`: A batch of model instances that need to be processed.
    /// * `db`: The database connection used to look up the sync_id of the referenced parent entities.
    ///
    /// # Returns
    /// A `ChunkFkMapping` that contains the mappings for all foreign key references in this batch of models.
    /// Key: The foreign key column name in the child table (e.g., "album_id")
    /// Value: HashMap { Parent table local numeric ID string -> Parent table sync_id (UUID string) }
    async fn generate_fk_mappings_for_records<M, E>(
        &self,
        entity_name: &str,
        records: &[M],
        db: &E,
    ) -> Result<ChunkFkMapping>
    where
        M: HLCRecord + Sync + Serialize,
        E: DatabaseExecutor + sea_orm::ConnectionTrait;
}
