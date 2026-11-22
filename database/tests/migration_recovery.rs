use anyhow::{Context, Result};
use database::connection::connect_main_db;
use fsio::FsIo;
use migration::{Migrator, MigratorTrait};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DbBackend, SqlxSqliteConnector, Statement,
    sqlx::{
        SqlitePool,
        sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
    },
};
use sea_orm_migration::prelude::SchemaManager;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::result;
use tempfile::TempDir;
use uuid::Uuid;

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

const MIGRATION_35: &str = "m20261111_000035_drop_playback_queue";
const MIGRATION_36: &str = "m20261111_000036_add_media_file_change_token";
const MIGRATION_37: &str = "m20261112_000037_rename_source_uuid";

const FILE_UUID: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const ALBUM_UUID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const ARTIST_UUID: &str = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
const PLAYLIST_UUID: &str = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";
const MIX_UUID: &str = "ffffffff-ffff-4fff-8fff-ffffffffffff";
const STAT_UUID: &str = "11111111-1111-4111-8111-111111111111";
const LIKE_UUID: &str = "22222222-2222-4222-8222-222222222222";

struct DiskLibraryFixture {
    _dir: TempDir,
    root: String,
    db_path: PathBuf,
    actor: String,
    indexed_tables_before_37: Vec<String>,
}

async fn open_raw_sqlite(db_path: &Path) -> Result<DatabaseConnection> {
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .synchronous(SqliteSynchronous::Extra)
        .journal_mode(SqliteJournalMode::Delete);
    let pool = SqlitePool::connect_with(options).await?;
    Ok(SqlxSqliteConnector::from_sqlx_sqlite_pool(pool))
}

fn migration_position(name: &str) -> u32 {
    Migrator::migrations()
        .iter()
        .position(|m| m.name() == name)
        .unwrap_or_else(|| panic!("missing migration {name}")) as u32
}

async fn setup_library_before(target_migration: &str) -> Result<DiskLibraryFixture> {
    let actor = Uuid::new_v4().to_string();
    let _ = migration::initialize_node_id(actor.clone());

    let dir = TempDir::new()?;
    let root_path = dir.path().join("library");
    let rune_dir = root_path.join(".rune");
    fs::create_dir_all(&rune_dir)?;
    let db_path = rune_dir.join(".0.db");

    let db = open_raw_sqlite(&db_path).await?;
    let steps = migration_position(target_migration);
    migration::up_with_actor(&db, Some(&actor), Some(steps)).await?;

    let schema = SchemaManager::new(&db);

    // Seed representative rows across tables touched by migrations 35-37.
    db.execute_unprepared(&format!(
        "INSERT INTO media_files \
         (id, file_name, directory, extension, file_hash, last_modified, sample_rate, duration, hlc_uuid) \
         VALUES (204, 'track.ogg', '/music', 'ogg', 'hash-204', '2026-01-01', 44100, 180, '{FILE_UUID}')"
    ))
    .await?;
    db.execute_unprepared(&format!(
        "INSERT INTO albums (id, name, \"group\", hlc_uuid) VALUES (10, 'Test Album', 'T', '{ALBUM_UUID}')"
    ))
    .await?;
    db.execute_unprepared(&format!(
        "INSERT INTO artists (id, name, \"group\", hlc_uuid) VALUES (20, 'Test Artist', 'T', '{ARTIST_UUID}')"
    ))
    .await?;
    db.execute_unprepared(&format!(
        "INSERT INTO playlists (id, name, \"group\", hlc_uuid) \
         VALUES (30, 'Test Playlist', 'T', '{PLAYLIST_UUID}')"
    ))
    .await?;
    db.execute_unprepared(&format!(
        "INSERT INTO mixes (id, name, \"group\", locked, scriptlet_mode, hlc_uuid) \
         VALUES (40, 'Test Mix', 'T', 0, 0, '{MIX_UUID}')"
    ))
    .await?;
    db.execute_unprepared(&format!(
        "INSERT INTO media_file_stats \
         (id, track_uuid, device_id, hlc_uuid, skipped, played_through, updated_at) \
         VALUES (50, '{FILE_UUID}', '{actor}', '{STAT_UUID}', 2, 7, '2026-01-01T00:00:00Z')"
    ))
    .await?;
    db.execute_unprepared(&format!(
        "INSERT INTO media_file_likes (id, track_uuid, hlc_uuid, liked, updated_at) \
         VALUES (60, '{FILE_UUID}', '{LIKE_UUID}', 1, '2026-01-01T00:00:00Z')"
    ))
    .await?;

    if schema.has_table("playback_queue").await? {
        db.execute_unprepared("INSERT INTO playback_queue (id, media_file_id) VALUES (1, 204)")
            .await?;
    }

    let mut indexed_tables_before_37 = Vec::new();
    for table in SOURCE_TABLES {
        let index_name = format!("idx_{table}_hlc_uuid");
        if schema.has_index(table, &index_name).await? {
            indexed_tables_before_37.push((*table).to_owned());
        }
    }
    assert!(
        !indexed_tables_before_37.is_empty(),
        "expected at least one idx_*_hlc_uuid index before migration 37"
    );

    db.close().await?;

    Ok(DiskLibraryFixture {
        _dir: dir,
        root: root_path.to_str().context("utf8 library path")?.to_owned(),
        db_path,
        actor,
        indexed_tables_before_37,
    })
}

async fn install_bookkeeping_fault(
    db_path: &Path,
    trigger_name: &str,
    version: &str,
) -> Result<()> {
    let db = open_raw_sqlite(db_path).await?;
    db.execute_unprepared(&format!(
        "CREATE TRIGGER {trigger_name} \
         BEFORE INSERT ON seaql_migrations \
         WHEN NEW.version = '{version}' \
         BEGIN \
             SELECT RAISE(ABORT, 'injected failure before bookkeeping for {version}'); \
         END;"
    ))
    .await?;
    db.close().await?;
    Ok(())
}

async fn drop_trigger(db_path: &Path, trigger_name: &str) -> Result<()> {
    let db = open_raw_sqlite(db_path).await?;
    db.execute_unprepared(&format!("DROP TRIGGER IF EXISTS {trigger_name}"))
        .await?;
    db.close().await?;
    Ok(())
}

async fn has_migration_record(db: &DatabaseConnection, version: &str) -> Result<bool> {
    let row = db
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            format!("SELECT version FROM seaql_migrations WHERE version = '{version}'"),
        ))
        .await?;
    Ok(row.is_some())
}

async fn all_source_tables_have_hlc_uuid(
    db: &DatabaseConnection,
    indexed_tables: &[String],
) -> Result<bool> {
    let schema = SchemaManager::new(db);
    for table in SOURCE_TABLES {
        if !schema.has_column(table, "hlc_uuid").await?
            || schema.has_column(table, "source_uuid").await?
        {
            return Ok(false);
        }
    }
    for table in indexed_tables {
        let old_index = format!("idx_{table}_hlc_uuid");
        let new_index = format!("idx_{table}_source_uuid");
        if !schema.has_index(table, &old_index).await?
            || schema.has_index(table, &new_index).await?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn verify_upgraded_library(
    db: &DatabaseConnection,
    indexed_tables: &[String],
    actor: &str,
) -> Result<()> {
    let schema = SchemaManager::new(db);

    // Migration 35: playback_queue removed.
    assert!(
        !schema.has_table("playback_queue").await?,
        "playback_queue must be dropped after upgrade"
    );

    // Migration 36: change_token added to media_files.
    assert!(
        schema.has_column("media_files", "change_token").await?,
        "media_files.change_token must exist after upgrade"
    );

    // Migration 37: all 18 tables renamed hlc_uuid -> source_uuid.
    for table in SOURCE_TABLES {
        assert!(
            schema.has_column(table, "source_uuid").await?,
            "{table}.source_uuid missing after upgrade"
        );
        assert!(
            !schema.has_column(table, "hlc_uuid").await?,
            "{table}.hlc_uuid still present after upgrade"
        );
    }

    // All idx_*_hlc_uuid indexes were replaced with idx_*_source_uuid.
    for table in indexed_tables {
        let new_index = format!("idx_{table}_source_uuid");
        let old_index = format!("idx_{table}_hlc_uuid");
        assert!(
            schema.has_index(table, &new_index).await?,
            "expected index {new_index} on {table}"
        );
        assert!(
            !schema.has_index(table, &old_index).await?,
            "stale index {old_index} still present on {table}"
        );
    }

    // Unique indexes on likes, fingerprint, and similarity remain intact.
    let likes_indexes = db
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA index_list('media_file_likes')".to_owned(),
        ))
        .await?;
    assert!(likes_indexes.iter().any(|idx| {
        idx.try_get::<String>("", "name").unwrap() == "p2_media_file_likes_uuid"
            && idx.try_get::<i64>("", "unique").unwrap() == 1
    }));

    // Seeded rows and UUIDs are preserved verbatim.
    for (table, id, expected_uuid) in [
        ("media_files", 204, FILE_UUID),
        ("albums", 10, ALBUM_UUID),
        ("artists", 20, ARTIST_UUID),
        ("playlists", 30, PLAYLIST_UUID),
        ("mixes", 40, MIX_UUID),
        ("media_file_stats", 50, STAT_UUID),
        ("media_file_likes", 60, LIKE_UUID),
    ] {
        let row = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("SELECT source_uuid FROM {table} WHERE id = {id}"),
            ))
            .await?
            .with_context(|| format!("missing preserved row {id} in {table}"))?;
        let actual: String = row.try_get("", "source_uuid")?;
        assert_eq!(actual, expected_uuid, "UUID mismatch in {table} row {id}");
    }

    let stats_row = db
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT device_id, skipped, played_through FROM media_file_stats WHERE id = 50"
                .to_owned(),
        ))
        .await?
        .context("missing media_file_stats row 50")?;
    assert_eq!(stats_row.try_get::<String>("", "device_id")?, actor);
    assert_eq!(stats_row.try_get::<i32>("", "skipped")?, 2);
    assert_eq!(stats_row.try_get::<i32>("", "played_through")?, 7);

    // Complete seaql_migrations ledger with zero pending migrations.
    let pending = Migrator::get_pending_migrations(db).await?;
    assert!(
        pending.is_empty(),
        "expected zero pending migrations, found: {:?}",
        pending.iter().map(|m| m.name()).collect::<Vec<_>>()
    );
    let expected_versions: BTreeSet<String> = Migrator::migrations()
        .into_iter()
        .map(|m| m.name().to_owned())
        .collect();
    let recorded_rows = db
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT version FROM seaql_migrations".to_owned(),
        ))
        .await?;
    let recorded_versions: BTreeSet<String> = recorded_rows
        .into_iter()
        .map(|row| row.try_get::<String>("", "version"))
        .collect::<result::Result<_, _>>()?;
    assert_eq!(recorded_versions, expected_versions);

    Ok(())
}

#[tokio::test]
async fn migration_35_interrupted_before_bookkeeping_rolls_back_and_reopens() -> Result<()> {
    let fixture = setup_library_before(MIGRATION_35).await?;
    install_bookkeeping_fault(&fixture.db_path, "fail_ledger_m35", MIGRATION_35).await?;

    let fsio = FsIo::new();
    let first_open = connect_main_db(&fsio, &fixture.root, None, &fixture.actor).await;
    assert!(
        first_open.is_err(),
        "expected injected bookkeeping failure during migration 35"
    );

    // Clear the transient fault, capture the pre-retry state, then reopen.
    drop_trigger(&fixture.db_path, "fail_ledger_m35").await?;
    let (queue_rows_before_retry, had_record_before_retry) = {
        let db = open_raw_sqlite(&fixture.db_path).await?;
        let schema = SchemaManager::new(&db);
        let rows = if schema.has_table("playback_queue").await? {
            db.query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM playback_queue WHERE id = 1 AND media_file_id = 204"
                    .to_owned(),
            ))
            .await?
            .context("playback_queue count")?
            .try_get::<i64>("", "n")?
        } else {
            0
        };
        let has_record = has_migration_record(&db, MIGRATION_35).await?;
        db.close().await?;
        (rows, has_record)
    };

    let reopened = connect_main_db(&fsio, &fixture.root, None, &fixture.actor)
        .await
        .context("reopening library after interrupted migration 35 failed")?;
    assert_eq!(
        queue_rows_before_retry, 1,
        "playback_queue and its row must remain intact after rolled-back migration 35"
    );
    assert!(
        !had_record_before_retry,
        "migration 35 must not be recorded before commit"
    );
    verify_upgraded_library(&reopened, &fixture.indexed_tables_before_37, &fixture.actor).await?;
    reopened.close().await?;
    Ok(())
}

#[tokio::test]
async fn migration_36_interrupted_before_bookkeeping_rolls_back_and_reopens() -> Result<()> {
    let fixture = setup_library_before(MIGRATION_36).await?;
    install_bookkeeping_fault(&fixture.db_path, "fail_ledger_m36", MIGRATION_36).await?;

    let fsio = FsIo::new();
    let first_open = connect_main_db(&fsio, &fixture.root, None, &fixture.actor).await;
    assert!(
        first_open.is_err(),
        "expected injected bookkeeping failure during migration 36"
    );

    drop_trigger(&fixture.db_path, "fail_ledger_m36").await?;
    let (had_change_token_before_retry, had_record_before_retry) = {
        let db = open_raw_sqlite(&fixture.db_path).await?;
        let schema = SchemaManager::new(&db);
        let has_col = schema.has_column("media_files", "change_token").await?;
        let has_record = has_migration_record(&db, MIGRATION_36).await?;
        db.close().await?;
        (has_col, has_record)
    };

    let reopened = connect_main_db(&fsio, &fixture.root, None, &fixture.actor)
        .await
        .context("reopening library after interrupted migration 36 failed")?;
    assert!(
        !had_change_token_before_retry,
        "media_files.change_token must not remain after rolled-back migration 36"
    );
    assert!(
        !had_record_before_retry,
        "migration 36 must not be recorded before commit"
    );
    verify_upgraded_library(&reopened, &fixture.indexed_tables_before_37, &fixture.actor).await?;
    reopened.close().await?;
    Ok(())
}

#[tokio::test]
async fn migration_37_interrupted_mid_multi_table_rename_rolls_back_and_reopens() -> Result<()> {
    let fixture = setup_library_before(MIGRATION_37).await?;

    // Cause table 18 ("playlists") in SOURCE_TABLES to fail after tables 1..17 have executed DDL.
    {
        let db = open_raw_sqlite(&fixture.db_path).await?;
        db.execute_unprepared("ALTER TABLE playlists ADD COLUMN source_uuid TEXT")
            .await?;
        db.close().await?;
    }

    let fsio = FsIo::new();
    let first_open = connect_main_db(&fsio, &fixture.root, None, &fixture.actor).await;
    assert!(
        first_open.is_err(),
        "expected injected mid-rename failure on playlists during migration 37"
    );

    // Clear the injected conflict on playlists and capture pre-retry schema before reopening.
    let (all_tables_rolled_back, had_record_before_retry) = {
        let db = open_raw_sqlite(&fixture.db_path).await?;
        db.execute_unprepared("ALTER TABLE playlists DROP COLUMN source_uuid")
            .await?;
        let has_record = has_migration_record(&db, MIGRATION_37).await?;
        let all_hlc =
            all_source_tables_have_hlc_uuid(&db, &fixture.indexed_tables_before_37).await?;
        db.close().await?;
        (all_hlc, has_record)
    };

    let reopened = connect_main_db(&fsio, &fixture.root, None, &fixture.actor)
        .await
        .context("reopening library after interrupted multi-table rename in migration 37 failed")?;
    assert!(
        all_tables_rolled_back,
        "all 18 tables must roll back to hlc_uuid after mid-rename failure"
    );
    assert!(
        !had_record_before_retry,
        "migration 37 must not be recorded after mid-rename failure"
    );
    verify_upgraded_library(&reopened, &fixture.indexed_tables_before_37, &fixture.actor).await?;
    reopened.close().await?;
    Ok(())
}

#[tokio::test]
async fn migration_37_interrupted_before_bookkeeping_rolls_back_and_reopens() -> Result<()> {
    let fixture = setup_library_before(MIGRATION_37).await?;
    install_bookkeeping_fault(&fixture.db_path, "fail_ledger_m37", MIGRATION_37).await?;

    let fsio = FsIo::new();
    let first_open = connect_main_db(&fsio, &fixture.root, None, &fixture.actor).await;
    assert!(
        first_open.is_err(),
        "expected injected bookkeeping failure during migration 37"
    );

    drop_trigger(&fixture.db_path, "fail_ledger_m37").await?;
    let (all_tables_rolled_back, had_record_before_retry) = {
        let db = open_raw_sqlite(&fixture.db_path).await?;
        let has_record = has_migration_record(&db, MIGRATION_37).await?;
        let all_hlc =
            all_source_tables_have_hlc_uuid(&db, &fixture.indexed_tables_before_37).await?;
        db.close().await?;
        (all_hlc, has_record)
    };

    let reopened = connect_main_db(&fsio, &fixture.root, None, &fixture.actor)
        .await
        .context("reopening library after interrupted migration 37 bookkeeping failed")?;
    assert!(
        all_tables_rolled_back,
        "all 18 tables must roll back to hlc_uuid after bookkeeping failure"
    );
    assert!(
        !had_record_before_retry,
        "migration 37 must not be recorded before commit"
    );
    verify_upgraded_library(&reopened, &fixture.indexed_tables_before_37, &fixture.actor).await?;
    reopened.close().await?;
    Ok(())
}
