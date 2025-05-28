use std::collections::{hash_map, HashMap};
use std::fmt::Debug;

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use log::{debug, warn};
use sea_orm::{
    ActiveModelBehavior, ActiveValue, ColumnTrait, ConnectionTrait, EntityName, EntityTrait,
    FromQueryResult, Iden, Iterable, PrimaryKeyToColumn, PrimaryKeyTrait, QueryFilter, QuerySelect,
    TryGetable, Value,
};
use serde::Serialize;

use sync::chunking::ChunkFkMapping;
use sync::foreign_key::{DatabaseExecutor, FkPayload, ForeignKeyResolver};
use sync::hlc::{HLCModel, HLCRecord};

use crate::entities::{
    albums, artists, genres, media_cover_art, media_file_albums, media_file_artists,
    media_file_genres, media_files,
};

#[derive(Debug, Clone)]
pub struct RuneForeignKeyResolver;

// Helper to get sync_id (hlc_uuid) of a referenced entity
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
                std::any::type_name::<ReferencedEntity>()
            )
        })?;
        let pk_column_in_query = pk_def.into_column();

        #[derive(Debug)]
        struct PkOnlyModel<RE: EntityTrait>
        where
            <RE::PrimaryKey as PrimaryKeyTrait>::ValueType:
                Clone + TryGetable + Send + Sync + Debug + 'static,
        {
            pk_value: <RE::PrimaryKey as PrimaryKeyTrait>::ValueType,
            _phantom: std::marker::PhantomData<RE>,
        }

        impl<RE: EntityTrait> FromQueryResult for PkOnlyModel<RE>
        where
            <RE::PrimaryKey as PrimaryKeyTrait>::ValueType:
                Clone + TryGetable + Send + Sync + Debug + 'static,
        {
            fn from_query_result(
                res: &sea_orm::QueryResult,
                pre: &str,
            ) -> std::result::Result<Self, sea_orm::DbErr> {
                let expected_pk_col_name = RE::PrimaryKey::iter()
                    .next()
                    .ok_or_else(|| {
                        sea_orm::DbErr::Custom(format!(
                            "Entity {} has no PK for FromQueryResult",
                            std::any::type_name::<RE>()
                        ))
                    })?
                    .to_string();
                let val: <RE::PrimaryKey as PrimaryKeyTrait>::ValueType =
                    res.try_get(pre, &expected_pk_col_name)?;
                Ok(PkOnlyModel {
                    pk_value: val,
                    _phantom: std::marker::PhantomData,
                })
            }
        }

        let result = ReferencedEntity::find()
            .select_only()
            .column(pk_column_in_query)
            .filter(ReferencedEntity::unique_id_column().eq(sid_str.clone()))
            .into_model::<PkOnlyModel<ReferencedEntity>>()
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
    async fn extract_foreign_key_sync_ids<M: HLCRecord + Sync + Serialize, DbEx>(
        &self,
        entity_name: &str,
        model: &M,
        db: &DbEx,
    ) -> Result<FkPayload>
    where
        DbEx: DatabaseExecutor + ConnectionTrait,
    {
        let mut payload = FkPayload::new();
        match entity_name {
            "media_files" => {
                let concrete_model: &media_files::Model =
                    unsafe { &*(model as *const M as *const media_files::Model) };
                if let Some(cover_art_pk_val) = concrete_model.cover_art_id {
                    let cover_art_sync_id = get_referenced_sync_id::<media_cover_art::Entity, _>(
                        db,
                        Some(cover_art_pk_val),
                        media_cover_art::Column::Id,
                    )
                    .await?;
                    payload.insert("cover_art_id".to_string(), cover_art_sync_id);
                } else {
                    payload.insert("cover_art_id".to_string(), None);
                }
            }
            "media_file_albums" => {
                let concrete_model: &media_file_albums::Model =
                    unsafe { &*(model as *const M as *const media_file_albums::Model) };
                let album_sync_id = get_referenced_sync_id::<albums::Entity, _>(
                    db,
                    Some(concrete_model.album_id),
                    albums::Column::Id,
                )
                .await?;
                payload.insert("album_id".to_string(), album_sync_id);
                let media_file_sync_id = get_referenced_sync_id::<media_files::Entity, _>(
                    db,
                    Some(concrete_model.media_file_id),
                    media_files::Column::Id,
                )
                .await?;
                payload.insert("media_file_id".to_string(), media_file_sync_id);
            }
            "media_file_artists" => {
                let concrete_model: &media_file_artists::Model =
                    unsafe { &*(model as *const M as *const media_file_artists::Model) };
                let artist_sync_id = get_referenced_sync_id::<artists::Entity, _>(
                    db,
                    Some(concrete_model.artist_id),
                    artists::Column::Id,
                )
                .await?;
                payload.insert("artist_id".to_string(), artist_sync_id);
                let media_file_sync_id = get_referenced_sync_id::<media_files::Entity, _>(
                    db,
                    Some(concrete_model.media_file_id),
                    media_files::Column::Id,
                )
                .await?;
                payload.insert("media_file_id".to_string(), media_file_sync_id);
            }
            "media_file_genres" => {
                let concrete_model: &media_file_genres::Model =
                    unsafe { &*(model as *const M as *const media_file_genres::Model) };
                let genre_sync_id = get_referenced_sync_id::<genres::Entity, _>(
                    db,
                    Some(concrete_model.genre_id),
                    genres::Column::Id,
                )
                .await?;
                payload.insert("genre_id".to_string(), genre_sync_id);
                let media_file_sync_id = get_referenced_sync_id::<media_files::Entity, _>(
                    db,
                    Some(concrete_model.media_file_id),
                    media_files::Column::Id,
                )
                .await?;
                payload.insert("media_file_id".to_string(), media_file_sync_id);
            }
            _ => {}
        }
        Ok(payload)
    }

    async fn remap_and_set_foreign_keys<AM: ActiveModelBehavior + Send, DbEx>(
        &self,
        entity_name: &str,
        active_model: &mut AM,
        fk_sync_id_payload: &FkPayload,
        db: &DbEx,
    ) -> Result<()>
    where
        DbEx: DatabaseExecutor + ConnectionTrait,
    {
        // ... (rest of the function remains the same)
        if entity_name == media_files::Entity::table_name(&media_files::Entity) {
            let concrete_am: &mut media_files::ActiveModel =
                unsafe { &mut *(active_model as *mut AM as *mut media_files::ActiveModel) };
            let fk_col_name = media_files::Column::CoverArtId.to_string();
            if let Some(cover_art_sync_id_opt_str) = fk_sync_id_payload.get(&fk_col_name) {
                if let Some(cover_art_sync_id_str) = cover_art_sync_id_opt_str {
                    let local_cover_art_pk_opt =
                        get_local_pk_from_sync_id::<media_cover_art::Entity, _>(
                            db,
                            Some(cover_art_sync_id_str),
                        )
                        .await?;
                    concrete_am.cover_art_id = ActiveValue::Set(local_cover_art_pk_opt);
                    if local_cover_art_pk_opt.is_none() {
                        debug!(
                            "CoverArt with sync_id {} not found locally for media_files.cover_art_id. Setting to NULL.",
                            cover_art_sync_id_str
                        );
                    }
                } else {
                    concrete_am.cover_art_id = ActiveValue::Set(None);
                }
            } else {
                concrete_am.cover_art_id = ActiveValue::NotSet;
            }
        } else if entity_name == media_file_albums::Entity::table_name(&media_file_albums::Entity) {
            let concrete_am: &mut media_file_albums::ActiveModel =
                unsafe { &mut *(active_model as *mut AM as *mut media_file_albums::ActiveModel) };
            let album_id_col_name = media_file_albums::Column::AlbumId.to_string();
            let album_sync_id = fk_sync_id_payload
                .get(&album_id_col_name)
                .and_then(|opt_s| opt_s.as_ref())
                .ok_or_else(|| {
                    anyhow!(
                        "FkPayload missing mandatory FK sync_id for '{}.{}'",
                        entity_name,
                        album_id_col_name
                    )
                })?;
            let local_album_pk =
                get_local_pk_from_sync_id::<albums::Entity, _>(db, Some(album_sync_id))
                    .await?
                    .ok_or_else(|| {
                        anyhow!(
                            "Failed to find local PK for {}.{} using sync_id: {}",
                            entity_name,
                            album_id_col_name,
                            album_sync_id
                        )
                    })?;
            concrete_am.album_id = ActiveValue::Set(local_album_pk);
            let media_file_id_col_name = media_file_albums::Column::MediaFileId.to_string();
            let media_file_sync_id = fk_sync_id_payload
                .get(&media_file_id_col_name)
                .and_then(|opt_s| opt_s.as_ref())
                .ok_or_else(|| {
                    anyhow!(
                        "FkPayload missing mandatory FK sync_id for '{}.{}'",
                        entity_name,
                        media_file_id_col_name
                    )
                })?;
            let local_media_file_pk =
                get_local_pk_from_sync_id::<media_files::Entity, _>(db, Some(media_file_sync_id))
                    .await?
                    .ok_or_else(|| {
                        anyhow!(
                            "Failed to find local PK for {}.{} using sync_id: {}",
                            entity_name,
                            media_file_id_col_name,
                            media_file_sync_id
                        )
                    })?;
            concrete_am.media_file_id = ActiveValue::Set(local_media_file_pk);
        } else if entity_name == media_file_artists::Entity::table_name(&media_file_artists::Entity)
        {
            let concrete_am: &mut media_file_artists::ActiveModel =
                unsafe { &mut *(active_model as *mut AM as *mut media_file_artists::ActiveModel) };
            let artist_id_col_name = media_file_artists::Column::ArtistId.to_string();
            let artist_sync_id = fk_sync_id_payload
                .get(&artist_id_col_name)
                .and_then(|opt_s| opt_s.as_ref())
                .ok_or_else(|| {
                    anyhow!(
                        "FkPayload missing mandatory FK sync_id for '{}.{}'",
                        entity_name,
                        artist_id_col_name
                    )
                })?;
            let local_artist_pk =
                get_local_pk_from_sync_id::<artists::Entity, _>(db, Some(artist_sync_id))
                    .await?
                    .ok_or_else(|| {
                        anyhow!(
                            "Failed to find local PK for {}.{} using sync_id: {}",
                            entity_name,
                            artist_id_col_name,
                            artist_sync_id
                        )
                    })?;
            concrete_am.artist_id = ActiveValue::Set(local_artist_pk);
            let media_file_id_col_name = media_file_artists::Column::MediaFileId.to_string();
            let media_file_sync_id = fk_sync_id_payload
                .get(&media_file_id_col_name)
                .and_then(|opt_s| opt_s.as_ref())
                .ok_or_else(|| {
                    anyhow!(
                        "FkPayload missing mandatory FK sync_id for '{}.{}'",
                        entity_name,
                        media_file_id_col_name
                    )
                })?;
            let local_media_file_pk =
                get_local_pk_from_sync_id::<media_files::Entity, _>(db, Some(media_file_sync_id))
                    .await?
                    .ok_or_else(|| {
                        anyhow!(
                            "Failed to find local PK for {}.{} using sync_id: {}",
                            entity_name,
                            media_file_id_col_name,
                            media_file_sync_id
                        )
                    })?;
            concrete_am.media_file_id = ActiveValue::Set(local_media_file_pk);
        } else if entity_name == media_file_genres::Entity::table_name(&media_file_genres::Entity) {
            let concrete_am: &mut media_file_genres::ActiveModel =
                unsafe { &mut *(active_model as *mut AM as *mut media_file_genres::ActiveModel) };
            let genre_id_col_name = media_file_genres::Column::GenreId.to_string();
            let genre_sync_id = fk_sync_id_payload
                .get(&genre_id_col_name)
                .and_then(|opt_s| opt_s.as_ref())
                .ok_or_else(|| {
                    anyhow!(
                        "FkPayload missing mandatory FK sync_id for '{}.{}'",
                        entity_name,
                        genre_id_col_name
                    )
                })?;
            let local_genre_pk =
                get_local_pk_from_sync_id::<genres::Entity, _>(db, Some(genre_sync_id))
                    .await?
                    .ok_or_else(|| {
                        anyhow!(
                            "Failed to find local PK for {}.{} using sync_id: {}",
                            entity_name,
                            genre_id_col_name,
                            genre_sync_id
                        )
                    })?;
            concrete_am.genre_id = ActiveValue::Set(local_genre_pk);

            let media_file_id_col_name = media_file_genres::Column::MediaFileId.to_string();
            let media_file_sync_id = fk_sync_id_payload
                .get(&media_file_id_col_name)
                .and_then(|opt_s| opt_s.as_ref())
                .ok_or_else(|| {
                    anyhow!(
                        "FkPayload missing mandatory FK sync_id for '{}.{}'",
                        entity_name,
                        media_file_id_col_name
                    )
                })?;
            let local_media_file_pk =
                get_local_pk_from_sync_id::<media_files::Entity, _>(db, Some(media_file_sync_id))
                    .await?
                    .ok_or_else(|| {
                        anyhow!(
                            "Failed to find local PK for {}.{} using sync_id: {}",
                            entity_name,
                            media_file_id_col_name,
                            media_file_sync_id
                        )
                    })?;
            concrete_am.media_file_id = ActiveValue::Set(local_media_file_pk);
        }
        Ok(())
    }

    async fn generate_fk_mappings_for_records<M, DbEx>(
        &self,
        entity_name: &str,
        records: &[M],
        db: &DbEx,
    ) -> Result<ChunkFkMapping>
    where
        M: HLCRecord + Sync + Serialize,
        DbEx: DatabaseExecutor + ConnectionTrait,
    {
        let mut overall_mapping = ChunkFkMapping::new();

        // Auxiliary function, adjusted for type correctness
        async fn process_fk_column<ParentEntity, RecordModel, DBE>(
            child_entity_name_for_log: &str,
            fk_column_name: &str,
            records_slice: &[RecordModel],
            // MODIFIED: Closure returns Option of ParentEntity's PK ValueType
            get_parent_local_id_fn: impl Fn(
                &RecordModel,
            ) -> Option<
                <ParentEntity::PrimaryKey as PrimaryKeyTrait>::ValueType,
            >,
            parent_entity_pk_col: ParentEntity::Column,
            overall_map: &mut ChunkFkMapping,
            db_conn: &DBE,
        ) -> Result<()>
        where
            ParentEntity: EntityTrait + HLCModel,
            // MODIFIED: Added ToString bound for parent_local_id.to_string()
            <ParentEntity::PrimaryKey as PrimaryKeyTrait>::ValueType:
                Into<Value> + Eq + Copy + Send + Sync + Debug + ToString,
            RecordModel: HLCRecord + Sync + Serialize,
            DBE: DatabaseExecutor + ConnectionTrait,
        {
            let mut column_specific_map = HashMap::new();
            for record_model in records_slice {
                if let Some(parent_local_id) = get_parent_local_id_fn(record_model) {
                    if let hash_map::Entry::Vacant(e) =
                        column_specific_map.entry(parent_local_id.to_string())
                    {
                        let parent_sync_id = get_referenced_sync_id::<ParentEntity, DBE>(
                            db_conn,
                            Some(parent_local_id), // parent_local_id is Copy, so this is fine
                            parent_entity_pk_col,  // ParentEntity::Column is Copy, so this is fine
                        )
                        .await
                        .with_context(|| {
                            format!(
                                "Failed to get sync_id for parent entity referenced by {} (local_id: {}) in child {}",
                                fk_column_name, parent_local_id.to_string(), child_entity_name_for_log
                            )
                        })?;

                        if let Some(sync_id_str) = parent_sync_id {
                            e.insert(sync_id_str);
                        } else {
                            warn!(
                                "Could not find sync_id for parent referenced by {}.{} = {} (child record {}). Parent might not exist or missing HLC UUID.",
                                child_entity_name_for_log, fk_column_name, parent_local_id.to_string(), record_model.unique_id()
                            );
                        }
                    }
                }
            }
            if !column_specific_map.is_empty() {
                overall_map.insert(fk_column_name.to_string(), column_specific_map);
            }
            Ok(())
        }

        match entity_name {
            "media_files" => {
                let concrete_records: &[media_files::Model] =
                    unsafe { std::mem::transmute(records) }; // Assuming M is media_files::Model

                // Call to process_fk_column, RecordModel will be media_files::Model
                // ParentEntity is media_cover_art::Entity. Its PK ValueType is i32.
                // Closure |rec| rec.cover_art_id returns Option<i32>, which matches.
                process_fk_column::<media_cover_art::Entity, _, _>(
                    // ChildEntity removed, RecordModel inferred (or explicit)
                    "media_files",
                    media_files::Column::CoverArtId.to_string().as_str(),
                    concrete_records,
                    |rec| rec.cover_art_id, // rec is &media_files::Model
                    media_cover_art::Column::Id,
                    &mut overall_mapping,
                    db,
                )
                .await?;
            }
            "media_file_albums" => {
                let concrete_records: &[media_file_albums::Model] =
                    unsafe { std::mem::transmute(records) }; // Assuming M is media_file_albums::Model

                // For album_id: ParentEntity is albums::Entity. PK ValueType i32.
                // Closure |rec| Some(rec.album_id) returns Option<i32>. Matches.
                process_fk_column::<albums::Entity, _, _>(
                    "media_file_albums",
                    media_file_albums::Column::AlbumId.to_string().as_str(),
                    concrete_records,
                    |rec| Some(rec.album_id), // rec is &media_file_albums::Model
                    albums::Column::Id,
                    &mut overall_mapping,
                    db,
                )
                .await?;

                // For media_file_id: ParentEntity is media_files::Entity. PK ValueType i32.
                // Closure |rec| Some(rec.media_file_id) returns Option<i32>. Matches.
                process_fk_column::<media_files::Entity, _, _>(
                    "media_file_albums",
                    media_file_albums::Column::MediaFileId.to_string().as_str(),
                    concrete_records,
                    |rec| Some(rec.media_file_id), // rec is &media_file_albums::Model
                    media_files::Column::Id,
                    &mut overall_mapping,
                    db,
                )
                .await?;
            }
            "media_file_artists" => {
                let concrete_records: &[media_file_artists::Model] =
                    unsafe { std::mem::transmute(records) };

                process_fk_column::<artists::Entity, _, _>(
                    "media_file_artists",
                    media_file_artists::Column::ArtistId.to_string().as_str(),
                    concrete_records,
                    |rec| Some(rec.artist_id),
                    artists::Column::Id,
                    &mut overall_mapping,
                    db,
                )
                .await?;

                process_fk_column::<media_files::Entity, _, _>(
                    "media_file_artists",
                    media_file_artists::Column::MediaFileId.to_string().as_str(),
                    concrete_records,
                    |rec| Some(rec.media_file_id),
                    media_files::Column::Id,
                    &mut overall_mapping,
                    db,
                )
                .await?;
            }
            "media_file_genres" => {
                let concrete_records: &[media_file_genres::Model] =
                    unsafe { std::mem::transmute(records) };

                process_fk_column::<genres::Entity, _, _>(
                    "media_file_genres",
                    media_file_genres::Column::GenreId.to_string().as_str(),
                    concrete_records,
                    |rec| Some(rec.genre_id),
                    genres::Column::Id,
                    &mut overall_mapping,
                    db,
                )
                .await?;

                process_fk_column::<media_files::Entity, _, _>(
                    "media_file_genres",
                    media_file_genres::Column::MediaFileId.to_string().as_str(),
                    concrete_records,
                    |rec| Some(rec.media_file_id),
                    media_files::Column::Id,
                    &mut overall_mapping,
                    db,
                )
                .await?;
            }
            _ => {}
        }
        Ok(overall_mapping)
    }

    fn extract_sync_ids_from_remote_model_with_mapping<M: HLCRecord + Send + Sync + Serialize>(
        &self,
        entity_name: &str,
        remote_model: &M,
        chunk_fk_map: Option<&ChunkFkMapping>,
    ) -> Result<FkPayload> {
        // ... (rest of the function remains the same)
        let mut payload = FkPayload::new();
        let Some(fk_map) = chunk_fk_map else {
            bail!(
                "Missing ChunkFkMapping for entity '{}', record '{}'",
                entity_name,
                remote_model.unique_id()
            );
        };
        match entity_name {
            "media_files" => {
                let concrete_model: &media_files::Model =
                    unsafe { &*(remote_model as *const M as *const media_files::Model) };
                let fk_col_name = media_files::Column::CoverArtId.to_string();
                if let Some(parent_local_id_i32) = concrete_model.cover_art_id {
                    let parent_local_id_str = parent_local_id_i32.to_string();
                    if let Some(col_map) = fk_map.get(&fk_col_name) {
                        if let Some(sync_id) = col_map.get(&parent_local_id_str) {
                            payload.insert(fk_col_name, Some(sync_id.clone()));
                        } else {
                            warn!("Sync ID not found in map for {}.{} = {} (remote model {}). Foreign key will be unresolved.", entity_name, fk_col_name, parent_local_id_str, remote_model.unique_id());
                            payload.insert(fk_col_name, None);
                        }
                    } else {
                        warn!("FK column '{}' not found in ChunkFkMapping for entity '{}' (remote model {}).", fk_col_name, entity_name, remote_model.unique_id());
                        payload.insert(fk_col_name, None);
                    }
                } else {
                    payload.insert(fk_col_name, None);
                }
            }
            "media_file_albums" => {
                let concrete_model: &media_file_albums::Model =
                    unsafe { &*(remote_model as *const M as *const media_file_albums::Model) };
                let album_fk_col = media_file_albums::Column::AlbumId.to_string();
                let album_parent_id_str = concrete_model.album_id.to_string();
                let album_sync_id = fk_map
                    .get(&album_fk_col)
                    .and_then(|col_map| col_map.get(&album_parent_id_str))
                    .cloned();
                if album_sync_id.is_none() {
                    warn!(
                        "Sync ID not found in map for {}.{} = {}",
                        entity_name, album_fk_col, album_parent_id_str
                    );
                }
                payload.insert(album_fk_col, album_sync_id);
                let mf_fk_col = media_file_albums::Column::MediaFileId.to_string();
                let mf_parent_id_str = concrete_model.media_file_id.to_string();
                let mf_sync_id = fk_map
                    .get(&mf_fk_col)
                    .and_then(|col_map| col_map.get(&mf_parent_id_str))
                    .cloned();
                if mf_sync_id.is_none() {
                    warn!(
                        "Sync ID not found in map for {}.{} = {}",
                        entity_name, mf_fk_col, mf_parent_id_str
                    );
                }
                payload.insert(mf_fk_col, mf_sync_id);
            }
            "media_file_artists" => {
                let concrete_model: &media_file_artists::Model =
                    unsafe { &*(remote_model as *const M as *const media_file_artists::Model) };
                let artist_fk_col = media_file_artists::Column::ArtistId.to_string();
                let artist_parent_id_str = concrete_model.artist_id.to_string();
                let artist_sync_id = fk_map
                    .get(&artist_fk_col)
                    .and_then(|m| m.get(&artist_parent_id_str))
                    .cloned();
                if artist_sync_id.is_none() {
                    warn!(
                        "Sync ID not found in map for {}.{} = {}",
                        entity_name, artist_fk_col, artist_parent_id_str
                    );
                }
                payload.insert(artist_fk_col, artist_sync_id);
                let mf_fk_col = media_file_artists::Column::MediaFileId.to_string();
                let mf_parent_id_str = concrete_model.media_file_id.to_string();
                let mf_sync_id = fk_map
                    .get(&mf_fk_col)
                    .and_then(|m| m.get(&mf_parent_id_str))
                    .cloned();
                if mf_sync_id.is_none() {
                    warn!(
                        "Sync ID not found in map for {}.{} = {}",
                        entity_name, mf_fk_col, mf_parent_id_str
                    );
                }
                payload.insert(mf_fk_col, mf_sync_id);
            }
            "media_file_genres" => {
                let concrete_model: &media_file_genres::Model =
                    unsafe { &*(remote_model as *const M as *const media_file_genres::Model) };
                let genre_fk_col = media_file_genres::Column::GenreId.to_string();
                let genre_parent_id_str = concrete_model.genre_id.to_string();
                let genre_sync_id = fk_map
                    .get(&genre_fk_col)
                    .and_then(|m| m.get(&genre_parent_id_str))
                    .cloned();
                if genre_sync_id.is_none() {
                    warn!(
                        "Sync ID not found in map for {}.{} = {}",
                        entity_name, genre_fk_col, genre_parent_id_str
                    );
                }
                payload.insert(genre_fk_col, genre_sync_id);
                let mf_fk_col = media_file_genres::Column::MediaFileId.to_string();
                let mf_parent_id_str = concrete_model.media_file_id.to_string();
                let mf_sync_id = fk_map
                    .get(&mf_fk_col)
                    .and_then(|m| m.get(&mf_parent_id_str))
                    .cloned();
                if mf_sync_id.is_none() {
                    warn!(
                        "Sync ID not found in map for {}.{} = {}",
                        entity_name, mf_fk_col, mf_parent_id_str
                    );
                }
                payload.insert(mf_fk_col, mf_sync_id);
            }
            _ => {}
        }
        Ok(payload)
    }
}
