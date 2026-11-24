use sea_orm_migration::prelude::*;

use crate::m20230912_000013_create_mixes_table::Mixes;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20230912_000014_create_mix_queries_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MixQueries::Table)
                    .col(
                        ColumnDef::new(MixQueries::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(MixQueries::MixId).integer().not_null())
                    .col(ColumnDef::new(MixQueries::Operator).text().not_null())
                    .col(ColumnDef::new(MixQueries::Parameter).text().not_null())
                    .col(ColumnDef::new(MixQueries::Group).integer().not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_mix_queries_mix_id")
                            .from(MixQueries::Table, MixQueries::MixId)
                            .to(Mixes::Table, Mixes::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .col(ColumnDef::new(MixQueries::CreatedAt).timestamp().not_null())
                    .col(ColumnDef::new(MixQueries::UpdatedAt).timestamp().not_null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MixQueries::Table).to_owned())
            .await
    }
}

#[derive(Iden, Clone, Copy)]
pub enum MixQueries {
    Table,
    Id,
    MixId,
    Operator,
    Parameter,
    Group,
    CreatedAt,
    UpdatedAt,
}
