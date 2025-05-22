use sea_orm_migration::prelude::*;

use super::m20230728_000008_create_media_cover_art_table::MediaCoverArt;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20230701_000001_create_media_files_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MediaFiles::Table)
                    .col(
                        ColumnDef::new(MediaFiles::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(MediaFiles::FileName).string().not_null())
                    .col(ColumnDef::new(MediaFiles::Directory).string().not_null())
                    .col(ColumnDef::new(MediaFiles::Extension).string().not_null())
                    .col(ColumnDef::new(MediaFiles::FileHash).char_len(64).not_null())
                    .col(
                        ColumnDef::new(MediaFiles::LastModified)
                            .timestamp()
                            .not_null(),
                    )
                    .col(ColumnDef::new(MediaFiles::CoverArtId).integer().null())
                    .col(ColumnDef::new(MediaFiles::SampleRate).integer().not_null())
                    .col(ColumnDef::new(MediaFiles::Duration).double().not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_media_files_cover_art_id")
                            .from(MediaFiles::Table, MediaFiles::CoverArtId)
                            .to(MediaCoverArt::Table, MediaCoverArt::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MediaFiles::Table).to_owned())
            .await
    }
}

#[derive(Iden, Clone, Copy)]
pub enum MediaFiles {
    Table,
    Id,
    FileName,
    Directory,
    Extension,
    FileHash,
    LastModified,
    CoverArtId,
    SampleRate,
    Duration,
}
