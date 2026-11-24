use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Default must remain 1: existing rows and writes from old clients are legacy.
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("media_analysis"))
                    .add_column(
                        ColumnDef::new(Alias::new("analysis_version"))
                            .integer()
                            .not_null()
                            .default(1),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_media_analysis_version_file")
                    .table(Alias::new("media_analysis"))
                    .col(Alias::new("analysis_version"))
                    .col(Alias::new("file_id"))
                    .to_owned(),
            )
            .await
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_media_analysis_version_file")
                    .table(Alias::new("media_analysis"))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("media_analysis"))
                    .drop_column(Alias::new("analysis_version"))
                    .to_owned(),
            )
            .await
    }
}
