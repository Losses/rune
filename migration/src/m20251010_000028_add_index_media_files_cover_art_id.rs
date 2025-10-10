use sea_orm_migration::prelude::*;

use super::m20230701_000001_create_media_files_table::MediaFiles;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20251010_000028_add_index_media_files_cover_art_id"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add index on cover_art_id to speed up UPDATE queries during cover art processing
        manager
            .create_index(
                Index::create()
                    .name("idx_media_files_cover_art_id")
                    .table(MediaFiles::Table)
                    .col(MediaFiles::CoverArtId)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_media_files_cover_art_id")
                    .to_owned(),
            )
            .await
    }
}
