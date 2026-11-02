use migration::{Migrator, MigratorTrait};
use sea_orm_migration::{prelude::SchemaManager, sea_orm::Database};

#[tokio::test]
async fn fresh_database_has_no_hlc_bookkeeping_columns() {
    let _ = migration::initialize_node_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned());
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let schema = SchemaManager::new(&db);
    for table in [
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
    ] {
        assert!(
            schema.has_column(table, "hlc_uuid").await.unwrap(),
            "{table}"
        );
        for column in ["created_at", "updated_at"].into_iter().flat_map(|prefix| {
            ["ts", "ver", "nid"].map(move |suffix| format!("{prefix}_hlc_{suffix}"))
        }) {
            assert!(
                !schema.has_column(table, &column).await.unwrap(),
                "{table}.{column}"
            );
        }
    }
}

#[tokio::test]
async fn v1_database_upgrades_without_bookkeeping_columns() {
    use sea_orm_migration::sea_orm::{ConnectionTrait, Statement};
    let _ = migration::initialize_node_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned());
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let steps = Migrator::migrations()
        .iter()
        .position(|m| m.name() == "m20250410_000025_add_hlc_columns")
        .unwrap() as u32;
    Migrator::up(&db, Some(steps)).await.unwrap();
    for sql in [
        "INSERT INTO playlists(id,name,\"group\",created_at,updated_at) VALUES(200,'list','g','2020','2020')",
        "INSERT INTO mixes(id,name,\"group\",locked,scriptlet_mode,created_at,updated_at) VALUES(201,'mix','g',0,0,'2020','2020')",
        "INSERT INTO mix_queries(id,mix_id,operator,parameter,\"group\",created_at,updated_at) VALUES(202,201,'lib::directory.deep','/music',0,'2020','2020')",
        "INSERT INTO media_cover_art(id,file_hash,binary,primary_color) VALUES(203,'crc',x'010203',0)",
        "INSERT INTO media_files(id,file_name,directory,extension,file_hash,last_modified,cover_art_id,sample_rate,duration) VALUES(204,'a.ogg','/music','ogg','hash','2020',203,44100,12)",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
    migration::up_with_actor(&db, Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"), None)
        .await
        .unwrap();
    for (table, id) in [
        ("playlists", 200),
        ("mixes", 201),
        ("mix_queries", 202),
        ("media_cover_art", 203),
        ("media_files", 204),
    ] {
        let sql = format!("SELECT id FROM {table} WHERE id={id}");
        assert!(
            db.query_one_raw(Statement::from_string(db.get_database_backend(), sql))
                .await
                .unwrap()
                .is_some()
        );
    }
    for table in [
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
    ] {
        for column in ["created_at", "updated_at"].into_iter().flat_map(|prefix| {
            ["ts", "ver", "nid"].map(move |suffix| format!("{prefix}_hlc_{suffix}"))
        }) {
            assert!(
                !SchemaManager::new(&db)
                    .has_column(table, &column)
                    .await
                    .unwrap()
            );
        }
    }
}
