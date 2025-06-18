use sea_orm_migration::prelude::*;

use super::m20230701_000001_create_media_files_table::MediaFiles;
use super::m20250311_000021_create_genres_table::Genres;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20250311_000021_create_media_file_genres_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MediaFileGenres::Table)
                    .col(
                        ColumnDef::new(MediaFileGenres::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(MediaFileGenres::MediaFileId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MediaFileGenres::GenreId)
                            .integer()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_media_file_genres_media_file_id")
                            .from(MediaFileGenres::Table, MediaFileGenres::MediaFileId)
                            .to(MediaFiles::Table, MediaFiles::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_media_file_genres_artist_id")
                            .from(MediaFileGenres::Table, MediaFileGenres::GenreId)
                            .to(Genres::Table, Genres::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MediaFileGenres::Table).to_owned())
            .await
    }
}

#[derive(Iden, Clone, Copy)]
pub enum MediaFileGenres {
    Table,
    Id,
    MediaFileId,
    GenreId,
}
