use sea_orm_migration::prelude::*;
use sync_orm::causal_schema;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager.has_table("sync_meta").await?
            && !manager.has_table("sync_export_cursors").await?
        {
            manager
                .create_table(causal_schema::export_cursors_table())
                .await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Previous releases could create this durable cursor table while opening
        // a replica. Keep that state intact when rolling back the migration.
        Ok(())
    }
}
