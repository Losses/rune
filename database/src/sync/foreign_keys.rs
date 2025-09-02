use std::collections::{HashMap, hash_map};
use std::fmt::Debug;
use std::marker::PhantomData;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use log::{debug, warn};
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, Iden, Iterable,
    PrimaryKeyToColumn, PrimaryKeyTrait, QueryFilter, QuerySelect, TryGetable, Value,
};

use ::sync::{
    chunking::ChunkFkMapping,
    foreign_key::{
        ActiveModelWithForeignKeyOps, DatabaseExecutor, FkPayload, ForeignKeyResolver,
        ModelWithForeignKeyOps,
    },
    hlc::{HLCModel, HLCRecord},
};

use crate::entities::{
    albums, artists, genres, media_cover_art, media_file_albums, media_file_artists,
    media_file_fingerprint, media_file_genres, media_file_similarity, media_files,
};

/// Foreign key resolver implementation for the Rune.
///
/// This resolver handles the mapping and resolution of foreign key relationships
/// between entities during synchronization operations. It provides functionality to:
/// - Extract sync IDs from local models
/// - Map foreign keys between local and remote representations
/// - Generate foreign key mappings for batches of records
/// - Resolve sync IDs back to local primary keys
#[derive(Debug, Clone)]
pub struct RuneForeignKeyResolver;

/// Retrieves the sync ID (HLC UUID) for a referenced entity by its primary key.
///
/// This is a generic helper function that looks up the unique sync identifier
/// for any entity that implements HLCModel, given its local primary key value.
///
/// # Arguments
/// * `db` - Database connection/executor
/// * `model_fk_value` - Optional foreign key value pointing to the referenced entity
/// * `referenced_entity_pk_col` - Primary key column of the referenced entity
///
/// # Returns
/// * `Ok(Some(String))` - The sync ID if the referenced entity exists
/// * `Ok(None)` - If the foreign key value is None or entity not found
/// * `Err` - Database error during lookup
async fn get_referenced_sync_id<ReferencedEntity, Db>(
    db: &Db,
    model_fk_value: Option<<ReferencedEntity::PrimaryKey as PrimaryKeyTrait>::ValueType>,
    referenced_entity_pk_col: ReferencedEntity::Column,
) -> Result<Option<String>>
where
    ReferencedEntity: EntityTrait + HLCModel,
    <ReferencedEntity::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Into<Value> + Eq + Copy + Send + Sync + Debug,
    Db: ConnectionTrait + DatabaseExecutor,
{
    if let Some(fk_val) = model_fk_value {
        /// Helper struct to select only the HLC UUID from query results
        #[derive(Debug, FromQueryResult)]
        struct HlcUuidOnly {
            hlc_uuid: String,
        }

        let result = ReferencedEntity::find()
            .select_only()
            .column_as(ReferencedEntity::unique_id_column(), "hlc_uuid")
            .filter(referenced_entity_pk_col.eq(fk_val))
            .into_model::<HlcUuidOnly>()
            .one(db)
            .await
            .with_context(|| {
                format!(
                    "DB error looking up referenced entity {:?} by PK {:?}",
                    std::any::type_name::<ReferencedEntity>(),
                    fk_val
                )
            })?;

        Ok(result.map(|r| r.hlc_uuid))
    } else {
        Ok(None)
    }
}

/// Retrieves the local primary key for an entity by its sync ID.
///
/// This function performs the reverse lookup - given a sync ID (HLC UUID),
/// it finds the corresponding local primary key value for the entity.
///
/// # Arguments
/// * `db` - Database connection/executor
/// * `sync_id` - Optional sync ID to look up
///
/// # Returns
/// * `Ok(Some(PK))` - The local primary key if entity exists
/// * `Ok(None)` - If sync_id is None or entity not found locally
/// * `Err` - Database error or entity configuration issues
async fn get_local_pk_from_sync_id<ReferencedEntity, Db>(
    db: &Db,
    sync_id: Option<&String>,
) -> Result<Option<<ReferencedEntity::PrimaryKey as PrimaryKeyTrait>::ValueType>>
where
    ReferencedEntity: EntityTrait + HLCModel,
    <ReferencedEntity::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Clone + TryGetable + Send + Sync + Debug + 'static,
    Db: ConnectionTrait + DatabaseExecutor,
{
    if let Some(sid_str) = sync_id {
        let pk_def = ReferencedEntity::PrimaryKey::iter().next().ok_or_else(|| {
            anyhow!(
                "Entity {} has no primary key defined",
                ReferencedEntity::table_name(&ReferencedEntity::default())
            )
        })?;
        let pk_column_in_query = pk_def.into_column();

        /// Helper struct to extract only the primary key value from query results
        #[derive(Debug)]
        struct PkOnlyModelHelper<RE: EntityTrait>
        where
            <RE::PrimaryKey as PrimaryKeyTrait>::ValueType:
                Clone + TryGetable + Send + Sync + Debug + 'static,
        {
            pk_value: <RE::PrimaryKey as PrimaryKeyTrait>::ValueType,
            _phantom: PhantomData<RE>,
        }

        impl<RE: EntityTrait> FromQueryResult for PkOnlyModelHelper<RE>
        where
            <RE::PrimaryKey as PrimaryKeyTrait>::ValueType:
                Clone + TryGetable + Send + Sync + Debug + 'static,
        {
            fn from_query_result(
                res: &sea_orm::QueryResult,
                pre: &str,
            ) -> std::result::Result<Self, sea_orm::DbErr> {
                let pk_item_def = RE::PrimaryKey::iter().next().ok_or_else(|| {
                    sea_orm::DbErr::Custom(format!(
                        "Entity {} has no primary key defined (in PkOnlyModelHelper for table {})",
                        std::any::type_name::<RE>(),
                        RE::table_name(&RE::default())
                    ))
                })?;
                let pk_column_name_in_result = pk_item_def.into_column().to_string();

                let val = res.try_get(pre, &pk_column_name_in_result)?;
                Ok(PkOnlyModelHelper {
                    pk_value: val,
                    _phantom: PhantomData,
                })
            }
        }

        let result = ReferencedEntity::find()
            .select_only()
            .column(pk_column_in_query)
            .filter(ReferencedEntity::unique_id_column().eq(sid_str.clone()))
            .into_model::<PkOnlyModelHelper<ReferencedEntity>>()
            .one(db)
            .await
            .with_context(|| {
                format!(
                    "DB error looking up PK for sync_id {} in table {}",
                    sid_str,
                    ReferencedEntity::table_name(&ReferencedEntity::default())
                )
            })?;

        if result.is_none() {
            debug!(
                "Could not find local PK for sync_id: {} in table {}",
                sid_str,
                ReferencedEntity::table_name(&ReferencedEntity::default())
            );
        }
        Ok(result.map(|r| r.pk_value))
    } else {
        Ok(None)
    }
}

/// Generates foreign key mappings for a specific column across a batch of records.
///
/// This helper function creates a mapping from local primary key values to sync IDs
/// for a parent entity referenced by multiple child records. It's used to avoid
/// redundant database queries when processing batches.
///
/// # Arguments
/// * `records_slice` - Slice of records to process
/// * `get_parent_local_id_fn` - Function to extract parent's local ID from each record
/// * `parent_entity_pk_col` - Primary key column of the parent entity
/// * `db_conn` - Database connection/executor
///
/// # Returns
/// * `Ok(Some(HashMap))` - Mapping from local ID strings to sync ID strings
/// * `Ok(None)` - If no mappings were generated (empty result)
/// * `Err` - Database errors during lookup
async fn generate_fk_mapping_for_column<ParentEntity, Rec, GetParentIdFn, DBE>(
    records_slice: &[Rec],
    get_parent_local_id_fn: GetParentIdFn,
    parent_entity_pk_col: ParentEntity::Column,
    db_conn: &DBE,
) -> Result<Option<HashMap<String, String>>>
where
    ParentEntity: EntityTrait + HLCModel,
    <ParentEntity::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Into<Value> + Eq + Copy + Send + Sync + Debug + ToString,
    Rec: HLCRecord + Sync,
    GetParentIdFn: Fn(&Rec) -> <ParentEntity::PrimaryKey as PrimaryKeyTrait>::ValueType,
    DBE: DatabaseExecutor,
{
    let mut col_specific_map = HashMap::new();
    for r_model in records_slice {
        let parent_local_id = get_parent_local_id_fn(r_model);
        if let hash_map::Entry::Vacant(e) = col_specific_map.entry(parent_local_id.to_string()) {
            if let Some(sync_id_str) = get_referenced_sync_id::<ParentEntity, _>(
                db_conn,
                Some(parent_local_id),
                parent_entity_pk_col,
            )
            .await?
            {
                e.insert(sync_id_str);
            } else {
                warn!(
                    "Could not find sync_id for parent {} referenced by local_id {} (child record {}). Parent might not exist or missing HLC UUID.",
                    std::any::type_name::<ParentEntity>(),
                    parent_local_id.to_string(),
                    r_model.unique_id()
                );
            }
        }
    }
    if col_specific_map.is_empty() {
        Ok(None)
    } else {
        Ok(Some(col_specific_map))
    }
}

/// Extracts sync ID from chunk foreign key mapping with error handling.
///
/// This helper function safely retrieves a sync ID from the chunk FK mapping
/// structure, providing appropriate warnings when mappings are missing.
///
/// # Arguments
/// * `chunk_fk_map` - The chunk-level foreign key mapping
/// * `table_name_for_log` - Table name for logging purposes
/// * `fk_col_name` - Foreign key column name
/// * `parent_id_str` - Parent entity ID as string
/// * `model_unique_id` - Unique ID of the model being processed
///
/// # Returns
/// * `Some(String)` - The sync ID if found in mapping
/// * `None` - If not found, with appropriate warning logged
fn extract_sync_id_from_chunk_map(
    chunk_fk_map: &ChunkFkMapping,
    table_name_for_log: &str,
    fk_col_name: &str,
    parent_id_str: &str,
    model_unique_id: &str,
) -> Option<String> {
    let sync_id = chunk_fk_map
        .get(fk_col_name)
        .and_then(|col_map| col_map.get(parent_id_str))
        .cloned();

    // Log warning if the parent ID exists in the column map but has no sync ID
    if sync_id.is_none()
        && chunk_fk_map
            .get(fk_col_name)
            .is_some_and(|m| m.get(parent_id_str).is_none())
    {
        warn!(
            "Sync ID not found in map for {table_name_for_log}.{fk_col_name} = {parent_id_str} (remote model {model_unique_id}). Child of non-existent parent?"
        );
    }
    sync_id
}

/// Sets a foreign key value on an ActiveModel from a sync ID.
///
/// This helper function resolves a sync ID to a local primary key and handles
/// the cases where the foreign key is nullable or mandatory.
///
/// # Arguments
/// * `sync_id_opt` - Optional sync ID to resolve
/// * `db` - Database executor
/// * `fk_col_name` - Foreign key column name (for error messages)
/// * `is_nullable` - Whether the foreign key column allows NULL values
///
/// # Returns
/// * `Ok(Some(PK))` - Resolved local primary key
/// * `Ok(None)` - When sync_id is None and FK is nullable
/// * `Err` - When resolution fails or constraints are violated
async fn set_foreign_key_from_sync_id<ParentEntity, E>(
    sync_id_opt: Option<&String>,
    db: &E,
    fk_col_name: &str,
    is_nullable: bool,
) -> Result<Option<<ParentEntity::PrimaryKey as PrimaryKeyTrait>::ValueType>>
where
    ParentEntity: EntityTrait + HLCModel,
    <ParentEntity::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Clone + TryGetable + Send + Sync + Debug + 'static,
    E: DatabaseExecutor,
{
    match sync_id_opt {
        Some(sync_id) => {
            let local_pk = get_local_pk_from_sync_id::<ParentEntity, _>(db, Some(sync_id)).await?;

            if local_pk.is_none() && !is_nullable {
                return Err(anyhow!(
                    "Failed to find local PK for {} using sync_id: {}. Referenced entity may not exist locally.",
                    fk_col_name,
                    sync_id
                ));
            }
            Ok(local_pk)
        }
        None => {
            if !is_nullable {
                return Err(anyhow!(
                    "FkPayload missing mandatory sync_id for non-nullable FK {}",
                    fk_col_name
                ));
            }
            Ok(None)
        }
    }
}

#[async_trait]
impl ForeignKeyResolver for RuneForeignKeyResolver {
    async fn extract_foreign_key_sync_ids<M, E>(&self, model: &M, db: &E) -> Result<FkPayload>
    where
        M: ModelWithForeignKeyOps,
        E: DatabaseExecutor,
    {
        model.extract_model_fk_sync_ids(db).await
    }

    async fn remap_and_set_foreign_keys<AM, E>(
        &self,
        active_model: &mut AM,
        fk_sync_id_payload: &FkPayload,
        db: &E,
    ) -> Result<()>
    where
        AM: ActiveModelWithForeignKeyOps,
        E: DatabaseExecutor,
    {
        active_model
            .remap_model_and_set_foreign_keys(fk_sync_id_payload, db)
            .await
    }

    fn extract_sync_ids_from_remote_model_with_mapping<M>(
        &self,
        remote_model: &M,
        chunk_fk_map: Option<&ChunkFkMapping>,
    ) -> Result<FkPayload>
    where
        M: ModelWithForeignKeyOps,
    {
        let Some(fk_map) = chunk_fk_map else {
            bail!(
                "Missing ChunkFkMapping for remote model '{}' (unique_id: {}). RuneForeignKeyResolver expects it.",
                std::any::type_name::<M>(), // This is fine as M is a generic type param here
                remote_model.unique_id()
            );
        };
        remote_model.extract_model_sync_ids_from_remote(fk_map)
    }

    async fn generate_fk_mappings_for_records<M, E>(
        &self,
        records: &[M],
        db: &E,
    ) -> Result<ChunkFkMapping>
    where
        M: ModelWithForeignKeyOps,
        E: DatabaseExecutor,
    {
        M::generate_model_fk_mappings_for_batch(records, db).await
    }
}

/// Macro to generate foreign key operations for simple entities (no foreign keys).
///
/// This macro generates boilerplate implementations for entities that don't have
/// foreign key relationships. All methods return empty payloads/mappings.
///
/// # Arguments
/// * `$model` - The model type (e.g., `albums::Model`)
/// * `$active_model` - The active model type (e.g., `albums::ActiveModel`)
macro_rules! impl_simple_entity_fk_ops {
    ($model:ty, $active_model:ty) => {
        #[async_trait]
        impl ModelWithForeignKeyOps for $model {
            async fn extract_model_fk_sync_ids<E: DatabaseExecutor>(
                &self,
                _db: &E,
            ) -> Result<FkPayload> {
                Ok(FkPayload::new())
            }
            async fn generate_model_fk_mappings_for_batch<DbEx: DatabaseExecutor>(
                _records: &[Self],
                _db: &DbEx,
            ) -> Result<ChunkFkMapping> {
                Ok(ChunkFkMapping::new())
            }
            fn extract_model_sync_ids_from_remote(
                &self,
                _chunk_fk_map: &ChunkFkMapping,
            ) -> Result<FkPayload> {
                Ok(FkPayload::new())
            }
        }

        #[async_trait]
        impl ActiveModelWithForeignKeyOps for $active_model {
            async fn remap_model_and_set_foreign_keys<E: DatabaseExecutor>(
                &mut self,
                _fk_sync_id_payload: &FkPayload,
                _db: &E,
            ) -> Result<()> {
                Ok(())
            }
        }
    };
}

// Generate implementations for simple entities (no foreign keys)
impl_simple_entity_fk_ops!(albums::Model, albums::ActiveModel);
impl_simple_entity_fk_ops!(artists::Model, artists::ActiveModel);
impl_simple_entity_fk_ops!(genres::Model, genres::ActiveModel);
impl_simple_entity_fk_ops!(media_cover_art::Model, media_cover_art::ActiveModel);

/// Macro to generate foreign key operations for junction tables with two foreign keys.
///
/// This macro generates comprehensive FK handling for many-to-many relationship tables
/// that typically have exactly two foreign key columns pointing to the related entities.
///
/// # Arguments
/// * `$model` - The junction table model type
/// * `$active_model` - The junction table active model type  
/// * `$table_name` - String literal of the table name (for logging)
/// * Array of two FK definitions, each containing:
///   - `$fk_field` - Field name in the model struct
///   - `$fk_col` - Column enum variant
///   - `$parent_entity` - Referenced entity type
///   - `$parent_col` - Referenced entity's PK column
macro_rules! impl_junction_table_fk_ops {
    (
        $model:ty,
        $active_model:ty,
        $table_name:expr,
        [
            $(($fk_field:ident, $fk_col:expr, $parent_entity:ty, $parent_col:expr)),*
        ]
    ) => {
        #[async_trait]
        impl ModelWithForeignKeyOps for $model {
            async fn extract_model_fk_sync_ids<E: DatabaseExecutor>(&self, db: &E) -> Result<FkPayload> {
                let mut payload = FkPayload::new();
                $(
                    let fk_col_name = $fk_col.to_string();
                    let fk_sync_id = get_referenced_sync_id::<$parent_entity, _>(
                        db,
                        Some(self.$fk_field),
                        $parent_col,
                    )
                    .await?;
                    payload.insert(fk_col_name, fk_sync_id);
                )*
                Ok(payload)
            }

            async fn generate_model_fk_mappings_for_batch<DbEx: DatabaseExecutor>(
                records: &[Self],
                db: &DbEx,
            ) -> Result<ChunkFkMapping> {
                let mut overall_mapping = ChunkFkMapping::new();
                $(
                    let fk_col_str = $fk_col.to_string();
                    if let Some(map) = generate_fk_mapping_for_column::<$parent_entity, _, _, _>(
                        records,
                        |r| r.$fk_field,
                        $parent_col,
                        db,
                    )
                    .await?
                    {
                        overall_mapping.insert(fk_col_str, map);
                    }
                )*
                Ok(overall_mapping)
            }

            fn extract_model_sync_ids_from_remote(
                &self,
                chunk_fk_map: &ChunkFkMapping,
            ) -> Result<FkPayload> {
                let mut payload = FkPayload::new();
                let model_unique_id = self.unique_id();
                $(
                    let fk_col_name = $fk_col.to_string();
                    let fk_parent_id_str = self.$fk_field.to_string();
                    let fk_sync_id = extract_sync_id_from_chunk_map(
                        chunk_fk_map,
                        $table_name,
                        &fk_col_name,
                        &fk_parent_id_str,
                        &model_unique_id,
                    );
                    payload.insert(fk_col_name, fk_sync_id);
                )*
                Ok(payload)
            }
        }

        #[async_trait]
        impl ActiveModelWithForeignKeyOps for $active_model {
            async fn remap_model_and_set_foreign_keys<E: DatabaseExecutor>(
                &mut self,
                fk_sync_id_payload: &FkPayload,
                db: &E,
            ) -> Result<()> {
                $(
                    let fk_col_name = $fk_col.to_string();
                    if let Some(sync_id_opt) = fk_sync_id_payload.get(&fk_col_name) {
                        let local_pk = set_foreign_key_from_sync_id::<$parent_entity, _>(
                            sync_id_opt.as_ref(),
                            db,
                            &fk_col_name,
                            false, // Assuming junction table FKs are not nullable
                        )
                        .await?
                        .ok_or_else(|| {
                            anyhow!(
                                "Failed to find local PK for {} using sync_id. Referenced entity may not exist locally.",
                                fk_col_name
                            )
                        })?;
                        self.$fk_field = ActiveValue::Set(local_pk);
                    }
                )*
                Ok(())
            }
        }
    };
}

// Generate implementations for junction tables (many-to-many relationships)
impl_junction_table_fk_ops!(
    media_file_albums::Model,
    media_file_albums::ActiveModel,
    "media_file_albums",
    [
        (
            album_id,
            media_file_albums::Column::AlbumId,
            albums::Entity,
            albums::Column::Id
        ),
        (
            media_file_id,
            media_file_albums::Column::MediaFileId,
            media_files::Entity,
            media_files::Column::Id
        )
    ]
);

impl_junction_table_fk_ops!(
    media_file_artists::Model,
    media_file_artists::ActiveModel,
    "media_file_artists",
    [
        (
            artist_id,
            media_file_artists::Column::ArtistId,
            artists::Entity,
            artists::Column::Id
        ),
        (
            media_file_id,
            media_file_artists::Column::MediaFileId,
            media_files::Entity,
            media_files::Column::Id
        )
    ]
);

impl_junction_table_fk_ops!(
    media_file_genres::Model,
    media_file_genres::ActiveModel,
    "media_file_genres",
    [
        (
            genre_id,
            media_file_genres::Column::GenreId,
            genres::Entity,
            genres::Column::Id
        ),
        (
            media_file_id,
            media_file_genres::Column::MediaFileId,
            media_files::Entity,
            media_files::Column::Id
        )
    ]
);

impl_junction_table_fk_ops!(
    media_file_fingerprint::Model,
    media_file_fingerprint::ActiveModel,
    "media_file_fingerprints",
    [(
        media_file_id,
        media_file_fingerprint::Column::MediaFileId,
        media_files::Entity,
        media_files::Column::Id
    )]
);

impl_junction_table_fk_ops!(
    media_file_similarity::Model,
    media_file_similarity::ActiveModel,
    "media_file_similarity",
    [
        (
            file_id1,
            media_file_similarity::Column::FileId1,
            media_files::Entity,
            media_files::Column::Id
        ),
        (
            file_id2,
            media_file_similarity::Column::FileId2,
            media_files::Entity,
            media_files::Column::Id
        )
    ]
);

#[async_trait]
impl ModelWithForeignKeyOps for media_files::Model {
    async fn extract_model_fk_sync_ids<E: DatabaseExecutor>(&self, db: &E) -> Result<FkPayload> {
        let mut payload = FkPayload::new();
        let fk_col_name = media_files::Column::CoverArtId.to_string();

        let cover_art_sync_id = get_referenced_sync_id::<media_cover_art::Entity, _>(
            db,
            self.cover_art_id,
            media_cover_art::Column::Id,
        )
        .await?;
        payload.insert(fk_col_name, cover_art_sync_id);

        Ok(payload)
    }

    async fn generate_model_fk_mappings_for_batch<DbEx: DatabaseExecutor>(
        records: &[Self],
        db: &DbEx,
    ) -> Result<ChunkFkMapping> {
        let mut overall_mapping = ChunkFkMapping::new();
        let fk_column_name = media_files::Column::CoverArtId.to_string();
        let mut column_specific_map = HashMap::new();

        for record_model in records {
            if let Some(parent_local_id) = record_model.cover_art_id
                && let hash_map::Entry::Vacant(e) =
                    column_specific_map.entry(parent_local_id.to_string())
            {
                let parent_sync_id = get_referenced_sync_id::<media_cover_art::Entity, _>(
                        db,
                        Some(parent_local_id),
                        media_cover_art::Column::Id,
                    )
                    .await
                    .with_context(|| {
                        format!(
                            "Failed to get sync_id for parent entity media_cover_art referenced by {fk_column_name} (local_id: {parent_local_id}) in child media_files"
                        )
                    })?;

                if let Some(sync_id_str) = parent_sync_id {
                    e.insert(sync_id_str);
                } else {
                    warn!(
                        "Could not find sync_id for parent media_cover_art referenced by media_files.{} = {} (child record {}). Parent might not exist or missing HLC UUID.",
                        fk_column_name,
                        parent_local_id,
                        record_model.unique_id()
                    );
                }
            }
        }
        if !column_specific_map.is_empty() {
            overall_mapping.insert(fk_column_name.to_string(), column_specific_map);
        }
        Ok(overall_mapping)
    }

    fn extract_model_sync_ids_from_remote(
        &self,
        chunk_fk_map: &ChunkFkMapping,
    ) -> Result<FkPayload> {
        let mut payload = FkPayload::new();
        let fk_col_name = media_files::Column::CoverArtId.to_string();
        let model_unique_id = self.unique_id();
        // It's better to get the table name programmatically if possible,
        // but for this specific impl, a const or string literal is also fine.
        const TABLE_NAME: &str = "media_files"; // Or media_files::Entity.table_name().to_string();

        if let Some(parent_local_id_i32) = self.cover_art_id {
            let parent_local_id_str = parent_local_id_i32.to_string();
            let sync_id = extract_sync_id_from_chunk_map(
                chunk_fk_map,
                TABLE_NAME,
                &fk_col_name,
                &parent_local_id_str,
                &model_unique_id,
            );
            payload.insert(fk_col_name.clone(), sync_id);
            // The original code had an additional warning if the fk_col_name itself was not in chunk_fk_map.
            // The helper function currently doesn't produce this specific warning, but returns None,
            // which is the correct behavior for the payload.
            // If that specific warning is important, the helper or this call site could be adjusted.
            // For now, this correctly uses the helper and resolves the 'Self' error.
            if payload.get(&fk_col_name).unwrap().is_none() // if sync_id is None
                && chunk_fk_map.get(&fk_col_name).is_none()
            // and the reason is column map missing
            {
                warn!(
                    "FK column map for '{}' not found in ChunkFkMapping for entity '{}' (remote model {}). Foreign key will be unresolved.",
                    fk_col_name,
                    TABLE_NAME,
                    self.unique_id()
                );
            }
        } else {
            // If the local FK is None, the remote FK reference is also None.
            payload.insert(fk_col_name, None);
        }
        Ok(payload)
    }
}

#[async_trait]
impl ActiveModelWithForeignKeyOps for media_files::ActiveModel {
    async fn remap_model_and_set_foreign_keys<E: DatabaseExecutor>(
        &mut self,
        fk_sync_id_payload: &FkPayload,
        db: &E,
    ) -> Result<()> {
        let fk_col_name = media_files::Column::CoverArtId.to_string();
        if let Some(cover_art_sync_id_opt_str) = fk_sync_id_payload.get(&fk_col_name) {
            let local_cover_art_pk = set_foreign_key_from_sync_id::<media_cover_art::Entity, _>(
                cover_art_sync_id_opt_str.as_ref(),
                db,
                &fk_col_name,
                true,
            )
            .await?;

            self.cover_art_id = ActiveValue::Set(local_cover_art_pk);

            if local_cover_art_pk.is_none() && cover_art_sync_id_opt_str.is_some() {
                debug!(
                    "CoverArt with sync_id {cover_art_sync_id_opt_str:?} not found locally for media_files.cover_art_id. Setting FK to NULL."
                );
            }
        }
        Ok(())
    }
}
