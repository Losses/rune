#[path = "support/causal_library.rs"]
mod support;

use anyhow::Result;
use database::entities::{media_cover_art, media_files};
use sea_orm::{EntityTrait, Set};
use uuid::Uuid;

// The library migration keeps a cover over the size limit but gives it no digest, as it is
// unavailable. A library that has one, with tracks that use it, must still open.
#[tokio::test]
async fn import_skips_a_cover_the_migration_left_without_a_digest() -> Result<()> {
    let local = support::SqlLibrary::new(Uuid::new_v4(), Uuid::new_v4()).await?;
    let cover = Uuid::new_v4().to_string();
    media_cover_art::Entity::insert(media_cover_art::ActiveModel {
        file_hash: Set("oversized".into()),
        binary: Set(vec![0; 16]),
        primary_color: Set(None),
        content_hash: Set(None),
        content_size: Set(None),
        local_available: Set(false),
        hlc_uuid: Set(cover.clone()),
        ..Default::default()
    })
    .exec(&local.db)
    .await?;
    media_files::Entity::insert(media_files::ActiveModel {
        file_name: Set("with oversized cover".into()),
        directory: Set(String::new()),
        extension: Set("flac".into()),
        file_hash: Set("track".into()),
        last_modified: Set("old".into()),
        sample_rate: Set(44100),
        duration: Set(rust_decimal::Decimal::ONE),
        cover_art_uuid: Set(Some(cover)),
        hlc_uuid: Set(Uuid::new_v4().to_string()),
        ..Default::default()
    })
    .exec(&local.db)
    .await?;
    local.library.import_basic().await?;
    Ok(())
}
