use anyhow::Result;
use database::{
    actions::metadata::sync_file_descriptions, connection::initialize_db, entities::media_files,
};
use fsio::FsIo;
use metadata::describe::describe_file;
use sea_orm::{ConnectionTrait, Database, DbBackend, EntityTrait, PaginatorTrait, Statement};

#[tokio::test]
async fn failed_metadata_insert_rolls_back_file() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    initialize_db(&db, "scan-test").await?;
    db.execute(Statement::from_string(DbBackend::Sqlite,
        "CREATE TRIGGER reject_metadata BEFORE INSERT ON media_metadata BEGIN SELECT RAISE(ABORT, 'test failure'); END;".to_owned())).await?;
    let fs = FsIo::new();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../assets")
        .canonicalize()?;
    let node = fs.canonicalize(&root.join("startup_0.ogg"))?;
    let mut descriptions = vec![Some(describe_file(&fs, &node, &Some(root))?)];
    assert!(
        sync_file_descriptions(&fs, &db, "scan-test", &mut descriptions, false)
            .await
            .is_err()
    );
    assert_eq!(media_files::Entity::find().count(&db).await?, 0);
    db.execute(Statement::from_string(
        DbBackend::Sqlite,
        "DROP TRIGGER reject_metadata".to_owned(),
    ))
    .await?;
    sync_file_descriptions(&fs, &db, "scan-test", &mut descriptions, false).await?;
    assert_eq!(media_files::Entity::find().count(&db).await?, 1);
    let cancelled = tokio_util::sync::CancellationToken::new();
    cancelled.cancel();
    let processed = database::actions::analysis::analysis_audio_library(
        std::sync::Arc::new(fs),
        &db,
        std::path::Path::new("."),
        "scan-test",
        1,
        analysis::utils::computing_device::ComputingDevice::Cpu,
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
    initialize_db(&db, "scan-test").await?;
    db.execute(Statement::from_string(DbBackend::Sqlite,
        "CREATE TRIGGER reject_one BEFORE INSERT ON media_metadata WHEN (SELECT file_name FROM media_files WHERE id=NEW.file_id)='bad.ogg' BEGIN SELECT RAISE(ABORT, 'test failure'); END;".to_owned())).await?;
    let dir = tempfile::tempdir()?;
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/startup_0.ogg");
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
        sync_file_descriptions(&fs, &db, "scan-test", &mut descriptions, false)
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
    let orphan = db.query_one(Statement::from_string(DbBackend::Sqlite,
        "SELECT COUNT(*) AS n FROM media_metadata WHERE file_id NOT IN (SELECT id FROM media_files)".to_owned())).await?.unwrap();
    assert_eq!(orphan.try_get::<i64>("", "n")?, 0);
    Ok(())
}

#[tokio::test]
async fn lookup_indexes_upgrade_existing_database_and_survive_reopen() -> Result<()> {
    use migration::{Migrator, MigratorTrait};
    let dir = tempfile::tempdir()?;
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("library.db").display()
    );
    let db = Database::connect(&url).await?;
    initialize_db(&db, "scan-test").await?;
    // Return to the previous release's schema, then exercise the upgrade path.
    let pending = Migrator::get_applied_migrations(&db)
        .await?
        .iter()
        .filter(|m| m.name() >= "m20261031_000029_add_library_lookup_indexes")
        .count() as u32;
    Migrator::down(&db, Some(pending)).await?;
    db.execute(Statement::from_string(DbBackend::Sqlite,
        "INSERT INTO media_files (file_name,directory,extension,file_hash,last_modified,sample_rate,duration) VALUES ('track.ogg','','ogg','abc','1',44100,10)".to_owned())).await?;
    initialize_db(&db, "scan-test").await?;
    db.close().await?;
    let db = Database::connect(&url).await?;
    initialize_db(&db, "scan-test").await?;
    assert_eq!(media_files::Entity::find().count(&db).await?, 1);
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
            .query_all(Statement::from_string(
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
    initialize_db(&db, "scan-test").await?;
    let cover = ensure_magic_cover_art(&db, "scan-test").await?;
    for (id, cover_id) in [(1, Some(cover.id)), (2, Some(cover.id)), (3, None)] {
        db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "INSERT INTO media_files (id,file_name,directory,extension,file_hash,last_modified,sample_rate,duration,cover_art_id) VALUES (?,?,'','ogg','abc','123',44100,10,?)",
            [id.into(), format!("{id}.ogg").into(), cover_id.into()])).await?;
    }
    let mut ids = vec![1; 500];
    ids.extend([2, 3, 999]);
    let timestamps = get_files_with_cover_art_timestamps(&db, ids).await?;
    assert_eq!(timestamps.len(), 2);
    for id in [1, 2] {
        assert_eq!(
            timestamps[&id],
            ("123".to_owned(), cover.updated_at_hlc_ts.clone())
        );
    }
    assert!(
        get_files_with_cover_art_timestamps(&db, vec![])
            .await?
            .is_empty()
    );
    Ok(())
}
