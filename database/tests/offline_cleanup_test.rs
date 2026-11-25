use database::test_fixtures::{causal_library, fixtures};

use anyhow::Result;
use database::{
    actions::{
        downloads::{CleanupReport, cleanup_unselected_downloads, set_download_rule},
        metadata::scan_audio_file,
    },
    connection::RecommendationDbConnection,
    entities::{media_files, sync_download_rules},
    sync::{SeaOrmSyncStore, model::Collection},
};
use fsio::FsIo;
use sea_orm::{ActiveModelTrait, Database, EntityTrait, Set};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs as std_fs;
use std::path::Path;
use uuid::Uuid;

async fn cleanup(library: &causal_library::SqlLibrary, root: &Path) -> Result<CleanupReport> {
    cleanup_unselected_downloads(
        &library.db,
        &RecommendationDbConnection::new(),
        &FsIo::new(),
        root,
        &library.sync_store.record_store(),
        &[],
    )
    .await
}

#[tokio::test]
async fn disabling_the_last_rule_before_any_download_is_an_empty_cleanup() -> Result<()> {
    let library = causal_library::SqlLibrary::new(Uuid::new_v4(), Uuid::new_v4()).await?;
    library
        .peer
        .write_local(
            causal_library::key(Collection::Tracks, Uuid::new_v4()),
            Some(causal_library::track()),
        )
        .await?;
    let db = &library.db;
    let track = media_files::Entity::find().one(db).await?.unwrap();
    assert!(!track.local_available);
    let root = library.root.path();
    let unrelated = root.join("notes.txt");
    fs::write(&unrelated, b"keep")?;
    let query = vec![("lib::track".to_owned(), track.id.to_string())];
    let rule = set_download_rule(db, query.clone(), None, true).await?;
    set_download_rule(db, query, None, false).await?;

    let report = cleanup(&library, root).await?;
    assert_eq!((report.removed, report.delete_failures), (0, 0));
    assert!(!root.join("Rune Sync").exists());
    assert_eq!(fs::read(unrelated)?, b"keep");
    assert_eq!(
        media_files::Entity::find_by_id(track.id).one(db).await?,
        Some(track)
    );
    assert!(
        !sync_download_rules::Entity::find_by_id(rule)
            .one(db)
            .await?
            .unwrap()
            .enabled
    );
    Ok(())
}

#[tokio::test]
async fn a_missing_directory_with_a_recorded_download_is_reported() -> Result<()> {
    let library = causal_library::SqlLibrary::new(Uuid::new_v4(), Uuid::new_v4()).await?;
    let file = causal_library::local_file(&library.db, 1, Uuid::new_v4(), "track.ogg").await?;
    let mut row: media_files::ActiveModel = file.into();
    row.directory = Set("Rune Sync".into());
    let stored = row.update(&library.db).await?;
    assert!(cleanup(&library, library.root.path()).await.is_err());
    assert_eq!(
        media_files::Entity::find_by_id(stored.id)
            .one(&library.db)
            .await?,
        Some(stored)
    );
    Ok(())
}

#[tokio::test]
async fn cleanup_reports_invalid_directories() -> Result<()> {
    let library = causal_library::SqlLibrary::new(Uuid::new_v4(), Uuid::new_v4()).await?;
    let root = library.root.path();
    let managed = root.join("Rune Sync");
    fs::write(&managed, b"keep")?;
    for invalid_root in [
        root.join("missing library"),
        managed.clone(),
        root.to_owned(),
    ] {
        assert!(
            cleanup(&library, &invalid_root).await.is_err(),
            "cleanup should reject {}",
            invalid_root.display()
        );
    }
    assert_eq!(fs::read(managed)?, b"keep");
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn cleanup_preserves_an_unresolved_managed_directory_entry() -> Result<()> {
    let library = causal_library::SqlLibrary::new(Uuid::new_v4(), Uuid::new_v4()).await?;
    let root = library.root.path();
    let entry = root.join("Rune Sync");
    std_fs::symlink(root.join("missing"), &entry)?;
    assert!(cleanup(&library, root).await.is_err());
    assert!(fs::symlink_metadata(entry)?.is_symlink());
    Ok(())
}

#[tokio::test]
async fn cleanup_removes_only_unselected_managed_files() -> Result<()> {
    let node = Uuid::new_v4();
    let db = Database::connect("sqlite::memory:").await?;
    migration::up_with_actor(&db, Some(&node.to_string()), None).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let root = tempfile::tempdir()?;
    let fs = FsIo::new();
    let managed = root.path().join("Rune Sync");
    fs::create_dir_all(&managed)?;
    let mut ids = Vec::new();
    for (index, directory) in [managed.as_path(), managed.as_path(), root.path()]
        .iter()
        .enumerate()
    {
        let path = root.path().join(format!("{index}.ogg"));
        fs::copy(fixtures::audio_fixture(), &path)?;
        let id = scan_audio_file(&fs, &db, &node.to_string(), root.path(), &path, &store).await?;
        if index < 2 {
            fs::rename(&path, directory.join(format!("{index}.ogg")))?;
        }
        let mut row: media_files::ActiveModel = media_files::Entity::find_by_id(id)
            .one(&db)
            .await?
            .unwrap()
            .into();
        row.directory = Set(if index < 2 {
            "Rune Sync".into()
        } else {
            String::new()
        });
        row.file_name = Set(format!("{index}.ogg"));
        row.update(&db).await?;
        ids.push(id);
    }
    set_download_rule(
        &db,
        vec![("lib::track".into(), ids[0].to_string())],
        None,
        true,
    )
    .await?;
    cleanup_unselected_downloads(
        &db,
        &RecommendationDbConnection::new(),
        &fs,
        root.path(),
        &store,
        &[],
    )
    .await?;
    assert!(managed.join("0.ogg").exists());
    assert!(!managed.join("1.ogg").exists());
    assert!(root.path().join("2.ogg").exists());
    let row = media_files::Entity::find_by_id(ids[1])
        .one(&db)
        .await?
        .unwrap();
    assert!(!row.local_available && row.directory.is_empty() && row.file_name.is_empty());
    Ok(())
}
