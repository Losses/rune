use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20230728_000008_create_media_cover_arts_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MediaCoverArts::Table)
                    .col(
                        ColumnDef::new(MediaCoverArts::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(MediaCoverArts::FileHash).char_len(64).not_null())
                    .col(ColumnDef::new(MediaCoverArts::Binary).var_binary(16777216).not_null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MediaCoverArts::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
pub enum MediaCoverArts {
    Table,
    Id,
    FileHash,
    Binary,
}
