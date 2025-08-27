use sea_orm_migration::{
    prelude::*,
    sea_orm::{FromQueryResult, Statement, prelude::Uuid},
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
    m20250312_000023_create_media_file_fingerprint_table::MediaFileFingerprint,
    m20250312_000024_create_media_file_similarity_table::MediaFileSimilarity,
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
        // Add columns to all tables first
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
        Self::add_tracking_columns(manager, MediaFileFingerprint::Table, true).await?;
        Self::add_tracking_columns(manager, MediaFileSimilarity::Table, true).await?;

        Self::add_tracking_columns_with_existing_ts(manager, Mixes::Table).await?;
        Self::add_tracking_columns_with_existing_ts(manager, Playlists::Table).await?;
        Self::add_tracking_columns_for_mix_queries(manager).await?;

        // Populate HLC UUIDs
        Self::populate_hlc_uuids(manager).await?;

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
        Self::remove_tracking_columns(manager, MediaFileFingerprint::Table, true).await?;
        Self::remove_tracking_columns(manager, MediaFileSimilarity::Table, true).await?;

        Self::remove_tracking_columns_with_existing_ts(manager, Mixes::Table).await?;
        Self::remove_tracking_columns_with_existing_ts(manager, MixQueries::Table).await?;
        Self::remove_tracking_columns_with_existing_ts(manager, Playlists::Table).await?;

        Ok(())
    }
}

impl Migration {
    async fn populate_hlc_uuids(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let node_id = crate::get_node_id();

        // Albums
        #[derive(Debug, FromQueryResult)]
        struct AlbumRow {
            id: i32,
            name: String,
        }
        let rows: Vec<AlbumRow> = AlbumRow::find_by_statement(Statement::from_string(
            db.get_database_backend(),
            "SELECT id, name FROM albums".to_string(),
        ))
        .all(db)
        .await?;
        for row in rows {
            let uuid = Uuid::new_v5(&Uuid::NAMESPACE_OID, row.name.as_bytes()).to_string();
            manager
                .exec_stmt(
                    Query::update()
                        .table(Albums::Table)
                        .values([
                            (CommonColumns::HlcUuid, uuid.into()),
                            (CommonColumns::CreatedAtHlcNid, node_id.into()),
                            (CommonColumns::UpdatedAtHlcNid, node_id.into()),
                        ])
                        .and_where(Expr::col(Albums::Id).eq(row.id))
                        .to_owned(),
                )
                .await?;
        }

        // Artists
        #[derive(Debug, FromQueryResult)]
        struct ArtistRow {
            id: i32,
            name: String,
        }
        let rows: Vec<ArtistRow> = ArtistRow::find_by_statement(Statement::from_string(
            db.get_database_backend(),
            "SELECT id, name FROM artists".to_string(),
        ))
        .all(db)
        .await?;
        for row in rows {
            let uuid = Uuid::new_v5(&Uuid::NAMESPACE_OID, row.name.as_bytes()).to_string();
            manager
                .exec_stmt(
                    Query::update()
                        .table(Artists::Table)
                        .values([
                            (CommonColumns::HlcUuid, uuid.into()),
                            (CommonColumns::CreatedAtHlcNid, node_id.into()),
                            (CommonColumns::UpdatedAtHlcNid, node_id.into()),
                        ])
                        .and_where(Expr::col(Artists::Id).eq(row.id))
                        .to_owned(),
                )
                .await?;
        }

        // Genres
        #[derive(Debug, FromQueryResult)]
        struct GenreRow {
            id: i32,
            name: String,
        }
        let rows: Vec<GenreRow> = GenreRow::find_by_statement(Statement::from_string(
            db.get_database_backend(),
            "SELECT id, name FROM genres".to_string(),
        ))
        .all(db)
        .await?;
        for row in rows {
            let uuid = Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!("RUNE_GENRES::{}", row.name).as_bytes(),
            )
            .to_string();
            manager
                .exec_stmt(
                    Query::update()
                        .table(Genres::Table)
                        .values([
                            (CommonColumns::HlcUuid, uuid.into()),
                            (CommonColumns::CreatedAtHlcNid, node_id.into()),
                            (CommonColumns::UpdatedAtHlcNid, node_id.into()),
                        ])
                        .and_where(Expr::col(Genres::Id).eq(row.id))
                        .to_owned(),
                )
                .await?;
        }

        // MediaFiles
        #[derive(Debug, FromQueryResult)]
        struct MediaFileRow {
            id: i32,
            file_hash: String,
        }
        let rows: Vec<MediaFileRow> = MediaFileRow::find_by_statement(Statement::from_string(
            db.get_database_backend(),
            "SELECT id, file_hash FROM media_files".to_string(),
        ))
        .all(db)
        .await?;
        for row in rows {
            let uuid = Uuid::new_v5(&Uuid::NAMESPACE_OID, row.file_hash.as_bytes()).to_string();
            manager
                .exec_stmt(
                    Query::update()
                        .table(MediaFiles::Table)
                        .values([
                            (CommonColumns::HlcUuid, uuid.into()),
                            (CommonColumns::CreatedAtHlcNid, node_id.into()),
                            (CommonColumns::UpdatedAtHlcNid, node_id.into()),
                        ])
                        .and_where(Expr::col(MediaFiles::Id).eq(row.id))
                        .to_owned(),
                )
                .await?;
        }

        // MediaCoverArt
        #[derive(Debug, FromQueryResult)]
        struct CoverArtRow {
            id: i32,
            file_hash: String,
        }
        let rows: Vec<CoverArtRow> = CoverArtRow::find_by_statement(Statement::from_string(
            db.get_database_backend(),
            "SELECT id, file_hash FROM media_cover_art".to_string(),
        ))
        .all(db)
        .await?;
        for row in rows {
            let uuid = if row.file_hash.is_empty() {
                Uuid::nil().to_string()
            } else {
                Uuid::new_v5(&Uuid::NAMESPACE_OID, row.file_hash.as_bytes()).to_string()
            };
            manager
                .exec_stmt(
                    Query::update()
                        .table(MediaCoverArt::Table)
                        .values([
                            (CommonColumns::HlcUuid, uuid.into()),
                            (CommonColumns::CreatedAtHlcNid, node_id.into()),
                            (CommonColumns::UpdatedAtHlcNid, node_id.into()),
                        ])
                        .and_where(Expr::col(MediaCoverArt::Id).eq(row.id))
                        .to_owned(),
                )
                .await?;
        }

        // MediaFileFingerprint
        #[derive(Debug, FromQueryResult)]
        struct FingerprintRow {
            id: i32,
            media_file_id: i32,
        }
        let rows: Vec<FingerprintRow> = FingerprintRow::find_by_statement(Statement::from_string(
            db.get_database_backend(),
            "SELECT id, media_file_id FROM media_file_fingerprint".to_string(),
        ))
        .all(db)
        .await?;
        for row in rows {
            let uuid = Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!("RUNE_FINGERPRINT::{}", row.media_file_id).as_bytes(),
            )
            .to_string();
            manager
                .exec_stmt(
                    Query::update()
                        .table(MediaFileFingerprint::Table)
                        .values([
                            (CommonColumns::HlcUuid, uuid.into()),
                            (CommonColumns::CreatedAtHlcNid, node_id.into()),
                            (CommonColumns::UpdatedAtHlcNid, node_id.into()),
                        ])
                        .and_where(Expr::col(MediaFileFingerprint::Id).eq(row.id))
                        .to_owned(),
                )
                .await?;
        }

        // MediaFileSimilarity
        #[derive(Debug, FromQueryResult)]
        struct SimilarityRow {
            id: i32,
            file_id1: i32,
            file_id2: i32,
        }
        let rows: Vec<SimilarityRow> = SimilarityRow::find_by_statement(Statement::from_string(
            db.get_database_backend(),
            "SELECT id, file_id1, file_id2 FROM media_file_similarity".to_string(),
        ))
        .all(db)
        .await?;
        for row in rows {
            let uuid = Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!("RUNE_SIMILARITY::{}+{}", row.file_id1, row.file_id2).as_bytes(),
            )
            .to_string();
            manager
                .exec_stmt(
                    Query::update()
                        .table(MediaFileSimilarity::Table)
                        .values([
                            (CommonColumns::HlcUuid, uuid.into()),
                            (CommonColumns::CreatedAtHlcNid, node_id.into()),
                            (CommonColumns::UpdatedAtHlcNid, node_id.into()),
                        ])
                        .and_where(Expr::col(MediaFileSimilarity::Id).eq(row.id))
                        .to_owned(),
                )
                .await?;
        }

        // MediaFilePlaylists
        #[derive(Debug, FromQueryResult)]
        struct MfpRow {
            id: i32,
            playlist_id: i32,
            media_file_id: i32,
        }
        let rows: Vec<MfpRow> = MfpRow::find_by_statement(Statement::from_string(
            db.get_database_backend(),
            "SELECT id, playlist_id, media_file_id FROM media_file_playlists".to_string(),
        ))
        .all(db)
        .await?;
        for row in rows {
            let uuid = Uuid::new_v5(
                &Uuid::NAMESPACE_URL,
                format!("RUNE_PLAYLIST::{}::{}", row.playlist_id, row.media_file_id).as_bytes(),
            )
            .to_string();
            manager
                .exec_stmt(
                    Query::update()
                        .table(MediaFilePlaylists::Table)
                        .values([
                            (CommonColumns::HlcUuid, uuid.into()),
                            (CommonColumns::CreatedAtHlcNid, node_id.into()),
                            (CommonColumns::UpdatedAtHlcNid, node_id.into()),
                        ])
                        .and_where(Expr::col(MediaFilePlaylists::Id).eq(row.id))
                        .to_owned(),
                )
                .await?;
        }

        // MediaAnalysis
        #[derive(Debug, FromQueryResult)]
        struct AnalysisRow {
            id: i32,
            file_hash: String,
        }
        let rows: Vec<AnalysisRow> = AnalysisRow::find_by_statement(Statement::from_string(
            db.get_database_backend(),
            "SELECT T1.id, T2.file_hash FROM media_analysis AS T1 JOIN media_files AS T2 ON T1.file_id = T2.id"
                .to_string(),
        ))
        .all(db)
        .await?;
        for row in rows {
            let uuid = Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!("RUNE_ANALYSIS::{}", row.file_hash).as_bytes(),
            )
            .to_string();
            manager
                .exec_stmt(
                    Query::update()
                        .table(MediaAnalysis::Table)
                        .values([
                            (CommonColumns::HlcUuid, uuid.into()),
                            (CommonColumns::CreatedAtHlcNid, node_id.into()),
                            (CommonColumns::UpdatedAtHlcNid, node_id.into()),
                        ])
                        .and_where(Expr::col(MediaAnalysis::Id).eq(row.id))
                        .to_owned(),
                )
                .await?;
        }

        // MediaMetadata
        #[derive(Debug, FromQueryResult)]
        struct MetadataRow {
            id: i32,
            file_hash: String,
            meta_key: String,
        }
        let rows: Vec<MetadataRow> = MetadataRow::find_by_statement(Statement::from_string(
            db.get_database_backend(),
            "SELECT T1.id, T2.file_hash, T1.meta_key FROM media_metadata AS T1 JOIN media_files AS T2 ON T1.file_id = T2.id"
                .to_string(),
        ))
        .all(db)
        .await?;
        for row in rows {
            let uuid = Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!("RUNE_METADATA::{}::{}", row.file_hash, row.meta_key).as_bytes(),
            )
            .to_string();
            manager
                .exec_stmt(
                    Query::update()
                        .table(MediaMetadata::Table)
                        .values([
                            (CommonColumns::HlcUuid, uuid.into()),
                            (CommonColumns::CreatedAtHlcNid, node_id.into()),
                            (CommonColumns::UpdatedAtHlcNid, node_id.into()),
                        ])
                        .and_where(Expr::col(MediaMetadata::Id).eq(row.id))
                        .to_owned(),
                )
                .await?;
        }

        // MediaFileAlbums
        #[derive(Debug, FromQueryResult)]
        struct MfaRow {
            id: i32,
            album_hlc_uuid: String,
            file_hash: String,
        }
        let rows: Vec<MfaRow> = MfaRow::find_by_statement(Statement::from_string(
            db.get_database_backend(),
            "SELECT T1.id, T2.hlc_uuid AS album_hlc_uuid, T3.file_hash FROM media_file_albums AS T1 JOIN albums AS T2 ON T1.album_id = T2.id JOIN media_files AS T3 ON T1.media_file_id = T3.id"
                .to_string(),
        ))
        .all(db)
        .await?;
        for row in rows {
            let uuid = Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!("RUNE_ALBUM_FILE::{}::{}", row.album_hlc_uuid, row.file_hash).as_bytes(),
            )
            .to_string();
            manager
                .exec_stmt(
                    Query::update()
                        .table(MediaFileAlbums::Table)
                        .values([
                            (CommonColumns::HlcUuid, uuid.into()),
                            (CommonColumns::CreatedAtHlcNid, node_id.into()),
                            (CommonColumns::UpdatedAtHlcNid, node_id.into()),
                        ])
                        .and_where(Expr::col(MediaFileAlbums::Id).eq(row.id))
                        .to_owned(),
                )
                .await?;
        }

        // MediaFileArtists
        #[derive(Debug, FromQueryResult)]
        struct MfartRow {
            id: i32,
            artist_hlc_uuid: String,
            file_hash: String,
        }
        let rows: Vec<MfartRow> = MfartRow::find_by_statement(Statement::from_string(
            db.get_database_backend(),
            "SELECT T1.id, T2.hlc_uuid AS artist_hlc_uuid, T3.file_hash FROM media_file_artists AS T1 JOIN artists AS T2 ON T1.artist_id = T2.id JOIN media_files AS T3 ON T1.media_file_id = T3.id"
                .to_string(),
        ))
        .all(db)
        .await?;
        for row in rows {
            let uuid = Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!(
                    "RUNE_ARTIST_FILE::{}::{}",
                    row.artist_hlc_uuid, row.file_hash
                )
                .as_bytes(),
            )
            .to_string();
            manager
                .exec_stmt(
                    Query::update()
                        .table(MediaFileArtists::Table)
                        .values([
                            (CommonColumns::HlcUuid, uuid.into()),
                            (CommonColumns::CreatedAtHlcNid, node_id.into()),
                            (CommonColumns::UpdatedAtHlcNid, node_id.into()),
                        ])
                        .and_where(Expr::col(MediaFileArtists::Id).eq(row.id))
                        .to_owned(),
                )
                .await?;
        }

        // MediaFileGenres
        #[derive(Debug, FromQueryResult)]
        struct MfgRow {
            id: i32,
            genre_hlc_uuid: String,
            file_hash: String,
        }
        let rows: Vec<MfgRow> = MfgRow::find_by_statement(Statement::from_string(
            db.get_database_backend(),
            "SELECT T1.id, T2.hlc_uuid AS genre_hlc_uuid, T3.file_hash FROM media_file_genres AS T1 JOIN genres AS T2 ON T1.genre_id = T2.id JOIN media_files AS T3 ON T1.media_file_id = T3.id"
                .to_string(),
        ))
        .all(db)
        .await?;
        for row in rows {
            let uuid = Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!(
                    "RUNE_GENRES_FILE::{}::{}",
                    row.genre_hlc_uuid, row.file_hash
                )
                .as_bytes(),
            )
            .to_string();
            manager
                .exec_stmt(
                    Query::update()
                        .table(MediaFileGenres::Table)
                        .values([
                            (CommonColumns::HlcUuid, uuid.into()),
                            (CommonColumns::CreatedAtHlcNid, node_id.into()),
                            (CommonColumns::UpdatedAtHlcNid, node_id.into()),
                        ])
                        .and_where(Expr::col(MediaFileGenres::Id).eq(row.id))
                        .to_owned(),
                )
                .await?;
        }

        Ok(())
    }

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
                    .name(format!("idx_{}_hlc_uuid", table.to_string()))
                    .table(table)
                    .col(CommonColumns::HlcUuid)
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

        // STEP 3: Populate the 'hlc_uuid' column.
        let db = manager.get_connection();
        let node_id = crate::get_node_id();

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
                        .values([
                            (CommonColumns::HlcUuid, new_uuid.into()),
                            (CommonColumns::CreatedAtHlcNid, node_id.into()),
                            (CommonColumns::UpdatedAtHlcNid, node_id.into()),
                        ])
                        .and_where(Expr::col(PrimaryKey::Id).eq(row.id))
                        .to_owned(),
                )
                .await?;
        }

        // STEP 4: Create the index on 'hlc_uuid'.
        manager
            .create_index(
                Index::create()
                    .name(format!("idx_{}_hlc_uuid", table.to_string()))
                    .table(table)
                    .col(CommonColumns::HlcUuid)
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

    async fn add_tracking_columns_for_mix_queries(
        manager: &SchemaManager<'_>,
    ) -> Result<(), DbErr> {
        let table = MixQueries::Table;
        // STEP 1: Add new columns one by one for SQLite compatibility.
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

        // STEP 3: Populate the 'hlc_uuid' column.
        let db = manager.get_connection();
        let node_id = crate::get_node_id();

        #[derive(Debug, FromQueryResult)]
        struct MixQueryRow {
            id: i32,
            operator: String,
            mix_hlc_uuid: String,
        }
        let rows: Vec<MixQueryRow> = MixQueryRow::find_by_statement(Statement::from_string(
            db.get_database_backend(),
            "SELECT T1.id, T1.operator, T2.hlc_uuid AS mix_hlc_uuid FROM mix_queries AS T1 JOIN mixes AS T2 ON T1.mix_id = T2.id"
                .to_string(),
        ))
        .all(db)
        .await?;

        for row in rows {
            let uuid = Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!("{}{}", row.mix_hlc_uuid, row.operator).as_bytes(),
            )
            .to_string();
            manager
                .exec_stmt(
                    Query::update()
                        .table(MixQueries::Table)
                        .values([
                            (CommonColumns::HlcUuid, uuid.into()),
                            (CommonColumns::CreatedAtHlcNid, node_id.into()),
                            (CommonColumns::UpdatedAtHlcNid, node_id.into()),
                        ])
                        .and_where(Expr::col(MixQueries::Id).eq(row.id))
                        .to_owned(),
                )
                .await?;
        }

        // STEP 4: Create the index on 'hlc_uuid'.
        manager
            .create_index(
                Index::create()
                    .name(format!("idx_{}_hlc_uuid", table.to_string()))
                    .table(table)
                    .col(CommonColumns::HlcUuid)
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
