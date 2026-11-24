use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20230728_000008_create_media_cover_art_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MediaCoverArt::Table)
                    .col(
                        ColumnDef::new(MediaCoverArt::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(MediaCoverArt::FileHash)
                            .char_len(64)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MediaCoverArt::Binary)
                            .var_binary(16777216)
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MediaCoverArt::Table).to_owned())
            .await
    }
}

#[derive(Iden, Clone, Copy)]
pub enum MediaCoverArt {
    Table,
    Id,
    FileHash,
    Binary,
    PrimaryColor,
}
