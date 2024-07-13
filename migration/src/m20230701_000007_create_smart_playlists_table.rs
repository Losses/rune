use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20230701_000007_create_smart_playlists_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(SmartPlaylists::Table)
                    .col(
                        ColumnDef::new(SmartPlaylists::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(SmartPlaylists::Name).string().not_null())
                    .col(ColumnDef::new(SmartPlaylists::Query).text().not_null())
                    .col(ColumnDef::new(SmartPlaylists::CreatedAt).timestamp().not_null())
                    .col(ColumnDef::new(SmartPlaylists::UpdatedAt).timestamp().not_null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(SmartPlaylists::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
pub enum SmartPlaylists {
    Table,
    Id,
    Name,
    Query,
    CreatedAt,
    UpdatedAt,
}
