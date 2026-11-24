//! Libraries made by builds before the first release are migrated in place:
//! every row they hold is still there, in today's tables, after the whole
//! migrator has run. Fresh and already migrated databases pass through the
//! migrations added for them unchanged.

use migration::test_fixtures::{PreReleaseLibrary, write_pre_release_library};
use migration::{Migrator, MigratorTrait};
use sea_orm_migration::{
    SchemaManager,
    sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement},
};

const NODE: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

/// The migrations added for pre-release libraries.
const ADDED: [&str; 5] = [
    "m20230701_000004_create_user_logs_table",
    "m20230701_000006_create_playlist_items_table",
    "m20230701_000007_create_smart_playlists_table",
    "m20230728_000008_create_media_cover_arts_table",
    "m20261123_000039_settle_pre_release_tables",
];

async fn database() -> DatabaseConnection {
    let _ = migration::initialize_node_id(NODE.to_owned());
    Database::connect("sqlite::memory:").await.unwrap()
}

async fn migrate(db: &DatabaseConnection) {
    migration::up_with_actor(db, Some(NODE), None)
        .await
        .unwrap();
}

async fn strings(db: &DatabaseConnection, sql: &str) -> Vec<String> {
    db.query_all_raw(Statement::from_string(
        db.get_database_backend(),
        sql.to_owned(),
    ))
    .await
    .unwrap_or_else(|error| panic!("{sql}: {error}"))
    .into_iter()
    .map(|row| row.try_get_by_index(0).unwrap())
    .collect()
}

async fn count(db: &DatabaseConnection, sql: &str) -> i64 {
    db.query_one_raw(Statement::from_string(
        db.get_database_backend(),
        sql.to_owned(),
    ))
    .await
    .unwrap_or_else(|error| panic!("{sql}: {error}"))
    .unwrap()
    .try_get_by_index(0)
    .unwrap()
}

async fn recorded(db: &DatabaseConnection) -> Vec<String> {
    let mut versions = strings(db, "SELECT version FROM seaql_migrations").await;
    versions.sort();
    versions
}

fn all_migrations() -> Vec<String> {
    let mut names: Vec<String> = Migrator::migrations()
        .iter()
        .map(|migration| migration.name().to_owned())
        .collect();
    names.sort();
    names
}

/// Each table with its column names, sorted.
async fn tables(db: &DatabaseConnection) -> Vec<(String, Vec<String>)> {
    let mut tables = Vec::new();
    for table in strings(
        db,
        "SELECT name FROM sqlite_master WHERE type = 'table' \
         AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .await
    {
        let mut columns = strings(
            db,
            &format!("SELECT name FROM pragma_table_info('{table}')"),
        )
        .await;
        columns.sort();
        tables.push((table, columns));
    }
    tables
}

#[tokio::test]
async fn every_pre_release_library_keeps_its_rows() {
    let fresh = database().await;
    migrate(&fresh).await;
    let today = tables(&fresh).await;
    for layout in PreReleaseLibrary::ALL {
        let db = database().await;
        write_pre_release_library(&db, layout).await.unwrap();
        migrate(&db).await;

        assert_eq!(recorded(&db).await, all_migrations(), "{layout:?}");
        // The same tables with the same columns as a database made today.
        assert_eq!(tables(&db).await, today, "{layout:?}");
        let schema = SchemaManager::new(&db);
        for gone in [
            "user_logs",
            "smart_playlists",
            "playlist_items",
            "media_cover_arts",
        ] {
            assert!(!schema.has_table(gone).await.unwrap(), "{layout:?}: {gone}");
        }

        assert_eq!(
            strings(
                &db,
                "SELECT directory || '/' || file_name FROM media_files ORDER BY id"
            )
            .await,
            ["/music/Intro.flac", "/music/Outro.flac"],
            "{layout:?}"
        );
        assert_eq!(
            strings(
                &db,
                "SELECT f.file_name || ':' || m.meta_key || '=' || m.meta_value \
                 FROM music_metadata_source_tags m JOIN media_files f ON f.id = m.file_id \
                 ORDER BY m.id"
            )
            .await,
            [
                "Intro.flac:title=Intro",
                "Intro.flac:artist=Ärzte",
                "Outro.flac:title=Outro"
            ],
            "{layout:?}"
        );
        // The playlist keeps its tracks in their order.
        assert_eq!(
            strings(
                &db,
                "SELECT p.name || ':' || p.\"group\" || ':' || f.file_name \
                 FROM media_file_playlists i \
                 JOIN playlists p ON p.source_uuid = i.playlist_uuid \
                 JOIN media_files f ON f.source_uuid = i.track_uuid \
                 ORDER BY i.position"
            )
            .await,
            ["Favourites::Outro.flac", "Favourites::Intro.flac"],
            "{layout:?}"
        );
        if layout.has_cover_art() {
            assert_eq!(
                strings(
                    &db,
                    "SELECT f.file_name || ':' || hex(a.binary) FROM media_files f \
                     JOIN media_cover_art a ON a.source_uuid = f.cover_art_uuid"
                )
                .await,
                ["Intro.flac:89504E47"],
                "{layout:?}"
            );
        }
        if layout.has_artists() {
            assert_eq!(
                strings(
                    &db,
                    "SELECT f.file_name || ':' || a.name || ':' || a.\"group\" \
                     FROM music_metadata_audio_artists l \
                     JOIN music_metadata_artists a ON a.source_uuid = l.artist_uuid \
                     JOIN media_files f ON f.source_uuid = l.track_uuid"
                )
                .await,
                ["Intro.flac:Ärzte:A"],
                "{layout:?}"
            );
            assert_eq!(
                strings(
                    &db,
                    "SELECT f.file_name || ':' || a.portable_title || ':' || a.portable_group \
                     FROM music_metadata_track_album_sources l \
                     JOIN music_metadata_albums a ON a.id = l.album_uuid \
                     JOIN media_files f ON f.source_uuid = l.track_uuid"
                )
                .await,
                ["Intro.flac:Greatest:G"],
                "{layout:?}"
            );
        }
    }
}

#[tokio::test]
async fn a_fresh_database_applies_the_added_migrations_as_nothing() {
    let db = database().await;
    migrate(&db).await;
    assert_eq!(recorded(&db).await, all_migrations());
    let schema = SchemaManager::new(&db);
    for table in ["user_logs", "smart_playlists", "playlist_items"] {
        assert!(!schema.has_table(table).await.unwrap(), "{table}");
    }
    // Running again finds nothing to do.
    migrate(&db).await;
}

#[tokio::test]
async fn a_database_migrated_before_them_runs_the_added_migrations_last() {
    let db = database().await;
    migrate(&db).await;
    db.execute_unprepared(
        "INSERT INTO playlists (name, \"group\", source_uuid) \
         VALUES ('Kept', '', 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')",
    )
    .await
    .unwrap();
    let schema_before = strings(
        &db,
        "SELECT ifnull(sql, '') FROM sqlite_master WHERE name != 'seaql_migrations' ORDER BY name",
    )
    .await;
    // As a released build left it: none of the added migrations recorded.
    for version in ADDED {
        db.execute_unprepared(&format!(
            "DELETE FROM seaql_migrations WHERE version = '{version}'"
        ))
        .await
        .unwrap();
    }
    migrate(&db).await;
    assert_eq!(recorded(&db).await, all_migrations());
    assert_eq!(count(&db, "SELECT count(*) FROM playlists").await, 1);
    // They run last there and change nothing.
    assert_eq!(
        strings(
            &db,
            "SELECT ifnull(sql, '') FROM sqlite_master WHERE name != 'seaql_migrations' ORDER BY name",
        )
        .await,
        schema_before
    );
}
