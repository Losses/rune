use sea_orm_migration::prelude::*;

use super::m20230701_000001_create_media_files_table::MediaFiles;
use super::m20230701_000005_create_playlists_table::Playlists;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20230701_000006_create_playlist_items_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(PlaylistItems::Table)
                    .col(
                        ColumnDef::new(PlaylistItems::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(PlaylistItems::PlaylistId).integer().not_null())
                    .col(ColumnDef::new(PlaylistItems::FileId).integer().not_null())
                    .col(ColumnDef::new(PlaylistItems::Position).integer().not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-playlist_items-playlist_id")
                            .from(PlaylistItems::Table, PlaylistItems::PlaylistId)
                            .to(Playlists::Table, Playlists::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-playlist_items-file_id")
                            .from(PlaylistItems::Table, PlaylistItems::FileId)
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
            .drop_table(Table::drop().table(PlaylistItems::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
pub enum PlaylistItems {
    Table,
    Id,
    PlaylistId,
    FileId,
    Position,
}
