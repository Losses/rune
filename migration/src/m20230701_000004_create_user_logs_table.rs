use sea_orm_migration::prelude::*;

use super::m20230701_000001_create_media_files_table::MediaFiles;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20230701_000004_create_user_logs_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(UserLogs::Table)
                    .col(
                        ColumnDef::new(UserLogs::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(UserLogs::FileId).integer().not_null())
                    .col(ColumnDef::new(UserLogs::ListenTime).timestamp().not_null())
                    .col(ColumnDef::new(UserLogs::Progress).double().not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-user_logs-file_id")
                            .from(UserLogs::Table, UserLogs::FileId)
                            .to(MediaFiles::Table, MediaFiles::Id),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(UserLogs::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
pub enum UserLogs {
    Table,
    Id,
    FileId,
    ListenTime,
    Progress,
}
