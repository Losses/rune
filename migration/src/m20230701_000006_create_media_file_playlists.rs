use sea_orm_migration::prelude::*;

use super::m20230701_000001_create_media_files_table::MediaFiles;
use super::m20230701_000005_create_playlists_table::Playlists;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20230701_000006_create_media_file_playlists"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MediaFilePlaylists::Table)
                    .col(
                        ColumnDef::new(MediaFilePlaylists::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(MediaFilePlaylists::PlaylistId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MediaFilePlaylists::MediaFileId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MediaFilePlaylists::Position)
                            .integer()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-media_file_playlists-playlist_id")
                            .from(MediaFilePlaylists::Table, MediaFilePlaylists::PlaylistId)
                            .to(Playlists::Table, Playlists::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-media_file_playlists-file_id")
                            .from(MediaFilePlaylists::Table, MediaFilePlaylists::MediaFileId)
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
            .drop_table(Table::drop().table(MediaFilePlaylists::Table).to_owned())
            .await
    }
}

#[derive(Iden, Clone, Copy)]
pub enum MediaFilePlaylists {
    Table,
    Id,
    PlaylistId,
    MediaFileId,
    Position,
}
