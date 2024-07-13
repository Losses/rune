use sea_orm_migration::prelude::*;

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
                    .col(ColumnDef::new(MediaFiles::LastModified).timestamp().not_null())
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

#[derive(Iden)]
pub enum MediaFiles {
    Table,
    Id,
    FileName,
    Directory,
    Extension,
    FileHash,
    LastModified,
}
