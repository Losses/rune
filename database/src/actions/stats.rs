use crate::error::ExpectedError;
use anyhow::{Result, anyhow};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter,
};
use uuid::Uuid;

use crate::entities::{media_file_likes as likes, media_file_stats as stats, media_files};

/// Apply a like assignment. This method does not author replication records.
async fn assign_liked<C: ConnectionTrait>(
    db: &C,
    file_id: i32,
    liked: bool,
    updated_at: &str,
) -> Result<likes::Model> {
    let file = media_files::Entity::find_by_id(file_id)
        .one(db)
        .await?
        .ok_or(ExpectedError::NotFound { what: "media file" })?;
    let existing = likes::Entity::find()
        .filter(likes::Column::TrackUuid.eq(&file.hlc_uuid))
        .one(db)
        .await?;
    let is_existing = existing.is_some();
    let mut row = existing
        .map(Into::into)
        .unwrap_or_else(|| likes::ActiveModel {
            track_uuid: Set(file.hlc_uuid.clone()),
            hlc_uuid: Set(file.hlc_uuid),
            ..Default::default()
        });
    row.liked = Set(liked);
    row.updated_at = Set(updated_at.to_owned());
    if is_existing {
        Ok(row.update(db).await?)
    } else {
        Ok(row.insert(db).await?)
    }
}

pub async fn get_liked<C: ConnectionTrait>(db: &C, file_id: i32) -> Result<bool> {
    use sea_orm::{JoinType, QuerySelect, RelationTrait};
    Ok(likes::Entity::find()
        .join(JoinType::InnerJoin, likes::Relation::MediaFiles.def())
        .filter(media_files::Column::Id.eq(file_id))
        .one(db)
        .await?
        .is_some_and(|row| row.liked))
}

pub async fn set_liked(
    db: &sea_orm::DatabaseConnection,
    store: &crate::sync::SeaOrmSyncStore,
    file_id: i32,
    liked: bool,
) -> Result<Option<likes::Model>> {
    use crate::sync::model::{ObjectKey, RuneObjectKey, author_source_on};
    use sea_orm::TransactionTrait;
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let Some(file) = media_files::Entity::find_by_id(file_id).one(&tx).await? else {
        return Ok(None);
    };
    let track_id = Uuid::parse_str(&file.hlc_uuid)?;
    if let Some(existing) = likes::Entity::find()
        .filter(likes::Column::TrackUuid.eq(&file.hlc_uuid))
        .one(&tx)
        .await?
        && existing.liked == liked
    {
        return Ok(Some(existing));
    }
    let row = assign_liked(&tx, file_id, liked, &Utc::now().to_rfc3339()).await?;
    author_source_on(&tx, store, ObjectKey::liked(track_id)).await?;
    tx.commit().await?;
    Ok(Some(row))
}

pub async fn increase_skipped(
    db: &sea_orm::DatabaseConnection,
    store: &crate::sync::SeaOrmSyncStore,
    file_id: i32,
    device_id: Uuid,
) -> Result<stats::Model> {
    author_increment(db, store, file_id, device_id, false).await
}
pub async fn increase_played_through(
    db: &sea_orm::DatabaseConnection,
    store: &crate::sync::SeaOrmSyncStore,
    file_id: i32,
    device_id: Uuid,
) -> Result<stats::Model> {
    author_increment(db, store, file_id, device_id, true).await
}
async fn author_increment(
    db: &sea_orm::DatabaseConnection,
    store: &crate::sync::SeaOrmSyncStore,
    file_id: i32,
    device_id: Uuid,
    played: bool,
) -> Result<stats::Model> {
    use crate::sync::model::{Collection, ObjectKey, author_source_on};
    use sea_orm::TransactionTrait;
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let row = increment_statistics(&tx, file_id, device_id, played).await?;
    let file = media_files::Entity::find_by_id(file_id)
        .one(&tx)
        .await?
        .ok_or(ExpectedError::NotFound { what: "media file" })?;
    let track_id = Uuid::parse_str(&file.hlc_uuid)?;
    let key = ObjectKey {
        collection: Collection::PlaybackStatistics,
        id: Uuid::new_v5(&track_id, device_id.as_bytes()),
    };
    author_source_on(&tx, store, key).await?;
    tx.commit().await?;
    Ok(row)
}

/// Increment only the supplied device's row within the caller's transaction.
async fn increment_statistics<C: ConnectionTrait>(
    db: &C,
    file_id: i32,
    device_id: Uuid,
    played: bool,
) -> Result<stats::Model> {
    use sea_orm::sea_query::{Expr, ExprTrait, OnConflict};
    let file = media_files::Entity::find_by_id(file_id)
        .one(db)
        .await?
        .ok_or(ExpectedError::NotFound { what: "media file" })?;
    let file_uuid = Uuid::parse_str(&file.hlc_uuid)?;
    let device = device_id.to_string();
    let now = Utc::now().to_rfc3339();
    let mut conflict = OnConflict::columns([stats::Column::TrackUuid, stats::Column::DeviceId]);
    conflict.value(stats::Column::UpdatedAt, Expr::value(now.clone()));
    if played {
        conflict.value(
            stats::Column::PlayedThrough,
            Expr::col(stats::Column::PlayedThrough).add(1),
        );
    } else {
        conflict.value(
            stats::Column::Skipped,
            Expr::col(stats::Column::Skipped).add(1),
        );
    }
    stats::Entity::insert(stats::ActiveModel {
        track_uuid: Set(file.hlc_uuid.clone()),
        device_id: Set(device.clone()),
        hlc_uuid: Set(Uuid::new_v5(&file_uuid, device_id.as_bytes()).to_string()),
        skipped: Set(i32::from(!played)),
        played_through: Set(i32::from(played)),
        updated_at: Set(now),
        ..Default::default()
    })
    .on_conflict(conflict.to_owned())
    .exec_without_returning(db)
    .await?;
    stats::Entity::find()
        .filter(stats::Column::TrackUuid.eq(&file.hlc_uuid))
        .filter(stats::Column::DeviceId.eq(device))
        .one(db)
        .await?
        .ok_or_else(|| anyhow!("statistics row missing after increment"))
}
