use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20251010_000027_add_index_cover_art_file_hash"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .name("idx_media_cover_art_file_hash")
                    .table(MediaCoverArt::Table)
                    .col(MediaCoverArt::FileHash)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_media_cover_art_file_hash")
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden, Clone, Copy)]
pub enum MediaCoverArt {
    Table,
    FileHash,
}
