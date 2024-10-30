use async_trait::async_trait;
use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20231029_000017_create_search_index"
    }
}

#[async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // ID is only for making sea-orm happy
        db.execute_unprepared(
            "CREATE VIRTUAL TABLE search_index USING fts5(id, key, entry_type, doc);"
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE `search_index`")
            .await?;

        Ok(())
    }
}
