use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const INDEXES: &[(&str, &str, &[&str])] = &[
    (
        "idx_media_files_path",
        "media_files",
        &["directory", "file_name"],
    ),
    ("idx_media_metadata_file_id", "media_metadata", &["file_id"]),
    ("idx_media_analysis_file_id", "media_analysis", &["file_id"]),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for &(name, table, columns) in INDEXES {
            let mut index = Index::create();
            index.name(name).table(Alias::new(table)).if_not_exists();
            for column in columns {
                index.col(Alias::new(*column));
            }
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for &(name, _, _) in INDEXES.iter().rev() {
            manager
                .drop_index(Index::drop().name(name).to_owned())
                .await?;
        }
        Ok(())
    }
}
