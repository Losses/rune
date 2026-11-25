use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20231117_000020_create_log_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Log::Table)
                    .col(
                        ColumnDef::new(Log::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Log::Date).timestamp().not_null())
                    .col(ColumnDef::new(Log::Level).string().not_null())
                    .col(ColumnDef::new(Log::Domain).string().not_null())
                    .col(ColumnDef::new(Log::Detail).string().not_null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Log::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
pub enum Log {
    Table,
    Id,
    Date,
    Level,
    Domain,
    Detail,
}
