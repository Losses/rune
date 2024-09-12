use sea_orm_migration::prelude::*;

use super::m20230701_000001_create_media_files_table::MediaFiles;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20230912_000015_create_media_file_stats_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MediaFileStats::Table)
                    .col(
                        ColumnDef::new(MediaFileStats::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(MediaFileStats::MediaFileId)
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(MediaFileStats::Liked).boolean().not_null())
                    .col(ColumnDef::new(MediaFileStats::Skipped).integer().not_null())
                    .col(
                        ColumnDef::new(MediaFileStats::PlayedThrough)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MediaFileStats::UpdatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-media_file_stats-file_id")
                            .from(MediaFileStats::Table, MediaFileStats::MediaFileId)
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
            .drop_table(Table::drop().table(MediaFileStats::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
pub enum MediaFileStats {
    Table,
    Id,
    MediaFileId,
    Liked,
    Skipped,
    PlayedThrough,
    UpdatedAt,
}
