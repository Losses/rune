use database::test_fixtures::causal_library as support;

use anyhow::{Context, Result};
use database::entities::{media_cover_art, media_files};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

/// A library as the migration leaves it: a cover over the size limit kept without a digest, as
/// it is unavailable, and a track that uses it. Returns the track's and the cover's UUIDs.
async fn library_with_oversized_cover(local: &support::SqlLibrary) -> Result<(String, String)> {
    let cover = Uuid::new_v4().to_string();
    media_cover_art::Entity::insert(media_cover_art::ActiveModel {
        file_hash: Set("oversized".into()),
        binary: Set(vec![0; 16]),
        primary_color: Set(None),
        content_hash: Set(None),
        content_size: Set(None),
        local_available: Set(false),
        source_uuid: Set(cover.clone()),
        ..Default::default()
    })
    .exec(&local.db)
    .await?;
    let track = Uuid::new_v4().to_string();
    media_files::Entity::insert(media_files::ActiveModel {
        file_name: Set("with oversized cover".into()),
        directory: Set(String::new()),
        extension: Set("flac".into()),
        file_hash: Set("track".into()),
        last_modified: Set("old".into()),
        sample_rate: Set(44100),
        duration: Set(rust_decimal::Decimal::ONE),
        cover_art_uuid: Set(Some(cover.clone())),
        source_uuid: Set(track.clone()),
        ..Default::default()
    })
    .exec(&local.db)
    .await?;
    Ok((track, cover))
}

// A library that has such a cover must still open.
#[tokio::test]
async fn import_skips_a_cover_the_migration_left_without_a_digest() -> Result<()> {
    let local = support::SqlLibrary::new(Uuid::new_v4(), Uuid::new_v4()).await?;
    library_with_oversized_cover(&local).await?;
    local.sync_store.import_basic().await?;
    Ok(())
}

// The track still reaches a peer, without the cover, like a track whose cover has not arrived.
#[tokio::test]
async fn a_track_whose_cover_was_left_out_still_syncs() -> Result<()> {
    let namespace = Uuid::new_v4();
    let local = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let (track, cover) = library_with_oversized_cover(&local).await?;
    local.sync_store.import_basic().await?;
    let remote = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    support::exchange(&local.peer, &remote.peer).await?;
    let received = media_files::Entity::find()
        .filter(media_files::Column::SourceUuid.eq(&track))
        .one(&remote.db)
        .await?
        .context("the track did not reach the peer")?;
    assert_eq!(received.cover_art_uuid.as_deref(), Some(cover.as_str()));
    assert!(
        media_cover_art::Entity::find()
            .filter(media_cover_art::Column::SourceUuid.eq(&cover))
            .one(&remote.db)
            .await?
            .is_none()
    );
    Ok(())
}
