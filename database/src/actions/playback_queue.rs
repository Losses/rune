use anyhow::Result;
use sea_orm::{EntityTrait, QueryOrder, Set};
use sea_orm::{TransactionTrait, prelude::*};

use crate::entities::playback_queue;

pub async fn replace_playback_queue(
    main_db: &DatabaseConnection,
    media_file_ids: Vec<i32>,
) -> Result<()> {
    use playback_queue::Entity as PlaybackQueueEntity;

    let txn = main_db.begin().await?;

    PlaybackQueueEntity::delete_many().exec(&txn).await?;

    for media_file_id in media_file_ids {
        let new_entry = playback_queue::ActiveModel {
            media_file_id: Set(media_file_id),
            ..Default::default()
        };
        new_entry.insert(&txn).await?;
    }

    txn.commit().await?;

    Ok(())
}

pub async fn list_playback_queue(db: &DatabaseConnection) -> Result<Vec<i32>> {
    use playback_queue::Entity as PlaybackQueueEntity;

    let entries = PlaybackQueueEntity::find()
        .order_by_asc(playback_queue::Column::Id)
        .all(db)
        .await?;

    let media_file_ids = entries
        .into_iter()
        .map(|entry| entry.media_file_id)
        .collect();

    Ok(media_file_ids)
}
