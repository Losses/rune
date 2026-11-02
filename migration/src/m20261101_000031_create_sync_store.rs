use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        sync_orm::causal_schema::install_on(manager.get_connection()).await
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        sync_orm::causal_schema::remove_on(manager.get_connection()).await
    }
}
