use sea_orm_migration::prelude::*;

use super::m20230701_000001_create_media_files_table::MediaFiles;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20230701_000002_create_media_metadata_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MediaMetadata::Table)
                    .col(
                        ColumnDef::new(MediaMetadata::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(MediaMetadata::FileId).integer().not_null())
                    .col(ColumnDef::new(MediaMetadata::MetaKey).string().not_null())
                    .col(ColumnDef::new(MediaMetadata::MetaValue).text().not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-media_metadata-file_id")
                            .from(MediaMetadata::Table, MediaMetadata::FileId)
                            .to(MediaFiles::Table, MediaFiles::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MediaMetadata::Table).to_owned())
            .await
    }
}

#[derive(Iden, Clone, Copy)]
pub enum MediaMetadata {
    Table,
    Id,
    FileId,
    MetaKey,
    MetaValue,
}
