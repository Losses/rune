use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20250529_000026_create_sync_record_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(SyncRecord::Table)
                    .col(
                        ColumnDef::new(SyncRecord::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(SyncRecord::TableName).string().not_null())
                    .col(ColumnDef::new(SyncRecord::ClientNodeId).string().not_null())
                    .col(
                        ColumnDef::new(SyncRecord::LastSyncHlcTs)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SyncRecord::LastSyncHlcVer)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SyncRecord::LastSyncHlcNid)
                            .string()
                            .not_null(),
                    )
                    .index(
                        Index::create()
                            .name("idx_sync_record_table_client_unique")
                            .col(SyncRecord::TableName)
                            .col(SyncRecord::ClientNodeId)
                            .unique(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(SyncRecord::Table).to_owned())
            .await
    }
}

#[derive(Iden, Clone, Copy)]
pub enum SyncRecord {
    Table,
    Id,
    TableName,
    ClientNodeId,
    LastSyncHlcTs,
    LastSyncHlcVer,
    LastSyncHlcNid,
}
