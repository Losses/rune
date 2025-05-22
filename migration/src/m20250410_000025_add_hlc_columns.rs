use sea_orm_migration::prelude::*;

use crate::{
    m20230701_000001_create_media_files_table::MediaFiles,
    m20230701_000002_create_media_metadata_table::MediaMetadata,
    m20230701_000003_create_media_analysis_table::MediaAnalysis,
    m20230701_000005_create_playlists_table::Playlists,
    m20230701_000006_create_media_file_playlists::MediaFilePlaylists,
    m20230728_000008_create_media_cover_art_table::MediaCoverArt,
    m20230806_000009_create_artists_table::Artists, m20230806_000011_create_albums_table::Albums,
    m20230912_000013_create_mixes_table::Mixes,
    m20230912_000014_create_mix_queries_table::MixQueries,
    m20250311_000021_create_genres_table::Genres,
};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(Iden)]
enum CommonColumns {
    CreatedAt,
    UpdatedAt,
    DataVersion,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        println!("Applying migration: Add tracking columns");

        Self::add_all_tracking_columns(manager, Albums::Table).await?;
        Self::add_all_tracking_columns(manager, Artists::Table).await?;
        Self::add_all_tracking_columns(manager, Genres::Table).await?;
        Self::add_all_tracking_columns(manager, MediaAnalysis::Table).await?;
        Self::add_all_tracking_columns(manager, MediaCoverArt::Table).await?;
        Self::add_all_tracking_columns(manager, MediaFiles::Table).await?;
        Self::add_all_tracking_columns(manager, MediaMetadata::Table).await?;
        Self::add_all_tracking_columns(manager, MediaFilePlaylists::Table).await?;

        Self::add_data_version_column(manager, Mixes::Table).await?;
        Self::add_data_version_column(manager, MixQueries::Table).await?;
        Self::add_data_version_column(manager, Playlists::Table).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        Self::remove_all_tracking_columns(manager, Albums::Table).await?;
        Self::remove_all_tracking_columns(manager, Artists::Table).await?;
        Self::remove_all_tracking_columns(manager, Genres::Table).await?;
        Self::remove_all_tracking_columns(manager, MediaAnalysis::Table).await?;
        Self::remove_all_tracking_columns(manager, MediaCoverArt::Table).await?;
        Self::remove_all_tracking_columns(manager, MediaFiles::Table).await?;
        Self::remove_all_tracking_columns(manager, MediaMetadata::Table).await?;
        Self::remove_all_tracking_columns(manager, MediaFilePlaylists::Table).await?;

        Self::remove_data_version_column(manager, Mixes::Table).await?;
        Self::remove_data_version_column(manager, MixQueries::Table).await?;
        Self::remove_data_version_column(manager, Playlists::Table).await?;

        Ok(())
    }
}

impl Migration {
    async fn add_all_tracking_columns<'a, T>(
        manager: &'a SchemaManager<'a>,
        table: T,
    ) -> Result<(), DbErr>
    where
        T: Iden + Copy + 'static,
    {
        // A common placeholder timestamp string.
        // SQLite will store this as TEXT.
        // Ensure your application logic correctly sets these timestamps on new/updated records.
        let default_timestamp_value =
            Value::String(Some(Box::new("1970-01-01 00:00:00.000".to_string())));

        // Add CreatedAt
        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .add_column(
                        ColumnDef::new(CommonColumns::CreatedAt)
                            .timestamp()
                            .not_null()
                            // Use a constant default value for existing rows
                            .default(default_timestamp_value.clone()),
                    )
                    .to_owned(),
            )
            .await?;

        // Add UpdatedAt
        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .add_column(
                        ColumnDef::new(CommonColumns::UpdatedAt)
                            .timestamp()
                            .not_null()
                            // Use a constant default value for existing rows
                            .default(default_timestamp_value),
                    )
                    .to_owned(),
            )
            .await?;

        // Add DataVersion
        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .add_column(
                        ColumnDef::new(CommonColumns::DataVersion)
                            .integer()
                            .not_null()
                            .default(0), // 0 is a constant, so this is fine
                    )
                    .to_owned(),
            )
            .await
    }

    async fn add_data_version_column<'a, T>(
        manager: &'a SchemaManager<'a>,
        table: T,
    ) -> Result<(), DbErr>
    where
        T: Iden + 'static,
    {
        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .add_column(
                        ColumnDef::new(CommonColumns::DataVersion)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn remove_all_tracking_columns<'a, T>(
        manager: &'a SchemaManager<'a>,
        table: T,
    ) -> Result<(), DbErr>
    where
        T: Iden + Copy + 'static,
    {
        // Remove CreatedAt
        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .drop_column(CommonColumns::CreatedAt)
                    .to_owned(),
            )
            .await?;

        // Remove UpdatedAt
        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .drop_column(CommonColumns::UpdatedAt)
                    .to_owned(),
            )
            .await?;

        // Remove DataVersion
        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .drop_column(CommonColumns::DataVersion)
                    .to_owned(),
            )
            .await
    }

    async fn remove_data_version_column<'a, T>(
        manager: &'a SchemaManager<'a>,
        table: T,
    ) -> Result<(), DbErr>
    where
        T: Iden + 'static,
    {
        println!(
            "     -> Removing data_version column from: {}",
            table.to_string()
        );
        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .drop_column(CommonColumns::DataVersion)
                    .to_owned(),
            )
            .await
    }
}
