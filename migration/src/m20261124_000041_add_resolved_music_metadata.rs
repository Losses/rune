use sea_orm_migration::prelude::*;

#[path = "resolved_metadata_schema.rs"]
mod schema;
#[path = "resolved_metadata_triggers.rs"]
mod triggers;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        schema::install(manager).await?;
        schema::migrate_artists(manager).await?;
        schema::migrate_albums(manager).await?;
        schema::move_table(
            manager,
            "media_file_albums",
            "music_metadata_track_album_sources",
        )
        .await?;
        schema::move_table(manager, "media_metadata", "music_metadata_source_tags").await?;
        triggers::install(manager).await?;
        super::resolved_metadata_import::import(manager).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        triggers::uninstall(manager).await?;
        schema::restore_artists(manager).await?;
        schema::restore_albums(manager).await?;
        schema::uninstall(manager).await?;
        schema::move_table(
            manager,
            "music_metadata_track_album_sources",
            "media_file_albums",
        )
        .await?;
        schema::move_table(manager, "music_metadata_source_tags", "media_metadata").await
    }
}
