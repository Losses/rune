use sea_orm_migration::prelude::*;

use super::m20230701_000001_create_media_files_table::MediaFiles;
use super::m20230806_000009_create_artists_table::Artists;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20230806_000010_create_media_file_artists_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MediaFileArtists::Table)
                    .col(
                        ColumnDef::new(MediaFileArtists::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(MediaFileArtists::MediaFileId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MediaFileArtists::ArtistId)
                            .integer()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_media_file_artists_media_file_id")
                            .from(MediaFileArtists::Table, MediaFileArtists::MediaFileId)
                            .to(MediaFiles::Table, MediaFiles::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_media_file_artists_artist_id")
                            .from(MediaFileArtists::Table, MediaFileArtists::ArtistId)
                            .to(Artists::Table, Artists::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MediaFileArtists::Table).to_owned())
            .await
    }
}

#[derive(Iden, Clone, Copy)]
pub enum MediaFileArtists {
    Table,
    Id,
    MediaFileId,
    ArtistId,
}
