use sea_orm_migration::{
    prelude::*,
    sea_orm::{prelude::Uuid, FromQueryResult, Statement},
};

use crate::{
    m20230701_000001_create_media_files_table::MediaFiles,
    m20230701_000002_create_media_metadata_table::MediaMetadata,
    m20230701_000003_create_media_analysis_table::MediaAnalysis,
    m20230701_000005_create_playlists_table::Playlists,
    m20230701_000006_create_media_file_playlists::MediaFilePlaylists,
    m20230728_000008_create_media_cover_art_table::MediaCoverArt,
    m20230806_000009_create_artists_table::Artists,
    m20230806_000010_create_media_file_artists_table::MediaFileArtists,
    m20230806_000011_create_albums_table::Albums,
    m20230806_000012_create_media_file_albums_table::MediaFileAlbums,
    m20230912_000013_create_mixes_table::Mixes,
    m20230912_000014_create_mix_queries_table::MixQueries,
    m20250311_000021_create_genres_table::Genres,
    m20250311_000022_create_media_file_genres_table::MediaFileGenres,
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
        Self::add_tracking_columns(manager, Albums::Table, true).await?;
        Self::add_tracking_columns(manager, Artists::Table, true).await?;
        Self::add_tracking_columns(manager, Genres::Table, true).await?;
        Self::add_tracking_columns(manager, MediaAnalysis::Table, true).await?;
        Self::add_tracking_columns(manager, MediaCoverArt::Table, true).await?;
        Self::add_tracking_columns(manager, MediaFiles::Table, true).await?;
        Self::add_tracking_columns(manager, MediaMetadata::Table, true).await?;
        Self::add_tracking_columns(manager, MediaFilePlaylists::Table, true).await?;
        Self::add_tracking_columns(manager, MediaFileAlbums::Table, true).await?;
        Self::add_tracking_columns(manager, MediaFileArtists::Table, true).await?;
        Self::add_tracking_columns(manager, MediaFileGenres::Table, true).await?;

        Self::add_tracking_columns_with_existing_ts(manager, Mixes::Table).await?;
        Self::add_tracking_columns_with_existing_ts(manager, MixQueries::Table).await?;
        Self::add_tracking_columns_with_existing_ts(manager, Playlists::Table).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        Self::remove_tracking_columns(manager, Albums::Table, true).await?;
        Self::remove_tracking_columns(manager, Artists::Table, true).await?;
        Self::remove_tracking_columns(manager, Genres::Table, true).await?;
        Self::remove_tracking_columns(manager, MediaAnalysis::Table, true).await?;
        Self::remove_tracking_columns(manager, MediaCoverArt::Table, true).await?;
        Self::remove_tracking_columns(manager, MediaFiles::Table, true).await?;
        Self::remove_tracking_columns(manager, MediaMetadata::Table, true).await?;
        Self::remove_tracking_columns(manager, MediaFilePlaylists::Table, true).await?;
        Self::remove_tracking_columns(manager, MediaFileAlbums::Table, true).await?;
        Self::remove_tracking_columns(manager, MediaFileArtists::Table, true).await?;
        Self::remove_tracking_columns(manager, MediaFileGenres::Table, true).await?;

        Self::remove_tracking_columns_with_existing_ts(manager, Mixes::Table).await?;
        Self::remove_tracking_columns_with_existing_ts(manager, MixQueries::Table).await?;
        Self::remove_tracking_columns_with_existing_ts(manager, Playlists::Table).await?;

        Ok(())
    }
}

impl Migration {
    async fn add_tracking_columns<'a, T>(
        manager: &'a SchemaManager<'a>,
        table: T,
        include_ts: bool,
    ) -> Result<(), DbErr>
    where
        T: Iden + Copy + 'static,
    {
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
            .create_index(
                Index::create()
                    .name(format!("idx_{}_hlc_uuid_unique", table.to_string()))
                    .table(table)
                    .col(CommonColumns::HlcUuid)
                    .unique()
                    .to_owned(),
            )
            .await?;

        if include_ts {
            manager
                .alter_table(
                    Table::alter()
                        .table(table)
                        .add_column(
                            ColumnDef::new(CommonColumns::CreatedAtHlcTs)
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
                            ColumnDef::new(CommonColumns::UpdatedAtHlcTs)
                                .timestamp()
                                .not_null()
                                .default(default_timestamp_value),
                        )
                        .to_owned(),
                )
                .await?;
        }

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

        Ok(())
    }

    async fn add_tracking_columns_with_existing_ts<'a, T>(
        manager: &'a SchemaManager<'a>,
        table: T,
    ) -> Result<(), DbErr>
    where
        T: Iden + Copy + 'static,
    {
        // STEP 1: Add new columns one by one for SQLite compatibility.
        // SQLite's ALTER TABLE command only supports one action at a time.
        // Therefore, we must issue a separate command for each column addition.
        let default_timestamp_value =
            Value::String(Some(Box::new("1970-01-01T00:00:00Z".to_string())));

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
                        ColumnDef::new(CommonColumns::CreatedAtHlcTs)
                            .timestamp_with_time_zone()
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
                        ColumnDef::new(CommonColumns::UpdatedAtHlcTs)
                            .timestamp_with_time_zone()
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

        // STEP 2: Populate the new timestamp columns from the old ones.
        #[derive(Iden)]
        enum OldTimestampColumns {
            CreatedAt,
            UpdatedAt,
        }

        manager
            .exec_stmt(
                Query::update()
                    .table(table)
                    .value(
                        CommonColumns::CreatedAtHlcTs,
                        Expr::col(OldTimestampColumns::CreatedAt),
                    )
                    .value(
                        CommonColumns::UpdatedAtHlcTs,
                        Expr::col(OldTimestampColumns::UpdatedAt),
                    )
                    .to_owned(),
            )
            .await?;

        // STEP 3: Populate the 'hlc_uuid' column with unique values.
        let db = manager.get_connection();

        #[derive(Iden)]
        enum PrimaryKey {
            Id,
        }
        #[derive(Debug, FromQueryResult)]
        struct RowId {
            id: i32,
        }

        let rows_to_update: Vec<RowId> = RowId::find_by_statement(Statement::from_string(
            db.get_database_backend(),
            format!("SELECT id FROM {}", table.to_string()),
        ))
        .all(db)
        .await?;

        for row in rows_to_update {
            let new_uuid = Uuid::new_v4().to_string();
            manager
                .exec_stmt(
                    Query::update()
                        .table(table)
                        .value(CommonColumns::HlcUuid, new_uuid)
                        .and_where(Expr::col(PrimaryKey::Id).eq(row.id))
                        .to_owned(),
                )
                .await?;
        }

        // STEP 4: Create the unique index on 'hlc_uuid'.
        manager
            .create_index(
                Index::create()
                    .name(format!("idx_{}_hlc_uuid_unique", table.to_string()))
                    .table(table)
                    .col(CommonColumns::HlcUuid)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // STEP 5: Drop the old columns one by one for SQLite compatibility.
        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .drop_column(OldTimestampColumns::CreatedAt)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .drop_column(OldTimestampColumns::UpdatedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn remove_tracking_columns<'a, T>(
        manager: &'a SchemaManager<'a>,
        table: T,
        include_ts: bool,
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

        if include_ts {
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
                        .drop_column(CommonColumns::UpdatedAtHlcTs)
                        .to_owned(),
                )
                .await?;
        }

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

    async fn remove_tracking_columns_with_existing_ts<'a, T>(
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
                    .drop_column(CommonColumns::UpdatedAtHlcTs)
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

        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .add_column(ColumnDef::new(Mixes::CreatedAt).timestamp().not_null())
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .add_column(ColumnDef::new(Mixes::UpdatedAt).timestamp().not_null())
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
