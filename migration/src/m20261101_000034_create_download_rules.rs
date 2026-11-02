use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("sync_download_rules"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Alias::new("queries")).text().not_null())
                    .col(ColumnDef::new(Alias::new("mix_id")).text())
                    .col(ColumnDef::new(Alias::new("enabled")).boolean().not_null())
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("sync_download_jobs"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Alias::new("file_uuid")).text().not_null())
                    .col(ColumnDef::new(Alias::new("digest")).text().not_null())
                    .col(ColumnDef::new(Alias::new("size")).big_integer().not_null())
                    .col(ColumnDef::new(Alias::new("directory")).text().not_null())
                    .col(ColumnDef::new(Alias::new("file_name")).text().not_null())
                    .col(ColumnDef::new(Alias::new("state")).text().not_null())
                    .col(
                        ColumnDef::new(Alias::new("attempts"))
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(ColumnDef::new(Alias::new("retry_at")).text())
                    .col(ColumnDef::new(Alias::new("retry_source")).text())
                    .col(ColumnDef::new(Alias::new("last_error")).text())
                    .index(
                        Index::create()
                            .col(Alias::new("file_uuid"))
                            .col(Alias::new("digest"))
                            .unique(),
                    )
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for name in ["sync_download_jobs", "sync_download_rules"] {
            manager
                .drop_table(Table::drop().table(Alias::new(name)).to_owned())
                .await?;
        }
        Ok(())
    }
}
