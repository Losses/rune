use anyhow::Result;
use database::{
    actions::{metadata::sync_file_descriptions, mixes, playlists},
    sync::LibrarySyncStore,
};
use fsio::FsIo;
use metadata::describe::describe_file;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ConnectionTrait, Database, Statement};
use uuid::Uuid;

#[tokio::test]
async fn alpha_database_with_bookkeeping_columns_still_works() -> Result<()> {
    let actor = Uuid::new_v4();
    let _ = migration::initialize_node_id(actor.to_string());
    let db = Database::connect("sqlite::memory:").await?;
    let steps = Migrator::migrations()
        .iter()
        .position(|m| m.name() == "m20250410_000025_add_hlc_columns")
        .unwrap() as u32;
    Migrator::up(&db, Some(steps)).await?;
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
        for prefix in ["created_at", "updated_at"] {
            for (suffix, def) in [
                ("ts", "TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z'"),
                ("ver", "INTEGER NOT NULL DEFAULT 0"),
                ("nid", "TEXT NOT NULL DEFAULT ''"),
            ] {
                db.execute_unprepared(&format!(
                    "ALTER TABLE {table} ADD COLUMN {prefix}_hlc_{suffix} {def}"
                ))
                .await?;
            }
        }
    }
    db.execute_unprepared(&format!("INSERT INTO media_cover_art(id,file_hash,binary,primary_color,{}_hlc_{}) VALUES(203,'crc',x'010203',0,'2023-01-02 03:04:05')", "updated_at", "ts")).await?;
    for sql in [
        "INSERT INTO playlists(id,name,\"group\",created_at,updated_at) VALUES(200,'list','g','2020','2020')",
        "INSERT INTO mixes(id,name,\"group\",locked,scriptlet_mode,created_at,updated_at) VALUES(201,'mix','g',0,0,'2020','2020')",
        "INSERT INTO mix_queries(id,mix_id,operator,parameter,\"group\",created_at,updated_at) VALUES(202,201,'lib::directory.deep','/music',0,'2020','2020')",
        "INSERT INTO media_files(id,file_name,directory,extension,file_hash,last_modified,cover_art_id,sample_rate,duration) VALUES(204,'a.ogg','/music','ogg','hash','2020',203,44100,12)",
    ] {
        db.execute_unprepared(sql).await?;
    }
    migration::up_with_actor(&db, Some(&actor.to_string()), None).await?;
    let row = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            "SELECT cover_art_checked_at FROM media_files WHERE id=204".to_owned(),
        ))
        .await?
        .unwrap();
    assert_eq!(
        row.try_get::<String>("", "cover_art_checked_at")?,
        "2023-01-02 03:04:05"
    );
    let library = LibrarySyncStore::open(db.clone(), actor, None).await?;
    library.import_basic().await?;
    let store = library.record_store();
    let list = playlists::create_playlist(&db, &store, "new".into(), "g".into()).await?;
    playlists::add_item_to_playlist(&db, &store, list.id, 204, None).await?;
    mixes::create_mix(
        &db,
        &store,
        "new mix".into(),
        "g".into(),
        false,
        0,
        false,
        vec![("lib::directory.deep".into(), "/music".into())],
    )
    .await?;
    let fs = FsIo::new();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../assets")
        .canonicalize()?;
    let audio = fs.canonicalize(&root.join("startup_0.ogg"))?;
    let mut descriptions = vec![Some(describe_file(&fs, &audio, &Some(root))?)];
    sync_file_descriptions(
        &fs,
        &db,
        &actor.to_string(),
        &mut descriptions,
        false,
        &store,
    )
    .await?;
    Ok(())
}
