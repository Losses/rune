#[path = "support/causal_library.rs"]
mod support;

use anyhow::Result;
use database::actions::stats::get_liked;
use database::{
    entities::{media_file_likes, media_files},
    sync::{bindings, model::*},
};
use sea_orm::{ConnectionTrait, EntityTrait, TransactionTrait};
use support::{SqlLibrary, key};
use uuid::Uuid;

#[tokio::test]
async fn projection_can_restore_lower_value_and_remove_without_authored_revision() -> Result<()> {
    let local = SqlLibrary::new(Uuid::new_v4(), Uuid::new_v4()).await?;
    let track = Uuid::new_v4();
    local
        .peer
        .write_local(key(Collection::Tracks, track), Some(support::track()))
        .await?;
    let file = media_files::Entity::find().one(&local.db).await?.unwrap();
    let key = key(Collection::Likes, track);
    let like = |liked| {
        Some(Payload::Liked(Liked {
            track_id: track,
            liked,
            updated_at: "2026-11-01T00:00:00Z".into(),
        }))
    };
    let lower = local.peer.write_local(key.clone(), like(true)).await?;
    let higher = local.peer.write_local(key.clone(), like(false)).await?;
    let tx = local.db.begin().await?;
    bindings::apply(&tx, &higher.key, higher.value.as_ref()).await?;
    assert!(!get_liked(&tx, file.id).await?);
    bindings::apply(&tx, &lower.key, lower.value.as_ref()).await?;
    assert!(get_liked(&tx, file.id).await?);
    tx.commit().await?;
    local.db.execute_unprepared("CREATE TRIGGER reject_projection BEFORE DELETE ON media_file_likes BEGIN SELECT RAISE(ABORT, 'test'); END").await?;
    let tx = local.db.begin().await?;
    assert!(bindings::apply(&tx, &key, None).await.is_err());
    tx.rollback().await?;
    assert!(get_liked(&local.db, file.id).await?);
    assert_eq!(local.peer.head().await?.sequence, 3);
    local
        .db
        .execute_unprepared("DROP TRIGGER reject_projection")
        .await?;
    let tx = local.db.begin().await?;
    bindings::apply(&tx, &key, None).await?;
    assert!(media_file_likes::Entity::find().one(&tx).await?.is_none());
    tx.commit().await?;
    assert_eq!(local.peer.head().await?.sequence, 3);
    assert!(
        media_files::Entity::find_by_id(file.id)
            .one(&local.db)
            .await?
            .is_some()
    );
    Ok(())
}
