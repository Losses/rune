use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20230912_000013_create_mixes_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Mixes::Table)
                    .col(
                        ColumnDef::new(Mixes::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Mixes::Name).string().not_null())
                    .col(ColumnDef::new(Mixes::Group).string().not_null())
                    .col(ColumnDef::new(Mixes::Mode).integer().null())
                    .col(ColumnDef::new(Mixes::Locked).boolean().not_null())
                    .col(ColumnDef::new(Mixes::ScriptletMode).boolean().not_null())
                    .col(ColumnDef::new(Mixes::CreatedAt).timestamp().not_null())
                    .col(ColumnDef::new(Mixes::UpdatedAt).timestamp().not_null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Mixes::Table).to_owned())
            .await
    }
}

#[derive(Iden, Clone, Copy)]
pub enum Mixes {
    Table,
    Id,
    Name,
    Group,
    Mode,
    Locked,
    ScriptletMode,
    CreatedAt,
    UpdatedAt,
}
