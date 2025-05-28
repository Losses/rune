use std::collections::{hash_map, HashMap};
use std::fmt::Debug;
use std::marker::PhantomData;

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use log::{debug, warn};
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, Iden, Iterable,
    PrimaryKeyToColumn, PrimaryKeyTrait, QueryFilter, QuerySelect, TryGetable, Value,
};

use sync::chunking::ChunkFkMapping;
use sync::foreign_key::{
    ActiveModelWithForeignKeyOps, DatabaseExecutor, FkPayload, ForeignKeyResolver,
    ModelWithForeignKeyOps,
};
use sync::hlc::{HLCModel, HLCRecord};

use crate::entities::{
    albums, artists, genres, media_cover_art, media_file_albums, media_file_artists,
    media_file_genres, media_files,
};

#[derive(Debug, Clone)]
pub struct RuneForeignKeyResolver;

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

// Helper to get local PK from a sync_id
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
                // RE is now constrained and known here.
                let pk_item_def = RE::PrimaryKey::iter().next().ok_or_else(|| {
                    sea_orm::DbErr::Custom(format!(
                        "Entity {} has no primary key defined (in PkOnlyModelHelper for table {})",
                        std::any::type_name::<RE>(),
                        RE::table_name(&RE::default())
                    ))
                })?;
                // The column selected was pk_def.into_column(). We need its string name.
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
            .column(pk_column_in_query) // Select the actual PK column
            .filter(ReferencedEntity::unique_id_column().eq(sid_str.clone()))
            // Pass ReferencedEntity as the generic argument to PkOnlyModelHelper
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
            // This resolver requires the map to be present to call the model's method,
            // as the model's method `extract_model_sync_ids_from_remote` expects a non-Option map.
            bail!(
                "Missing ChunkFkMapping for remote model '{}' (unique_id: {}). RuneForeignKeyResolver expects it.",
                std::any::type_name::<M>(), // Log model type for easier debugging
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

#[async_trait]
impl ModelWithForeignKeyOps for media_files::Model {
    async fn extract_model_fk_sync_ids<E: DatabaseExecutor>(
        &self, // self is &media_files::Model
        db: &E,
    ) -> Result<FkPayload> {
        let mut payload = FkPayload::new();
        let fk_col_name = media_files::Column::CoverArtId.to_string();
        if let Some(cover_art_pk_val) = self.cover_art_id {
            // Direct field access
            let cover_art_sync_id = get_referenced_sync_id::<media_cover_art::Entity, _>(
                db,
                Some(cover_art_pk_val),
                media_cover_art::Column::Id,
            )
            .await?;
            payload.insert(fk_col_name, cover_art_sync_id);
        } else {
            payload.insert(fk_col_name, None);
        }
        Ok(payload)
    }

    async fn generate_model_fk_mappings_for_batch<DbEx: DatabaseExecutor>(
        records: &[Self], // Self is media_files::Model
        db: &DbEx,
    ) -> Result<ChunkFkMapping> {
        let mut overall_mapping = ChunkFkMapping::new();
        let fk_column_name = media_files::Column::CoverArtId.to_string();
        let mut column_specific_map = HashMap::new();

        for record_model in records {
            if let Some(parent_local_id) = record_model.cover_art_id {
                if let hash_map::Entry::Vacant(e) =
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
                            "Failed to get sync_id for parent entity media_cover_art referenced by {} (local_id: {}) in child media_files",
                            fk_column_name, parent_local_id
                        )
                    })?;

                    if let Some(sync_id_str) = parent_sync_id {
                        e.insert(sync_id_str);
                    } else {
                        warn!(
                                "Could not find sync_id for parent media_cover_art referenced by media_files.{} = {} (child record {}). Parent might not exist or missing HLC UUID.",
                                fk_column_name, parent_local_id, record_model.unique_id()
                            );
                    }
                }
            }
        }
        if !column_specific_map.is_empty() {
            overall_mapping.insert(fk_column_name.to_string(), column_specific_map);
        }
        Ok(overall_mapping)
    }

    fn extract_model_sync_ids_from_remote(
        &self, // self is &media_files::Model
        chunk_fk_map: &ChunkFkMapping,
    ) -> Result<FkPayload> {
        let mut payload = FkPayload::new();
        let fk_col_name = media_files::Column::CoverArtId.to_string();

        if let Some(parent_local_id_i32) = self.cover_art_id {
            let parent_local_id_str = parent_local_id_i32.to_string();
            if let Some(col_map) = chunk_fk_map.get(&fk_col_name) {
                if let Some(sync_id) = col_map.get(&parent_local_id_str) {
                    payload.insert(fk_col_name.clone(), Some(sync_id.clone()));
                } else {
                    // Warn if the specific local ID is not in the map for this FK column
                    warn!("Sync ID not found in map for media_files.{} = {} (remote model {}). Foreign key will be unresolved.", fk_col_name, parent_local_id_str, self.unique_id());
                    payload.insert(fk_col_name.clone(), None);
                }
            } else {
                // Warn if the FK column itself is not in the ChunkFkMapping
                warn!("FK column '{}' not found in ChunkFkMapping for entity 'media_files' (remote model {}). Foreign key will be unresolved.", fk_col_name, self.unique_id());
                payload.insert(fk_col_name.clone(), None);
            }
        } else {
            // FK is NULL on the remote model
            payload.insert(fk_col_name, None);
        }
        Ok(payload)
    }
}

#[async_trait]
impl ActiveModelWithForeignKeyOps for media_files::ActiveModel {
    async fn remap_model_and_set_foreign_keys<E: DatabaseExecutor>(
        &mut self, // self is &mut media_files::ActiveModel
        fk_sync_id_payload: &FkPayload,
        db: &E,
    ) -> Result<()> {
        let fk_col_name = media_files::Column::CoverArtId.to_string();
        if let Some(cover_art_sync_id_opt_str) = fk_sync_id_payload.get(&fk_col_name) {
            if let Some(cover_art_sync_id_str) = cover_art_sync_id_opt_str {
                let local_cover_art_pk_opt =
                    get_local_pk_from_sync_id::<media_cover_art::Entity, _>(
                        db,
                        Some(cover_art_sync_id_str),
                    )
                    .await?;
                self.cover_art_id = ActiveValue::Set(local_cover_art_pk_opt);
                if local_cover_art_pk_opt.is_none() {
                    debug!(
                        "CoverArt with sync_id {} not found locally for media_files.cover_art_id. Setting FK to NULL.",
                        cover_art_sync_id_str
                    );
                }
            } else {
                // FK was explicitly None in payload
                self.cover_art_id = ActiveValue::Set(None);
            }
        }
        // If fk_col_name is not in fk_sync_id_payload, cover_art_id remains Unchanged or as previously set.
        Ok(())
    }
}

// Example for media_file_albums (Junction Table with two FKs)
#[async_trait]
impl ModelWithForeignKeyOps for media_file_albums::Model {
    async fn extract_model_fk_sync_ids<E: DatabaseExecutor>(&self, db: &E) -> Result<FkPayload> {
        let mut payload = FkPayload::new();
        // Album FK
        let album_fk_col_name = media_file_albums::Column::AlbumId.to_string();
        let album_sync_id = get_referenced_sync_id::<albums::Entity, _>(
            db,
            Some(self.album_id),
            albums::Column::Id,
        )
        .await?;
        payload.insert(album_fk_col_name, album_sync_id);

        // MediaFile FK
        let mf_fk_col_name = media_file_albums::Column::MediaFileId.to_string();
        let media_file_sync_id = get_referenced_sync_id::<media_files::Entity, _>(
            db,
            Some(self.media_file_id),
            media_files::Column::Id,
        )
        .await?;
        payload.insert(mf_fk_col_name, media_file_sync_id);
        Ok(payload)
    }

    async fn generate_model_fk_mappings_for_batch<DbEx: DatabaseExecutor>(
        records: &[Self],
        db: &DbEx,
    ) -> Result<ChunkFkMapping> {
        let mut overall_mapping = ChunkFkMapping::new();

        // Helper closure for repeated logic
        async fn gen_map_for_col<ParentEntity, Rec, GetParentIdFn, DBE>(
            records_slice: &[Rec],
            get_parent_local_id_fn: GetParentIdFn,
            parent_entity_pk_col: ParentEntity::Column, // PK column in ParentEntity table (e.g., albums::Column::Id)
            db_conn: &DBE,
        ) -> Result<Option<HashMap<String, String>>>
        where
            ParentEntity: EntityTrait + HLCModel,
            <ParentEntity::PrimaryKey as PrimaryKeyTrait>::ValueType:
                Into<Value> + Eq + Copy + Send + Sync + Debug + ToString,
            Rec: HLCRecord + Sync, // The record type being processed (e.g., media_file_albums::Model)
            GetParentIdFn: Fn(&Rec) -> <ParentEntity::PrimaryKey as PrimaryKeyTrait>::ValueType,
            DBE: DatabaseExecutor,
        {
            let mut col_specific_map = HashMap::new();
            for r_model in records_slice {
                let parent_local_id = get_parent_local_id_fn(r_model); // e.g., r_model.album_id
                if let hash_map::Entry::Vacant(e) =
                    col_specific_map.entry(parent_local_id.to_string())
                {
                    if let Some(sync_id_str) = get_referenced_sync_id::<ParentEntity, _>(
                        db_conn,
                        Some(parent_local_id), // The local PK value of the parent
                        parent_entity_pk_col,  // The column name of the PK in the parent table
                    )
                    .await?
                    {
                        e.insert(sync_id_str);
                    } else {
                        warn!("Could not find sync_id for parent {} referenced by local_id {} (child record {}). Parent might not exist or missing HLC UUID.",
                            std::any::type_name::<ParentEntity>(), parent_local_id.to_string(), r_model.unique_id());
                    }
                }
            }
            if col_specific_map.is_empty() {
                Ok(None)
            } else {
                Ok(Some(col_specific_map))
            }
        }

        let album_fk_col_str = media_file_albums::Column::AlbumId.to_string();
        if let Some(map) = gen_map_for_col::<albums::Entity, _, _, _>(
            records,
            |r| r.album_id,
            albums::Column::Id, // PK column in albums table
            db,
        )
        .await?
        {
            overall_mapping.insert(album_fk_col_str, map);
        }

        let mf_fk_col_str = media_file_albums::Column::MediaFileId.to_string();
        if let Some(map) = gen_map_for_col::<media_files::Entity, _, _, _>(
            records,
            |r| r.media_file_id,
            media_files::Column::Id, // PK column in media_files table
            db,
        )
        .await?
        {
            overall_mapping.insert(mf_fk_col_str, map);
        }
        Ok(overall_mapping)
    }

    fn extract_model_sync_ids_from_remote(
        &self,
        chunk_fk_map: &ChunkFkMapping,
    ) -> Result<FkPayload> {
        let mut payload = FkPayload::new();

        let album_fk_col = media_file_albums::Column::AlbumId.to_string();
        let album_parent_id_str = self.album_id.to_string();
        let album_sync_id = chunk_fk_map
            .get(&album_fk_col)
            .and_then(|col_map| col_map.get(&album_parent_id_str))
            .cloned();
        if album_sync_id.is_none()
            && chunk_fk_map
                .get(&album_fk_col)
                .map_or(false, |m| m.get(&album_parent_id_str).is_none())
        {
            warn!(
                "Sync ID not found in map for media_file_albums.{} = {} (remote model {}). Child of non-existent parent?",
                album_fk_col,
                album_parent_id_str,
                self.unique_id()
            );
        }
        payload.insert(album_fk_col.clone(), album_sync_id);

        let mf_fk_col = media_file_albums::Column::MediaFileId.to_string();
        let mf_parent_id_str = self.media_file_id.to_string();
        let mf_sync_id = chunk_fk_map
            .get(&mf_fk_col)
            .and_then(|col_map| col_map.get(&mf_parent_id_str))
            .cloned();
        if mf_sync_id.is_none()
            && chunk_fk_map
                .get(&mf_fk_col)
                .map_or(false, |m| m.get(&mf_parent_id_str).is_none())
        {
            warn!(
                "Sync ID not found in map for media_file_albums.{} = {} (remote model {}). Child of non-existent parent?",
                mf_fk_col,
                mf_parent_id_str,
                self.unique_id()
            );
        }
        payload.insert(mf_fk_col.clone(), mf_sync_id);
        Ok(payload)
    }
}

#[async_trait]
impl ActiveModelWithForeignKeyOps for media_file_albums::ActiveModel {
    async fn remap_model_and_set_foreign_keys<E: DatabaseExecutor>(
        &mut self,
        fk_sync_id_payload: &FkPayload,
        db: &E,
    ) -> Result<()> {
        let album_id_col_name = media_file_albums::Column::AlbumId.to_string();
        if let Some(album_sync_id_opt) = fk_sync_id_payload.get(&album_id_col_name) {
            // For junction tables, FKs are usually not nullable.
            let album_sync_id = album_sync_id_opt.as_ref().ok_or_else(|| {
                anyhow!(
                    "FkPayload missing mandatory sync_id for non-nullable FK media_file_albums.{}",
                    album_id_col_name
                )
            })?;
            let local_album_pk =
                get_local_pk_from_sync_id::<albums::Entity, _>(db, Some(album_sync_id))
                    .await?
                    .ok_or_else(|| {
                        anyhow!(
                            "Failed to find local PK for media_file_albums.{} (FK to albums) using sync_id: {}. Referenced album may not exist locally.",
                            album_id_col_name,
                            album_sync_id
                        )
                    })?;
            self.album_id = ActiveValue::Set(local_album_pk);
        }

        let media_file_id_col_name = media_file_albums::Column::MediaFileId.to_string();
        if let Some(mf_sync_id_opt) = fk_sync_id_payload.get(&media_file_id_col_name) {
            let media_file_sync_id = mf_sync_id_opt.as_ref().ok_or_else(|| {
                anyhow!(
                    "FkPayload missing mandatory sync_id for non-nullable FK media_file_albums.{}",
                    media_file_id_col_name
                )
            })?;
            let local_media_file_pk =
                get_local_pk_from_sync_id::<media_files::Entity, _>(db, Some(media_file_sync_id))
                    .await?
                    .ok_or_else(|| {
                        anyhow!(
                            "Failed to find local PK for media_file_albums.{} (FK to media_files) using sync_id: {}. Referenced media_file may not exist locally.",
                            media_file_id_col_name,
                            media_file_sync_id
                        )
                    })?;
            self.media_file_id = ActiveValue::Set(local_media_file_pk);
        }
        Ok(())
    }
}

#[async_trait]
impl ModelWithForeignKeyOps for artists::Model {
    async fn extract_model_fk_sync_ids<E: DatabaseExecutor>(&self, _db: &E) -> Result<FkPayload> {
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
impl ActiveModelWithForeignKeyOps for artists::ActiveModel {
    async fn remap_model_and_set_foreign_keys<E: DatabaseExecutor>(
        &mut self,
        _fk_sync_id_payload: &FkPayload,
        _db: &E,
    ) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl ModelWithForeignKeyOps for genres::Model {
    async fn extract_model_fk_sync_ids<E: DatabaseExecutor>(&self, _db: &E) -> Result<FkPayload> {
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
impl ActiveModelWithForeignKeyOps for genres::ActiveModel {
    async fn remap_model_and_set_foreign_keys<E: DatabaseExecutor>(
        &mut self,
        _fk_sync_id_payload: &FkPayload,
        _db: &E,
    ) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl ModelWithForeignKeyOps for media_cover_art::Model {
    async fn extract_model_fk_sync_ids<E: DatabaseExecutor>(&self, _db: &E) -> Result<FkPayload> {
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
impl ActiveModelWithForeignKeyOps for media_cover_art::ActiveModel {
    async fn remap_model_and_set_foreign_keys<E: DatabaseExecutor>(
        &mut self,
        _fk_sync_id_payload: &FkPayload,
        _db: &E,
    ) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl ModelWithForeignKeyOps for media_file_artists::Model {
    async fn extract_model_fk_sync_ids<E: DatabaseExecutor>(&self, db: &E) -> Result<FkPayload> {
        let mut payload = FkPayload::new();
        let artist_fk_col_name = media_file_artists::Column::ArtistId.to_string();
        let artist_sync_id = get_referenced_sync_id::<artists::Entity, _>(
            db,
            Some(self.artist_id),
            artists::Column::Id,
        )
        .await?;
        payload.insert(artist_fk_col_name, artist_sync_id);

        let mf_fk_col_name = media_file_artists::Column::MediaFileId.to_string();
        let media_file_sync_id = get_referenced_sync_id::<media_files::Entity, _>(
            db,
            Some(self.media_file_id),
            media_files::Column::Id,
        )
        .await?;
        payload.insert(mf_fk_col_name, media_file_sync_id);
        Ok(payload)
    }

    async fn generate_model_fk_mappings_for_batch<DbEx: DatabaseExecutor>(
        records: &[Self],
        db: &DbEx,
    ) -> Result<ChunkFkMapping> {
        let mut overall_mapping = ChunkFkMapping::new();
        // Helper closure (copied and adapted from media_file_albums)
        async fn gen_map_for_col<ParentEntity, Rec, GetParentIdFn, DBE>(
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
                if let hash_map::Entry::Vacant(e) =
                    col_specific_map.entry(parent_local_id.to_string())
                {
                    if let Some(sync_id_str) = get_referenced_sync_id::<ParentEntity, _>(
                        db_conn,
                        Some(parent_local_id),
                        parent_entity_pk_col,
                    )
                    .await?
                    {
                        e.insert(sync_id_str);
                    } else {
                        warn!("Could not find sync_id for parent {} referenced by local_id {} (child record {}). Parent might not exist or missing HLC UUID.",
                            std::any::type_name::<ParentEntity>(), parent_local_id.to_string(), r_model.unique_id());
                    }
                }
            }
            if col_specific_map.is_empty() {
                Ok(None)
            } else {
                Ok(Some(col_specific_map))
            }
        }

        let artist_fk_col = media_file_artists::Column::ArtistId.to_string();
        if let Some(map) = gen_map_for_col::<artists::Entity, _, _, _>(
            records,
            |r| r.artist_id,
            artists::Column::Id,
            db,
        )
        .await?
        {
            overall_mapping.insert(artist_fk_col, map);
        }

        let mf_fk_col = media_file_artists::Column::MediaFileId.to_string();
        if let Some(map) = gen_map_for_col::<media_files::Entity, _, _, _>(
            records,
            |r| r.media_file_id,
            media_files::Column::Id,
            db,
        )
        .await?
        {
            overall_mapping.insert(mf_fk_col, map);
        }
        Ok(overall_mapping)
    }

    fn extract_model_sync_ids_from_remote(
        &self,
        chunk_fk_map: &ChunkFkMapping,
    ) -> Result<FkPayload> {
        let mut payload = FkPayload::new();
        let artist_fk_col = media_file_artists::Column::ArtistId.to_string();
        let artist_parent_id_str = self.artist_id.to_string();
        let artist_sync_id = chunk_fk_map
            .get(&artist_fk_col)
            .and_then(|m| m.get(&artist_parent_id_str))
            .cloned();
        if artist_sync_id.is_none()
            && chunk_fk_map
                .get(&artist_fk_col)
                .map_or(false, |m| m.get(&artist_parent_id_str).is_none())
        {
            warn!(
                "Sync ID not found in map for media_file_artists.{} = {} (remote model {}). Child of non-existent parent?",
                artist_fk_col,
                artist_parent_id_str,
                self.unique_id()
            );
        }
        payload.insert(artist_fk_col.clone(), artist_sync_id);

        let mf_fk_col = media_file_artists::Column::MediaFileId.to_string();
        let mf_parent_id_str = self.media_file_id.to_string();
        let mf_sync_id = chunk_fk_map
            .get(&mf_fk_col)
            .and_then(|m| m.get(&mf_parent_id_str))
            .cloned();
        if mf_sync_id.is_none()
            && chunk_fk_map
                .get(&mf_fk_col)
                .map_or(false, |m| m.get(&mf_parent_id_str).is_none())
        {
            warn!(
                "Sync ID not found in map for media_file_artists.{} = {} (remote model {}). Child of non-existent parent?",
                mf_fk_col,
                mf_parent_id_str,
                self.unique_id()
            );
        }
        payload.insert(mf_fk_col.clone(), mf_sync_id);
        Ok(payload)
    }
}

#[async_trait]
impl ActiveModelWithForeignKeyOps for media_file_artists::ActiveModel {
    async fn remap_model_and_set_foreign_keys<E: DatabaseExecutor>(
        &mut self,
        fk_sync_id_payload: &FkPayload,
        db: &E,
    ) -> Result<()> {
        let artist_id_col_name = media_file_artists::Column::ArtistId.to_string();
        if let Some(sync_id_opt) = fk_sync_id_payload.get(&artist_id_col_name) {
            let sync_id = sync_id_opt.as_ref().ok_or_else(|| {
                anyhow!(
                    "FkPayload missing mandatory sync_id for non-nullable FK media_file_artists.{}",
                    artist_id_col_name
                )
            })?;
            let local_pk = get_local_pk_from_sync_id::<artists::Entity, _>(db, Some(sync_id))
                .await?
                .ok_or_else(|| {
                    anyhow!(
                        "Failed to find local PK for media_file_artists.{} (FK to artists) using sync_id: {}. Referenced artist may not exist locally.",
                        artist_id_col_name,
                        sync_id
                    )
                })?;
            self.artist_id = ActiveValue::Set(local_pk);
        }

        let media_file_id_col_name = media_file_artists::Column::MediaFileId.to_string();
        if let Some(sync_id_opt) = fk_sync_id_payload.get(&media_file_id_col_name) {
            let sync_id = sync_id_opt.as_ref().ok_or_else(|| {
                anyhow!(
                    "FkPayload missing mandatory sync_id for non-nullable FK media_file_artists.{}",
                    media_file_id_col_name
                )
            })?;
            let local_pk = get_local_pk_from_sync_id::<media_files::Entity, _>(db, Some(sync_id))
                .await?
                .ok_or_else(|| {
                    anyhow!(
                        "Failed to find local PK for media_file_artists.{} (FK to media_files) using sync_id: {}. Referenced media_file may not exist locally.",
                        media_file_id_col_name,
                        sync_id
                    )
                })?;
            self.media_file_id = ActiveValue::Set(local_pk);
        }
        Ok(())
    }
}

#[async_trait]
impl ModelWithForeignKeyOps for media_file_genres::Model {
    async fn extract_model_fk_sync_ids<E: DatabaseExecutor>(&self, db: &E) -> Result<FkPayload> {
        let mut payload = FkPayload::new();
        let genre_fk_col_name = media_file_genres::Column::GenreId.to_string();
        let genre_sync_id = get_referenced_sync_id::<genres::Entity, _>(
            db,
            Some(self.genre_id),
            genres::Column::Id,
        )
        .await?;
        payload.insert(genre_fk_col_name, genre_sync_id);

        let mf_fk_col_name = media_file_genres::Column::MediaFileId.to_string();
        let media_file_sync_id = get_referenced_sync_id::<media_files::Entity, _>(
            db,
            Some(self.media_file_id),
            media_files::Column::Id,
        )
        .await?;
        payload.insert(mf_fk_col_name, media_file_sync_id);
        Ok(payload)
    }

    async fn generate_model_fk_mappings_for_batch<DbEx: DatabaseExecutor>(
        records: &[Self],
        db: &DbEx,
    ) -> Result<ChunkFkMapping> {
        let mut overall_mapping = ChunkFkMapping::new();
        // Helper closure (copied and adapted)
        async fn gen_map_for_col<ParentEntity, Rec, GetParentIdFn, DBE>(
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
                if let hash_map::Entry::Vacant(e) =
                    col_specific_map.entry(parent_local_id.to_string())
                {
                    if let Some(sync_id_str) = get_referenced_sync_id::<ParentEntity, _>(
                        db_conn,
                        Some(parent_local_id),
                        parent_entity_pk_col,
                    )
                    .await?
                    {
                        e.insert(sync_id_str);
                    } else {
                        warn!("Could not find sync_id for parent {} referenced by local_id {} (child record {}). Parent might not exist or missing HLC UUID.",
                            std::any::type_name::<ParentEntity>(), parent_local_id.to_string(), r_model.unique_id());
                    }
                }
            }
            if col_specific_map.is_empty() {
                Ok(None)
            } else {
                Ok(Some(col_specific_map))
            }
        }

        let genre_fk_col = media_file_genres::Column::GenreId.to_string();
        if let Some(map) = gen_map_for_col::<genres::Entity, _, _, _>(
            records,
            |r| r.genre_id,
            genres::Column::Id,
            db,
        )
        .await?
        {
            overall_mapping.insert(genre_fk_col, map);
        }

        let mf_fk_col = media_file_genres::Column::MediaFileId.to_string();
        if let Some(map) = gen_map_for_col::<media_files::Entity, _, _, _>(
            records,
            |r| r.media_file_id,
            media_files::Column::Id,
            db,
        )
        .await?
        {
            overall_mapping.insert(mf_fk_col, map);
        }
        Ok(overall_mapping)
    }

    fn extract_model_sync_ids_from_remote(
        &self,
        chunk_fk_map: &ChunkFkMapping,
    ) -> Result<FkPayload> {
        let mut payload = FkPayload::new();
        let genre_fk_col = media_file_genres::Column::GenreId.to_string();
        let genre_parent_id_str = self.genre_id.to_string();
        let genre_sync_id = chunk_fk_map
            .get(&genre_fk_col)
            .and_then(|m| m.get(&genre_parent_id_str))
            .cloned();
        if genre_sync_id.is_none()
            && chunk_fk_map
                .get(&genre_fk_col)
                .map_or(false, |m| m.get(&genre_parent_id_str).is_none())
        {
            warn!(
                "Sync ID not found in map for media_file_genres.{} = {} (remote model {}). Child of non-existent parent?",
                genre_fk_col,
                genre_parent_id_str,
                self.unique_id()
            );
        }
        payload.insert(genre_fk_col.clone(), genre_sync_id);

        let mf_fk_col = media_file_genres::Column::MediaFileId.to_string();
        let mf_parent_id_str = self.media_file_id.to_string();
        let mf_sync_id = chunk_fk_map
            .get(&mf_fk_col)
            .and_then(|m| m.get(&mf_parent_id_str))
            .cloned();
        if mf_sync_id.is_none()
            && chunk_fk_map
                .get(&mf_fk_col)
                .map_or(false, |m| m.get(&mf_parent_id_str).is_none())
        {
            warn!(
                "Sync ID not found in map for media_file_genres.{} = {} (remote model {}). Child of non-existent parent?",
                mf_fk_col,
                mf_parent_id_str,
                self.unique_id()
            );
        }
        payload.insert(mf_fk_col.clone(), mf_sync_id);
        Ok(payload)
    }
}

#[async_trait]
impl ActiveModelWithForeignKeyOps for media_file_genres::ActiveModel {
    async fn remap_model_and_set_foreign_keys<E: DatabaseExecutor>(
        &mut self,
        fk_sync_id_payload: &FkPayload,
        db: &E,
    ) -> Result<()> {
        let genre_id_col_name = media_file_genres::Column::GenreId.to_string();
        if let Some(sync_id_opt) = fk_sync_id_payload.get(&genre_id_col_name) {
            let sync_id = sync_id_opt.as_ref().ok_or_else(|| {
                anyhow!(
                    "FkPayload missing mandatory sync_id for non-nullable FK media_file_genres.{}",
                    genre_id_col_name
                )
            })?;
            let local_pk = get_local_pk_from_sync_id::<genres::Entity, _>(db, Some(sync_id))
                .await?
                .ok_or_else(|| {
                    anyhow!(
                        "Failed to find local PK for media_file_genres.{} (FK to genres) using sync_id: {}. Referenced genre may not exist locally.",
                        genre_id_col_name,
                        sync_id
                    )
                })?;
            self.genre_id = ActiveValue::Set(local_pk);
        }

        let media_file_id_col_name = media_file_genres::Column::MediaFileId.to_string();
        if let Some(sync_id_opt) = fk_sync_id_payload.get(&media_file_id_col_name) {
            let sync_id = sync_id_opt.as_ref().ok_or_else(|| {
                anyhow!(
                    "FkPayload missing mandatory sync_id for non-nullable FK media_file_genres.{}",
                    media_file_id_col_name
                )
            })?;
            let local_pk = get_local_pk_from_sync_id::<media_files::Entity, _>(db, Some(sync_id))
                .await?
                .ok_or_else(|| {
                    anyhow!(
                        "Failed to find local PK for media_file_genres.{} (FK to media_files) using sync_id: {}. Referenced media_file may not exist locally.",
                        media_file_id_col_name,
                        sync_id
                    )
                })?;
            self.media_file_id = ActiveValue::Set(local_pk);
        }
        Ok(())
    }
}
