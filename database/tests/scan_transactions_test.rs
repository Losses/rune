#[path = "support/fixtures.rs"]
mod fixtures;
use analysis::utils::computing_device::ComputingDevice;
use anyhow::Result;
use database::sync::SeaOrmSyncStore;
use database::{
    actions::{analysis::analysis_audio_library, metadata::sync_file_descriptions},
    connection::initialize_db,
    entities::media_files,
};
use fsio::FsIo;
use metadata::describe::describe_file;
use sea_orm::sea_query::Alias;
use sea_orm::sea_query::Expr;
use sea_orm::sea_query::Query;
use sea_orm::{ConnectionTrait, Database, DbBackend, EntityTrait, PaginatorTrait, Statement};
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn failed_metadata_insert_rolls_back_file() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    initialize_db(&db, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").await?;
    let store = SeaOrmSyncStore::create_empty(
        db.clone(),
        uuid::Uuid::from_u128(2),
        uuid::Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")?,
    )
    .await?;
    db.execute_raw(Statement::from_string(DbBackend::Sqlite,
        "CREATE TRIGGER reject_metadata BEFORE INSERT ON media_metadata BEGIN SELECT RAISE(ABORT, 'test failure'); END;".to_owned())).await?;
    let fs = FsIo::new();
    let root = fixtures::asset_directory().canonicalize()?;
    let node = fs.canonicalize(&root.join("startup_0.ogg"))?;
    let mut descriptions = vec![Some(describe_file(&fs, &node, &Some(root))?)];
    assert!(
        sync_file_descriptions(
            &fs,
            &db,
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            &mut descriptions,
            false,
            &store
        )
        .await
        .is_err()
    );
    assert_eq!(media_files::Entity::find().count(&db).await?, 0);
    db.execute_raw(Statement::from_string(
        DbBackend::Sqlite,
        "DROP TRIGGER reject_metadata".to_owned(),
    ))
    .await?;
    sync_file_descriptions(
        &fs,
        &db,
        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
        &mut descriptions,
        false,
        &store,
    )
    .await?;
    assert_eq!(media_files::Entity::find().count(&db).await?, 1);
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let processed = analysis_audio_library(
        Arc::new(fs),
        &db,
        Path::new("."),
        1,
        ComputingDevice::Cpu,
        |_, _| {},
        Some(cancelled),
    )
    .await?;
    assert_eq!(
        processed, 0,
        "Cancellation must report completed work, not queue size"
    );
    Ok(())
}

#[tokio::test]
async fn failed_file_does_not_rollback_neighbors_or_stop_the_batch() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    initialize_db(&db, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").await?;
    let store = SeaOrmSyncStore::create_empty(
        db.clone(),
        uuid::Uuid::from_u128(2),
        uuid::Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")?,
    )
    .await?;
    db.execute_raw(Statement::from_string(DbBackend::Sqlite,
        "CREATE TRIGGER reject_one BEFORE INSERT ON media_metadata WHEN (SELECT file_name FROM media_files WHERE id=NEW.file_id)='bad.ogg' BEGIN SELECT RAISE(ABORT, 'test failure'); END;".to_owned())).await?;
    let dir = tempfile::tempdir()?;
    let source = fixtures::audio_fixture();
    let fs = FsIo::new();
    let mut descriptions = Vec::new();
    for name in ["before.ogg", "bad.ogg", "after.ogg"] {
        let path = dir.path().join(name);
        std::fs::copy(&source, &path)?;
        descriptions.push(Some(describe_file(
            &fs,
            &fs.canonicalize(&path)?,
            &Some(dir.path().to_path_buf()),
        )?));
    }
    assert!(
        sync_file_descriptions(
            &fs,
            &db,
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            &mut descriptions,
            false,
            &store
        )
        .await
        .is_err()
    );
    let mut names: Vec<_> = media_files::Entity::find()
        .all(&db)
        .await?
        .into_iter()
        .map(|file| file.file_name)
        .collect();
    names.sort();
    assert_eq!(names, ["after.ogg", "before.ogg"]);
    let orphan = db.query_one_raw(Statement::from_string(DbBackend::Sqlite,
        "SELECT COUNT(*) AS n FROM media_metadata WHERE file_id NOT IN (SELECT id FROM media_files)".to_owned())).await?.unwrap();
    assert_eq!(orphan.try_get::<i64>("", "n")?, 0);
    Ok(())
}

#[tokio::test]
async fn lookup_indexes_upgrade_existing_database_and_survive_reopen() -> Result<()> {
    use migration::{Migrator, MigratorTrait};
    use sea_orm::ExprTrait;
    let dir = tempfile::tempdir()?;
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("library.db").display()
    );
    let db = Database::connect(&url).await?;
    // Construct the previous schema directly; UUID conversion cannot be reversed.
    let _ = migration::initialize_node_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned());
    let before_lookup_indexes = Migrator::migrations()
        .iter()
        .position(|m| m.name() == "m20261031_000029_add_library_lookup_indexes")
        .expect("lookup index migration exists");
    Migrator::up(&db, Some(before_lookup_indexes as u32)).await?;
    db.execute_raw(Statement::from_string(DbBackend::Sqlite,
        "INSERT INTO media_files (file_name,directory,extension,file_hash,last_modified,sample_rate,duration) VALUES ('track.ogg','','ogg','abc','1',44100,10)".to_owned())).await?;
    // Verify the original lookup indexes immediately after their upgrade.
    Migrator::up(&db, Some(1)).await?;
    for (sql, index) in [
        (
            "SELECT id FROM media_files WHERE directory='' AND file_name='track.ogg'",
            "idx_media_files_path",
        ),
        (
            "SELECT id FROM media_metadata WHERE file_id=1",
            "idx_media_metadata_file_id",
        ),
        (
            "SELECT id FROM media_analysis WHERE file_id=1",
            "idx_media_analysis_file_id",
        ),
    ] {
        let plan = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("EXPLAIN QUERY PLAN {sql}"),
            ))
            .await?;
        assert!(
            plan.iter()
                .any(|row| row.try_get::<String>("", "detail").unwrap().contains(index)),
            "Missing index in upgrade query plan: {index}"
        );
    }
    initialize_db(&db, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").await?;
    db.close().await?;
    let db = Database::connect(&url).await?;
    initialize_db(&db, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").await?;
    assert_eq!(media_files::Entity::find().count(&db).await?, 1);
    for name in [
        "idx_media_files_path",
        "idx_media_metadata_file_id",
        "idx_media_analysis_file_id",
    ] {
        let q = Query::select()
            .expr(Expr::val(1))
            .from(Alias::new("sqlite_master"))
            .and_where(Expr::col(Alias::new("type")).eq("index"))
            .and_where(Expr::col(Alias::new("name")).eq(name))
            .to_owned();
        assert!(
            db.query_one(&q).await?.is_some(),
            "Index lost during conversion: {name}"
        );
    }
    for (sql, index) in [
        (
            "SELECT id FROM media_files WHERE directory='' AND file_name='track.ogg'",
            "idx_media_files_path",
        ),
        ("SELECT id FROM media_metadata WHERE file_id=1", "file_id=?"),
        (
            "SELECT id FROM media_analysis WHERE file_id=1",
            "idx_media_analysis_file_id",
        ),
    ] {
        let plan = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("EXPLAIN QUERY PLAN {sql}"),
            ))
            .await?;
        assert!(
            plan.iter()
                .any(|row| row.try_get::<String>("", "detail").unwrap().contains(index)),
            "Missing index in query plan: {index}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn cover_timestamps_preserve_shared_and_absent_cover_behavior_across_chunks() -> Result<()> {
    use database::actions::cover_art::{
        ensure_magic_cover_art, get_files_with_cover_art_timestamps,
    };
    let db = Database::connect("sqlite::memory:").await?;
    initialize_db(&db, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").await?;
    let cover = ensure_magic_cover_art(&db).await?;
    for (id, cover_id) in [
        (1, Some(cover.hlc_uuid.clone())),
        (2, Some(cover.hlc_uuid.clone())),
        (3, None),
    ] {
        db.execute_raw(Statement::from_sql_and_values(DbBackend::Sqlite,
            "INSERT INTO media_files (id,file_name,directory,extension,file_hash,last_modified,sample_rate,duration,cover_art_uuid,hlc_uuid,cover_art_checked_at) VALUES (?,?,'','ogg','abc','123',44100,10,?,?,?)",
            [id.into(), format!("{id}.ogg").into(), cover_id.clone().into(), uuid::Uuid::from_u128(id as u128).to_string().into(), cover_id.map(|_| "2020-01-01T00:00:00Z".to_owned()).into()])).await?;
    }
    let mut ids = vec![1; 500];
    ids.extend([2, 3, 999]);
    let timestamps = get_files_with_cover_art_timestamps(&db, ids).await?;
    assert_eq!(timestamps.len(), 2);
    for id in [1, 2] {
        assert_eq!(
            timestamps[&id],
            ("123".to_owned(), "2020-01-01T00:00:00Z".to_owned())
        );
    }
    assert!(
        get_files_with_cover_art_timestamps(&db, vec![])
            .await?
            .is_empty()
    );
    Ok(())
}
