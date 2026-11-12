use sea_orm_migration::prelude::*;

const SOURCE_TABLES: &[&str] = &[
    "albums",
    "artists",
    "genres",
    "media_analysis",
    "media_cover_art",
    "media_file_albums",
    "media_file_artists",
    "media_file_fingerprint",
    "media_file_genres",
    "media_file_likes",
    "media_file_playlists",
    "media_file_similarity",
    "media_file_stats",
    "media_files",
    "media_metadata",
    "mix_queries",
    "mixes",
    "playlists",
];

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        rename_source_columns(manager, "hlc_uuid", "source_uuid").await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        rename_source_columns(manager, "source_uuid", "hlc_uuid").await
    }
}

async fn rename_source_columns(
    manager: &SchemaManager<'_>,
    old_column: &str,
    new_column: &str,
) -> Result<(), DbErr> {
    for table in SOURCE_TABLES {
        let old_index = format!("idx_{table}_{old_column}");
        let new_index = format!("idx_{table}_{new_column}");
        let had_source_index = manager.has_index(table, &old_index).await?;
        if had_source_index {
            manager
                .drop_index(
                    Index::drop()
                        .name(&old_index)
                        .table(Alias::new(*table))
                        .to_owned(),
                )
                .await?;
        }

        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new(*table))
                    .rename_column(Alias::new(old_column), Alias::new(new_column))
                    .to_owned(),
            )
            .await?;

        if had_source_index {
            if manager.has_index(table, &new_index).await? {
                manager
                    .drop_index(
                        Index::drop()
                            .name(&new_index)
                            .table(Alias::new(*table))
                            .to_owned(),
                    )
                    .await?;
            }
            manager
                .create_index(
                    Index::create()
                        .name(&new_index)
                        .table(Alias::new(*table))
                        .col(Alias::new(new_column))
                        .to_owned(),
                )
                .await?;
        }
    }
    Ok(())
}
