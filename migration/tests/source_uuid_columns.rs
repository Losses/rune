use migration::{Migrator, MigratorTrait};
use sea_orm_migration::{
    prelude::SchemaManager,
    sea_orm::{ConnectionTrait, Database, Statement},
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

#[tokio::test]
async fn fresh_database_has_source_uuid_columns() {
    let _ = migration::initialize_node_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned());
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
    let _ = migration::initialize_node_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned());
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let migration_count = Migrator::migrations().len() as u32;
    Migrator::up(&db, Some(migration_count - 1)).await.unwrap();
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
