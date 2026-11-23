use migration::{Migrator, MigratorTrait};
use sea_orm_migration::{
    prelude::SchemaManager,
    sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement},
};

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

/// Tables that exist before the source conversion adds likes and statistics.
const ALPHA_TABLES: &[&str] = &[
    "albums",
    "artists",
    "genres",
    "media_analysis",
    "media_cover_art",
    "media_file_albums",
    "media_file_artists",
    "media_file_fingerprint",
    "media_file_genres",
    "media_file_playlists",
    "media_file_similarity",
    "media_files",
    "media_metadata",
    "mix_queries",
    "mixes",
    "playlists",
];

/// Retired timestamp bookkeeping. Current migrations neither create nor read it;
/// an older alpha database keeps it because every column has a default.
const BOOKKEEPING_COLUMNS: [&str; 6] = [
    "created_at_hlc_ts",
    "created_at_hlc_ver",
    "created_at_hlc_nid",
    "updated_at_hlc_ts",
    "updated_at_hlc_ver",
    "updated_at_hlc_nid",
];

const NODE: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const ARTWORK_BODY: &[u8] = &[1, 2, 3];

async fn database_at_alpha_schema() -> DatabaseConnection {
    let _ = migration::initialize_node_id(NODE.to_owned());
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let steps = Migrator::migrations()
        .iter()
        .position(|m| m.name() == "m20250410_000025_add_hlc_columns")
        .unwrap() as u32;
    Migrator::up(&db, Some(steps + 1)).await.unwrap();
    db
}

async fn add_bookkeeping_columns(db: &DatabaseConnection) {
    for table in ALPHA_TABLES {
        for prefix in ["created_at", "updated_at"] {
            for (suffix, def) in [
                ("ts", "TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z'"),
                ("ver", "INTEGER NOT NULL DEFAULT 0"),
                ("nid", "TEXT NOT NULL DEFAULT ''"),
            ] {
                db.execute_unprepared(&format!(
                    "ALTER TABLE {table} ADD COLUMN {prefix}_hlc_{suffix} {def}"
                ))
                .await
                .unwrap();
            }
        }
    }
}

/// Identities read back before the upgrade.
struct Populated {
    playlist: String,
    mix: String,
    directory_clause: String,
    track_clause: String,
    track: String,
    membership: String,
}

async fn populate(db: &DatabaseConnection, checked_at: bool) -> Populated {
    let cover_art = if checked_at {
        "INSERT INTO media_cover_art(id,file_hash,binary,primary_color,hlc_uuid,updated_at_hlc_ts) VALUES(203,'crc',x'010203',0,'10000000-0000-4000-8000-000000000203','2023-01-02 03:04:05')"
    } else {
        "INSERT INTO media_cover_art(id,file_hash,binary,primary_color,hlc_uuid) VALUES(203,'crc',x'010203',0,'10000000-0000-4000-8000-000000000203')"
    };
    for sql in [
        cover_art,
        "INSERT INTO playlists(id,name,\"group\",hlc_uuid) VALUES(200,'list','g','10000000-0000-4000-8000-000000000200')",
        "INSERT INTO mixes(id,name,\"group\",locked,scriptlet_mode,hlc_uuid) VALUES(201,'mix','g',0,0,'10000000-0000-4000-8000-000000000201')",
        "INSERT INTO mix_queries(id,mix_id,operator,parameter,\"group\",hlc_uuid) VALUES(202,201,'lib::directory.deep','/music',0,'10000000-0000-4000-8000-000000000202')",
        "INSERT INTO mix_queries(id,mix_id,operator,parameter,\"group\",hlc_uuid) VALUES(206,201,'lib::track','204',0,'10000000-0000-4000-8000-000000000206')",
        "INSERT INTO media_files(id,file_name,directory,extension,file_hash,last_modified,cover_art_id,sample_rate,duration,hlc_uuid) VALUES(204,'a.ogg','/music','ogg','hash','2020',203,44100,12,'10000000-0000-4000-8000-000000000204')",
        "INSERT INTO media_file_playlists(id,playlist_id,media_file_id,position,hlc_uuid) VALUES(205,200,204,0,'10000000-0000-4000-8000-000000000205')",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
    Populated {
        playlist: text(db, "SELECT hlc_uuid FROM playlists WHERE id=200").await,
        mix: text(db, "SELECT hlc_uuid FROM mixes WHERE id=201").await,
        directory_clause: text(db, "SELECT hlc_uuid FROM mix_queries WHERE id=202").await,
        track_clause: text(db, "SELECT hlc_uuid FROM mix_queries WHERE id=206").await,
        track: text(db, "SELECT hlc_uuid FROM media_files WHERE id=204").await,
        membership: text(db, "SELECT hlc_uuid FROM media_file_playlists WHERE id=205").await,
    }
}

async fn text(db: &DatabaseConnection, sql: &str) -> String {
    let row = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            sql.to_owned(),
        ))
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("no row for {sql}"));
    row.try_get_by_index(0).unwrap()
}

async fn optional_text(db: &DatabaseConnection, sql: &str) -> Option<String> {
    let row = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            sql.to_owned(),
        ))
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("no row for {sql}"));
    row.try_get_by_index(0).unwrap()
}

async fn count(db: &DatabaseConnection, sql: &str) -> i64 {
    let row = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            sql.to_owned(),
        ))
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("no row for {sql}"));
    row.try_get_by_index(0).unwrap()
}

async fn assert_upgrade_contract(db: &DatabaseConnection, populated: &Populated, alpha: bool) {
    let schema = SchemaManager::new(db);
    for table in SOURCE_TABLES {
        assert!(
            schema.has_column(table, "source_uuid").await.unwrap(),
            "{table}.source_uuid"
        );
        assert!(
            !schema.has_column(table, "hlc_uuid").await.unwrap(),
            "{table}.hlc_uuid"
        );
        let keeps_bookkeeping = alpha && ALPHA_TABLES.contains(table);
        // Retired columns stay absent from current schemas. An alpha database
        // keeps the six defaults on tables that existed then; later tables never
        // had them, and current code neither reads nor writes either set.
        for column in BOOKKEEPING_COLUMNS {
            assert_eq!(
                schema.has_column(table, column).await.unwrap(),
                keeps_bookkeeping,
                "{table}.{column}"
            );
        }
    }
    assert!(
        schema
            .has_index("media_files", "idx_media_files_source_uuid")
            .await
            .unwrap()
    );
    assert!(
        !schema
            .has_index("media_files", "idx_media_files_hlc_uuid")
            .await
            .unwrap()
    );

    for (table, id) in [
        ("playlists", 200),
        ("mixes", 201),
        ("mix_queries", 202),
        ("mix_queries", 206),
        ("media_cover_art", 203),
        ("media_files", 204),
        ("media_file_playlists", 205),
    ] {
        assert_eq!(
            count(db, &format!("SELECT COUNT(*) FROM {table} WHERE id={id}")).await,
            1,
            "{table} keeps local id {id}"
        );
    }
    assert_eq!(
        text(db, "SELECT source_uuid FROM playlists WHERE id=200").await,
        populated.playlist
    );
    assert_eq!(
        text(db, "SELECT source_uuid FROM mixes WHERE id=201").await,
        populated.mix
    );
    assert_eq!(
        text(db, "SELECT source_uuid FROM mix_queries WHERE id=202").await,
        populated.directory_clause
    );
    assert_eq!(
        text(db, "SELECT source_uuid FROM mix_queries WHERE id=206").await,
        populated.track_clause
    );
    assert_eq!(
        text(db, "SELECT source_uuid FROM media_files WHERE id=204").await,
        populated.track
    );
    assert_eq!(
        text(
            db,
            "SELECT source_uuid FROM media_file_playlists WHERE id=205"
        )
        .await,
        populated.membership
    );

    // Artwork identity follows the stored body digest; the track reference follows it.
    let artwork =
        migration::catalog_identity::artwork(blake3::hash(ARTWORK_BODY).to_hex().as_str())
            .to_string();
    assert_eq!(
        text(db, "SELECT source_uuid FROM media_cover_art WHERE id=203").await,
        artwork
    );
    assert_eq!(
        text(db, "SELECT cover_art_uuid FROM media_files WHERE id=204").await,
        artwork
    );

    assert_eq!(
        text(db, "SELECT mix_uuid FROM mix_queries WHERE id=202").await,
        populated.mix
    );
    assert_eq!(
        text(db, "SELECT mix_uuid FROM mix_queries WHERE id=206").await,
        populated.mix
    );
    assert_eq!(
        optional_text(db, "SELECT target_uuid FROM mix_queries WHERE id=202").await,
        None
    );
    assert_eq!(
        text(db, "SELECT target_uuid FROM mix_queries WHERE id=206").await,
        populated.track
    );
    assert_eq!(
        text(db, "SELECT local_directory FROM mix_queries WHERE id=202").await,
        "/music"
    );
    assert_eq!(
        text(
            db,
            "SELECT playlist_uuid FROM media_file_playlists WHERE id=205"
        )
        .await,
        populated.playlist
    );
    assert_eq!(
        text(
            db,
            "SELECT track_uuid FROM media_file_playlists WHERE id=205"
        )
        .await,
        populated.track
    );
    assert_eq!(
        optional_text(
            db,
            "SELECT cover_art_checked_at FROM media_files WHERE id=204"
        )
        .await,
        alpha.then(|| "2023-01-02 03:04:05".to_owned())
    );
}

#[tokio::test]
async fn fresh_database_has_source_uuid_columns() {
    let _ = migration::initialize_node_id(NODE.to_owned());
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let schema = SchemaManager::new(&db);
    for table in SOURCE_TABLES {
        assert!(
            schema.has_column(table, "source_uuid").await.unwrap(),
            "{table}"
        );
        assert!(
            !schema.has_column(table, "hlc_uuid").await.unwrap(),
            "{table}"
        );
        for column in BOOKKEEPING_COLUMNS {
            assert!(
                !schema.has_column(table, column).await.unwrap(),
                "{table}.{column}"
            );
        }
    }
    assert!(
        schema
            .has_index("media_files", "idx_media_files_source_uuid")
            .await
            .unwrap()
    );
    assert!(
        !schema
            .has_index("media_files", "idx_media_files_hlc_uuid")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn existing_source_uuids_survive_column_rename() {
    let _ = migration::initialize_node_id(NODE.to_owned());
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let before_rename = Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m20261112_000037_rename_source_uuid")
        .expect("migration 37 must be registered") as u32;
    Migrator::up(&db, Some(before_rename)).await.unwrap();
    db.execute_unprepared("INSERT INTO media_files(id,file_name,directory,extension,file_hash,last_modified,sample_rate,duration,hlc_uuid) VALUES(204,'a.ogg','/music','ogg','hash','2020',44100,12,'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')")
        .await
        .unwrap();

    Migrator::up(&db, None).await.unwrap();

    let schema = SchemaManager::new(&db);
    assert!(
        schema
            .has_index("media_files", "idx_media_files_source_uuid")
            .await
            .unwrap()
    );
    assert!(
        !schema
            .has_index("media_files", "idx_media_files_hlc_uuid")
            .await
            .unwrap()
    );
    let unique_indexes = db
        .query_all_raw(Statement::from_string(
            db.get_database_backend(),
            "PRAGMA index_list('media_file_likes')".to_owned(),
        ))
        .await
        .unwrap();
    assert!(unique_indexes.iter().any(|index| {
        index.try_get::<String>("", "name").unwrap() == "p2_media_file_likes_uuid"
            && index.try_get::<i64>("", "unique").unwrap() == 1
    }));

    for (table, expected) in [("media_files", "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb")] {
        let row = db
            .query_one_raw(Statement::from_string(
                db.get_database_backend(),
                format!("SELECT source_uuid FROM {table} WHERE id = 204"),
            ))
            .await
            .unwrap()
            .unwrap();
        let actual: String = row.try_get("", "source_uuid").unwrap();
        assert_eq!(actual, expected);
    }
}

#[tokio::test]
async fn older_database_upgrade_keeps_populated_identities() {
    let db = database_at_alpha_schema().await;
    let populated = populate(&db, false).await;
    migration::up_with_actor(&db, Some(NODE), None)
        .await
        .unwrap();
    assert_upgrade_contract(&db, &populated, false).await;
}

#[tokio::test]
async fn alpha_database_with_bookkeeping_columns_keeps_populated_identities() {
    let db = database_at_alpha_schema().await;
    add_bookkeeping_columns(&db).await;
    let populated = populate(&db, true).await;
    migration::up_with_actor(&db, Some(NODE), None)
        .await
        .unwrap();
    assert_upgrade_contract(&db, &populated, true).await;
}
