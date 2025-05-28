use std::{collections::HashMap, fmt::Debug};

use anyhow::Result;
use async_trait::async_trait;
use sea_orm::{ActiveModelBehavior, ConnectionTrait, DatabaseConnection, DatabaseTransaction};
use serde::Serialize;

use crate::{chunking::ChunkFkMapping, hlc::HLCRecord};

// Maps an FK column name (in the current entity) to the sync_id of the referenced record.
// Option<String> because the FK might be nullable.
pub type FkPayload = HashMap<String, Option<String>>;

// Define a marker trait for types that can execute database queries.
// This helps constrain the `db` parameter in ForeignKeyResolver methods.
pub trait DatabaseExecutor: ConnectionTrait + Send + Sync {} // Ensure ConnectionTrait is a supertrait

// Implement the marker trait for specific SeaORM connection types.
impl DatabaseExecutor for DatabaseConnection {}
impl DatabaseExecutor for DatabaseTransaction {}

#[async_trait]
pub trait ModelWithForeignKeyOps: HLCRecord + Sync + Send + Serialize + Sized {
    /// Extracts the `sync_id`s of all records this model instance references via foreign keys.
    async fn extract_model_fk_sync_ids<E: DatabaseExecutor>(&self, db: &E) -> Result<FkPayload>;

    /// Generates FK mappings for a batch of these models.
    async fn generate_model_fk_mappings_for_batch<E: DatabaseExecutor>(
        records: &[Self],
        db: &E,
    ) -> Result<ChunkFkMapping>;

    /// Extracts FkPayload from this remote model using provided chunk mappings.
    fn extract_model_sync_ids_from_remote(
        &self,
        chunk_fk_map: &ChunkFkMapping,
    ) -> Result<FkPayload>;
}

// New trait for ActiveModelBehavior types
#[async_trait]
pub trait ActiveModelWithForeignKeyOps: ActiveModelBehavior + Send + Sized {
    /// Resolves `sync_id`s to local PKs and sets them on this active model.
    async fn remap_model_and_set_foreign_keys<E: DatabaseExecutor>(
        &mut self,
        fk_sync_id_payload: &FkPayload,
        db: &E,
    ) -> Result<()>;
}

#[async_trait]
pub trait ForeignKeyResolver: Send + Sync + Debug {
    /// For a given model instance of a specific entity type, extract the `sync_id`s
    /// of all records it references via foreign keys.
    ///
    /// # Arguments
    /// * `model`: The model instance.
    /// * `db`: Database connection for lookups.
    ///
    /// # Returns
    /// A map where keys are FK column names in `model`'s entity,
    /// and values are `Option<sync_id>` of the referenced records.
    async fn extract_foreign_key_sync_ids<M: HLCRecord + Sync + Serialize, E>(
        &self,
        model: &M,
        db: &E,
    ) -> Result<FkPayload>
    where
        M: ModelWithForeignKeyOps,
        E: DatabaseExecutor;

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
    /// * `active_model`: The active model to be modified.
    /// * `fk_sync_id_payload`: The map of FK column names to referenced `sync_id`s.
    /// * `db`: Database connection for lookups.
    async fn remap_and_set_foreign_keys<AM: ActiveModelBehavior + Send, E>(
        &self,
        active_model: &mut AM,
        fk_sync_id_payload: &FkPayload,
        db: &E,
    ) -> Result<()>
    where
        AM: ActiveModelWithForeignKeyOps,
        E: DatabaseExecutor;

    /// Extracts FkPayload from a remote model (usually obtained via get_remote_records_in_hlc_range).
    /// Now it requires access to the ChunkFkMapping associated with these models.
    ///
    /// # Arguments
    /// * `remote_model_with_sync_id_fks`: An instance of the remote model.
    /// * `chunk_fk_map`: Option<&ChunkFkMapping> containing FK mappings for the current context.
    ///                  Should be provided if the model comes from a DataChunk carrying mappings.
    ///                  May be None for individually fetched models, requiring fallback or specific logic.
    fn extract_sync_ids_from_remote_model_with_mapping<M>(
        &self,
        remote_model_with_sync_id_fks: &M,
        chunk_fk_map: Option<&ChunkFkMapping>,
    ) -> Result<FkPayload>
    where
        M: ModelWithForeignKeyOps;

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
        records: &[M],
        db: &E,
    ) -> Result<ChunkFkMapping>
    where
        M: ModelWithForeignKeyOps,
        E: DatabaseExecutor;
}
