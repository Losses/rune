use sea_orm_migration::prelude::*;

use super::m20230701_000001_create_media_files_table::MediaFiles;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20250312_000024_create_media_file_similarity_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MediaFileSimilarity::Table)
                    .col(
                        ColumnDef::new(MediaFileSimilarity::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(MediaFileSimilarity::FileId1)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MediaFileSimilarity::FileId2)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(MediaFileSimilarity::Similarity)
                            .float()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_media_file_similarity_file_id1")
                            .from(MediaFileSimilarity::Table, MediaFileSimilarity::FileId1)
                            .to(MediaFiles::Table, MediaFiles::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_media_file_similarity_file_id2")
                            .from(MediaFileSimilarity::Table, MediaFileSimilarity::FileId2)
                            .to(MediaFiles::Table, MediaFiles::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .index(
                        Index::create()
                            .name("idx_file_ids")
                            .col(MediaFileSimilarity::FileId1)
                            .col(MediaFileSimilarity::FileId2)
                            .unique(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MediaFileSimilarity::Table).to_owned())
            .await
    }
}

#[derive(Iden, Clone, Copy)]
pub enum MediaFileSimilarity {
    Table,
    Id,
    FileId1,
    FileId2,
    Similarity,
}
