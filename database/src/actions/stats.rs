use anyhow::Result;
use chrono::Utc;
use sea_orm::ActiveValue;
use sea_orm::prelude::*;

use crate::entities::media_file_stats;
use crate::entities::media_files;

/// Set the liked status of a media file.
///
/// # Arguments
/// * `main_db` - A reference to the database connection.
/// * `media_file_id` - The ID of the media file to update.
/// * `liked` - The new liked status.
///
/// # Returns
/// * `Result<Model>` - The updated media file stats model or an error.
pub async fn set_liked(
    main_db: &DatabaseConnection,
    media_file_id: i32,
    liked: bool,
) -> Result<Option<media_file_stats::Model>> {
    let media_file = media_files::Entity::find_by_id(media_file_id)
        .one(main_db)
        .await?;

    if media_file.is_none() {
        return Ok(None);
    }

    // Find the media file stats by media file ID
    let stats = media_file_stats::Entity::find()
        .filter(media_file_stats::Column::MediaFileId.eq(media_file_id))
        .one(main_db)
        .await?;

    let updated_stats = if let Some(stats) = stats {
        let mut active_model: media_file_stats::ActiveModel = stats.into();

        // Update the liked status
        active_model.liked = ActiveValue::Set(liked);
        active_model.updated_at = ActiveValue::Set(Utc::now().to_rfc3339());

        // Update the media file stats in the database
        active_model.update(main_db).await?
    } else {
        // Create a new media file stats record
        let new_stats = media_file_stats::ActiveModel {
            media_file_id: ActiveValue::Set(media_file_id),
            liked: ActiveValue::Set(liked),
            skipped: ActiveValue::Set(0),
            played_through: ActiveValue::Set(0),
            updated_at: ActiveValue::Set(Utc::now().to_rfc3339()),
            ..Default::default()
        };

        new_stats.insert(main_db).await?
    };

    Ok(Some(updated_stats))
}

/// Get the liked status of a media file.
///
/// # Arguments
/// * `main_db` - A reference to the database connection.
/// * `media_file_id` - The ID of the media file to update.
///
/// # Returns
/// * `Result<Model>` - The updated media file stats model or an error.
pub async fn get_liked(main_db: &DatabaseConnection, media_file_id: i32) -> Result<bool> {
    use media_file_stats::Entity as MediaFileStatsEntity;

    // Find the media file stats by media file ID
    let stats = MediaFileStatsEntity::find()
        .filter(media_file_stats::Column::MediaFileId.eq(media_file_id))
        .one(main_db)
        .await?;

    match stats {
        Some(stats) => Ok(stats.liked),
        None => Ok(false),
    }
}

/// Increase the skipped count of a media file.
///
/// # Arguments
/// * `main_db` - A reference to the database connection.
/// * `media_file_id` - The ID of the media file to update.
///
/// # Returns
/// * `Result<Model>` - The updated media file stats model or an error.
pub async fn increase_skipped(
    main_db: &DatabaseConnection,
    media_file_id: i32,
) -> Result<media_file_stats::Model> {
    use media_file_stats::Entity as MediaFileStatsEntity;

    // Find the media file stats by media file ID
    let stats = MediaFileStatsEntity::find()
        .filter(media_file_stats::Column::MediaFileId.eq(media_file_id))
        .one(main_db)
        .await?;

    let updated_stats = if let Some(stats) = stats {
        let mut active_model: media_file_stats::ActiveModel = stats.clone().into();

        // Increase the skipped count
        active_model.skipped = ActiveValue::Set(stats.skipped + 1);
        active_model.updated_at = ActiveValue::Set(Utc::now().to_rfc3339());

        // Update the media file stats in the database
        active_model.update(main_db).await?
    } else {
        // Create a new media file stats record
        let new_stats = media_file_stats::ActiveModel {
            media_file_id: ActiveValue::Set(media_file_id),
            liked: ActiveValue::Set(false),
            skipped: ActiveValue::Set(1),
            played_through: ActiveValue::Set(0),
            updated_at: ActiveValue::Set(Utc::now().to_rfc3339()),
            ..Default::default()
        };

        new_stats.insert(main_db).await?
    };

    Ok(updated_stats)
}

/// Increase the played through count of a media file.
///
/// # Arguments
/// * `main_db` - A reference to the database connection.
/// * `media_file_id` - The ID of the media file to update.
///
/// # Returns
/// * `Result<Model>` - The updated media file stats model or an error.
pub async fn increase_played_through(
    main_db: &DatabaseConnection,
    media_file_id: i32,
) -> Result<media_file_stats::Model> {
    use media_file_stats::Entity as MediaFileStatsEntity;

    // Find the media file stats by media file ID
    let stats = MediaFileStatsEntity::find()
        .filter(media_file_stats::Column::MediaFileId.eq(media_file_id))
        .one(main_db)
        .await?;

    let updated_stats = if let Some(stats) = stats {
        let mut active_model: media_file_stats::ActiveModel = stats.clone().into();

        // Increase the played through count
        active_model.played_through = ActiveValue::Set(stats.played_through + 1);
        active_model.updated_at = ActiveValue::Set(Utc::now().to_rfc3339());

        // Update the media file stats in the database
        active_model.update(main_db).await?
    } else {
        // Create a new media file stats record
        let new_stats = media_file_stats::ActiveModel {
            media_file_id: ActiveValue::Set(media_file_id),
            liked: ActiveValue::Set(false),
            skipped: ActiveValue::Set(0),
            played_through: ActiveValue::Set(1),
            updated_at: ActiveValue::Set(Utc::now().to_rfc3339()),
            ..Default::default()
        };

        new_stats.insert(main_db).await?
    };

    Ok(updated_stats)
}
