use sea_orm_migration::prelude::*;

use super::m20230701_000001_create_media_files_table::MediaFiles;
use super::m20230806_000011_create_albums_table::Albums;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20230806_000012_create_media_file_albums_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MediaFileAlbums::Table)
                    .col(
                        ColumnDef::new(MediaFileAlbums::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(MediaFileAlbums::MediaFileId)
                            .integer()
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(MediaFileAlbums::TrackNumber)
                            .integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(MediaFileAlbums::AlbumId)
                            .integer()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_media_file_albums_media_file_id")
                            .from(MediaFileAlbums::Table, MediaFileAlbums::MediaFileId)
                            .to(MediaFiles::Table, MediaFiles::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_media_file_albums_album_id")
                            .from(MediaFileAlbums::Table, MediaFileAlbums::AlbumId)
                            .to(Albums::Table, Albums::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MediaFileAlbums::Table).to_owned())
            .await
    }
}

#[derive(Iden, Clone, Copy)]
pub enum MediaFileAlbums {
    Table,
    Id,
    MediaFileId,
    AlbumId,
    TrackNumber,
}
