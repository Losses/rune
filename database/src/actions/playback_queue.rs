use anyhow::Result;
use sea_orm::{EntityTrait, QueryOrder, Set};
use sea_orm::{TransactionTrait, prelude::*};

use crate::entities::playback_queue::{self, Entity as PlaybackQueueEntity};

use playback::player::PlayingItem;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, OnceLock, RwLock, Weak},
};
use uuid::Uuid;

pub type SessionQueue = RwLock<Vec<PlayingItem>>;
type QueueBinding = (Uuid, Weak<SessionQueue>);
static SESSION_QUEUES: OnceLock<RwLock<HashMap<PathBuf, QueueBinding>>> = OnceLock::new();

fn catalog_path(db: &DatabaseConnection) -> PathBuf {
    db.get_sqlite_connection_pool()
        .connect_options()
        .get_filename()
        .to_owned()
}

/// Bind a catalog connection (including clones) to this node's session queue.
pub fn bind_session_queue(db: &DatabaseConnection, library_id: Uuid, queue: &Arc<SessionQueue>) {
    SESSION_QUEUES
        .get_or_init(RwLock::default)
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(catalog_path(db), (library_id, Arc::downgrade(queue)));
}

pub async fn replace_playback_queue(
    main_db: &DatabaseConnection,
    media_file_ids: Vec<i32>,
) -> Result<()> {
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
    let binding = SESSION_QUEUES
        .get_or_init(RwLock::default)
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&catalog_path(db))
        .and_then(|(id, queue)| queue.upgrade().map(|queue| (*id, queue)));
    if let Some((library_id, queue)) = binding {
        let id = library_id.to_string();
        return Ok(queue
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter_map(|item| match item {
                PlayingItem::InLibrary(file_id, library_id) if library_id == &id => Some(*file_id),
                _ => None,
            })
            .collect());
    }
    list_legacy_playback_queue(db).await
}

/// Read the old catalog table only for the one-time session migration.
pub async fn list_legacy_playback_queue(db: &DatabaseConnection) -> Result<Vec<i32>> {
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
