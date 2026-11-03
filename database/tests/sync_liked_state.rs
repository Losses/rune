use anyhow::Result;
use database::{
    actions::{
        metadata::scan_audio_library,
        stats::{get_liked, increase_played_through, increase_skipped, set_liked},
    },
    entities::media_files,
    sync::LibrarySyncStore,
};
use fsio::FsIo;
use migration::{Migrator, MigratorTrait};
use sea_orm::{Database, EntityTrait};
use std::{path::Path, sync::Arc};
use uuid::Uuid;

async fn exercise_duplicate(played: bool) -> Result<()> {
    let root = tempfile::tempdir()?;
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/startup_0.ogg");
    std::fs::copy(&fixture, root.path().join("first.ogg"))?;
    let db = Database::connect("sqlite::memory:").await?;
    let node = Uuid::new_v4();
    let _ = migration::initialize_node_id(node.to_string());
    Migrator::up(&db, None).await?;
    let fs = Arc::new(FsIo::new());
    let store = LibrarySyncStore::open(db.clone(), node, None).await?;
    scan_audio_library(
        &fs,
        &db,
        &node.to_string(),
        root.path(),
        false,
        false,
        |_| {},
        None,
        &store.record_store(),
        database::actions::metadata::MissingHostFilePolicy::RemoveEverywhere,
    )
    .await?;
    let first = media_files::Entity::find().all(&db).await?.pop().unwrap();
    set_liked(&db, &store.record_store(), first.id, true).await?;
    store.head().await?;
    assert!(get_liked(&db, first.id).await?);

    std::fs::copy(&fixture, root.path().join("second.ogg"))?;
    scan_audio_library(
        &fs,
        &db,
        &node.to_string(),
        root.path(),
        false,
        false,
        |_| {},
        None,
        &store.record_store(),
        database::actions::metadata::MissingHostFilePolicy::RemoveEverywhere,
    )
    .await?;
    let second = media_files::Entity::find()
        .all(&db)
        .await?
        .into_iter()
        .find(|f| f.file_name == "second.ogg")
        .unwrap();
    store.head().await?;
    assert!(
        get_liked(&db, first.id).await?,
        "adding the copy must preserve the original like"
    );
    if played {
        increase_played_through(&db, &store.record_store(), second.id, node).await?;
    } else {
        increase_skipped(&db, &store.record_store(), second.id, node).await?;
    }
    store.head().await?;
    assert!(
        get_liked(&db, first.id).await?,
        "recording playback on an identical copy must preserve the existing like"
    );
    assert!(
        !get_liked(&db, second.id).await?,
        "a separate file must not inherit the original like"
    );
    Ok(())
}

#[tokio::test]
async fn playing_new_identical_copy_preserves_like() -> Result<()> {
    exercise_duplicate(true).await
}

#[tokio::test]
async fn skipping_new_identical_copy_preserves_like() -> Result<()> {
    exercise_duplicate(false).await
}
