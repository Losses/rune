use sea_orm_migration::prelude::*;

use super::m20230701_000001_create_media_files_table::MediaFiles;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20250312_000023_create_media_file_fingerprint_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MediaFileFingerprint::Table)
                    .col(
                        ColumnDef::new(MediaFileFingerprint::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(MediaFileFingerprint::MediaFileId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MediaFileFingerprint::Fingerprint)
                            .var_binary(16777216)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MediaFileFingerprint::IsDuplicated)
                            .integer()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_media_file_fingerprint_media_file_id")
                            .from(
                                MediaFileFingerprint::Table,
                                MediaFileFingerprint::MediaFileId,
                            )
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
            .drop_table(Table::drop().table(MediaFileFingerprint::Table).to_owned())
            .await
    }
}

#[derive(Iden, Clone, Copy)]
pub enum MediaFileFingerprint {
    Table,
    Id,
    MediaFileId,
    Fingerprint,
    IsDuplicated,
}
