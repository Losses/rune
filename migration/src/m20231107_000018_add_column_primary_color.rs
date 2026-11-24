use sea_orm_migration::prelude::*;

use crate::m20230728_000008_create_media_cover_art_table::MediaCoverArt;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20231107_000018_add_column_primary_color"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(MediaCoverArt::Table)
                    .add_column(ColumnDef::new(MediaCoverArt::PrimaryColor).integer().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(MediaCoverArt::Table)
                    .drop_column(MediaCoverArt::PrimaryColor)
                    .to_owned(),
            )
            .await
    }
}
