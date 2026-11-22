use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("media_analysis"))
                    .add_column(ColumnDef::new(Alias::new("source_hash")).text().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("media_analysis"))
                    .add_column(
                        ColumnDef::new(Alias::new("source_size"))
                            .big_integer()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Legacy analyses never recorded the bytes they were computed from,
        // and the old code did not invalidate them when a rescan changed a
        // file. A matching source UUID only proves that the file hash (a CRC)
        // matched at some point; the file hash is not updated when metadata
        // parsing fails, so it cannot prove that a row describes the current
        // bytes. Every legacy row is therefore untrusted: drop them all and
        // let the next analysis run recompute them with a recorded source
        // identity.
        manager
            .get_connection()
            .execute_unprepared("DELETE FROM media_analysis")
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("media_analysis"))
                    .drop_column(Alias::new("source_size"))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("media_analysis"))
                    .drop_column(Alias::new("source_hash"))
                    .to_owned(),
            )
            .await
    }
}
