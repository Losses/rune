use std::fmt::Debug;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use log::debug;
use sea_orm::{
    ActiveModelBehavior, ActiveValue, ColumnTrait, ConnectionTrait, EntityName, EntityTrait,
    FromQueryResult, Iden, Iterable, PrimaryKeyToColumn, PrimaryKeyTrait, QueryFilter, QuerySelect,
    TryGetable, Value,
};
use serde::Serialize;

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
    ReferencedEntity: EntityTrait + HLCModel, // The entity being referenced
    // Ensure the PK value type is usable in queries
    <ReferencedEntity::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Into<Value> + Eq + Copy + Send + Sync + Debug,
    Db: ConnectionTrait + DatabaseExecutor,
{
    if let Some(fk_val) = model_fk_value {
        // Temporary struct to select only the hlc_uuid
        #[derive(Debug, FromQueryResult)]
        struct HlcUuidOnly {
            hlc_uuid: String,
        }

        let result = ReferencedEntity::find()
            .select_only()
            .column_as(ReferencedEntity::unique_id_column(), "hlc_uuid") // Alias to match struct
            .filter(referenced_entity_pk_col.eq(fk_val))
            .into_model::<HlcUuidOnly>() // Use the temporary struct
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
        Clone + TryGetable + Send + Sync + Debug + 'static, // Changed TryFrom<Value> to TryGetable
    // And ensure DbErr is compatible if TryGetable has an Error type
    // DbErr from TryGetable::try_get_by must be compatible with anyhow::Error
    // This is usually fine as DbErr implements std::error::Error.
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

                // Now try_get should work because ValueType is TryGetable
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
        model: &M, // This is the concrete Model struct
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
        active_model: &mut AM, // AM is generic
        fk_sync_id_payload: &FkPayload,
        db: &DbEx,
    ) -> Result<()>
    where
        DbEx: DatabaseExecutor + ConnectionTrait,
    {
        if entity_name == media_files::Entity::table_name(&media_files::Entity) {
            // CORRECTED
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
            // CORRECTED
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
            // CORRECTED
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
            // CORRECTED
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

    fn extract_sync_ids_from_remote_model<M: HLCRecord + Send + Sync + Serialize>(
        &self,
        entity_name: &str,
        remote_model_with_sync_id_fks: &M,
    ) -> Result<FkPayload> {
        let mut payload = FkPayload::new();
        // This still relies on the strong assumption that the integer FK fields
        // in the remote model are actually string representations of sync_ids.
        match entity_name {
            "media_files" => {
                let concrete_model: &media_files::Model = unsafe {
                    &*(remote_model_with_sync_id_fks as *const M as *const media_files::Model)
                };
                payload.insert(
                    "cover_art_id".to_string(),
                    concrete_model.cover_art_id.map(|id| id.to_string()),
                );
            }
            "media_file_albums" => {
                let concrete_model: &media_file_albums::Model = unsafe {
                    &*(remote_model_with_sync_id_fks as *const M as *const media_file_albums::Model)
                };
                payload.insert(
                    "album_id".to_string(),
                    Some(concrete_model.album_id.to_string()),
                );
                payload.insert(
                    "media_file_id".to_string(),
                    Some(concrete_model.media_file_id.to_string()),
                );
            }
            "media_file_artists" => {
                let concrete_model: &media_file_artists::Model = unsafe {
                    &*(remote_model_with_sync_id_fks as *const M
                        as *const media_file_artists::Model)
                };
                payload.insert(
                    "artist_id".to_string(),
                    Some(concrete_model.artist_id.to_string()),
                );
                payload.insert(
                    "media_file_id".to_string(),
                    Some(concrete_model.media_file_id.to_string()),
                );
            }
            "media_file_genres" => {
                let concrete_model: &media_file_genres::Model = unsafe {
                    &*(remote_model_with_sync_id_fks as *const M as *const media_file_genres::Model)
                };
                payload.insert(
                    "genre_id".to_string(),
                    Some(concrete_model.genre_id.to_string()),
                );
                payload.insert(
                    "media_file_id".to_string(),
                    Some(concrete_model.media_file_id.to_string()),
                );
            }
            _ => {}
        }
        Ok(payload)
    }
}
