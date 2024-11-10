use sea_orm_migration::prelude::*;

use crate::m20230701_000001_create_media_files_table::MediaFiles;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20231110_000019_create_playback_queue_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(PlaybackQueue::Table)
                    .col(
                        ColumnDef::new(PlaybackQueue::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PlaybackQueue::MediaFileId)
                            .integer()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-playback_queue-file_id")
                            .from(PlaybackQueue::Table, PlaybackQueue::MediaFileId)
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
            .drop_table(Table::drop().table(PlaybackQueue::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
pub enum PlaybackQueue {
    Table,
    Id,
    MediaFileId,
}
