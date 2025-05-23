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
    HlcUuid,
    CreatedAtHlcTs,
    CreatedAtHlcVer,
    CreatedAtHlcNid,
    UpdatedAtHlcTs,
    UpdatedAtHlcVer,
    UpdatedAtHlcNid,
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
        Self::add_all_tracking_columns(manager, Mixes::Table).await?;
        Self::add_all_tracking_columns(manager, MixQueries::Table).await?;
        Self::add_all_tracking_columns(manager, Playlists::Table).await?;
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
        Self::remove_all_tracking_columns(manager, Mixes::Table).await?;
        Self::remove_all_tracking_columns(manager, MixQueries::Table).await?;
        Self::remove_all_tracking_columns(manager, Playlists::Table).await?;

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

        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .add_column(
                        ColumnDef::new(CommonColumns::HlcUuid)
                            .string()
                            .not_null()
                            .default(""),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .add_column(
                        ColumnDef::new(CommonColumns::UpdatedAtHlcTs)
                            .timestamp()
                            .not_null()
                            .default(default_timestamp_value.clone()),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .add_column(
                        ColumnDef::new(CommonColumns::UpdatedAtHlcVer)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .add_column(
                        ColumnDef::new(CommonColumns::UpdatedAtHlcNid)
                            .text()
                            .not_null()
                            .default(""),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .add_column(
                        ColumnDef::new(CommonColumns::CreatedAtHlcTs)
                            .timestamp()
                            .not_null()
                            .default(default_timestamp_value),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .add_column(
                        ColumnDef::new(CommonColumns::CreatedAtHlcVer)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .add_column(
                        ColumnDef::new(CommonColumns::CreatedAtHlcNid)
                            .text()
                            .not_null()
                            .default(""),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn remove_all_tracking_columns<'a, T>(
        manager: &'a SchemaManager<'a>,
        table: T,
    ) -> Result<(), DbErr>
    where
        T: Iden + Copy + 'static,
    {
        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .drop_column(CommonColumns::HlcUuid)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .drop_column(CommonColumns::CreatedAtHlcTs)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .drop_column(CommonColumns::CreatedAtHlcVer)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .drop_column(CommonColumns::CreatedAtHlcNid)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .drop_column(CommonColumns::UpdatedAtHlcTs)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .drop_column(CommonColumns::UpdatedAtHlcVer)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .drop_column(CommonColumns::UpdatedAtHlcNid)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
