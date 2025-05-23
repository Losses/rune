use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20230701_000005_create_playlists_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Playlists::Table)
                    .col(
                        ColumnDef::new(Playlists::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Playlists::Name).string().not_null())
                    .col(ColumnDef::new(Playlists::Group).string().not_null())
                    .col(ColumnDef::new(Playlists::CreatedAt).timestamp().not_null())
                    .col(ColumnDef::new(Playlists::UpdatedAt).timestamp().not_null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Playlists::Table).to_owned())
            .await
    }
}

#[derive(Iden, Clone, Copy)]
pub enum Playlists {
    Table,
    Id,
    Name,
    Group,
    CreatedAt,
    UpdatedAt,
}
