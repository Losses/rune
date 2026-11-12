#[path = "support/causal_library.rs"]
mod support;
use anyhow::Result;
use database::{
    actions::{
        downloads::{prepare_download, remove_download_rule, set_download_rule},
        file::attach_download,
    },
    connection::RecommendationDbConnection,
    entities::{media_files, sync_download_jobs, sync_download_rules},
    sync::model::{Collection, ObjectKey},
};
use fsio::FsIo;
use hub::server::sync::downloads::download_requested_audio;
use sea_orm::{ColumnTrait, Database, EntityTrait, PaginatorTrait, QueryFilter};
use std::sync::Arc;
use support::fixtures;
use uuid::Uuid;

#[tokio::test]
async fn rules_persist_and_empty_rules_do_not_contact_peer() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("rules.db");
    let url = format!("sqlite://{}", db_path.display());
    let db = fixtures::disk_database(&db_path, Uuid::new_v4()).await?;
    let queries = vec![("lib::track".to_owned(), "42".to_owned())];
    let id = set_download_rule(&db, queries.clone(), None, true).await?;
    assert_eq!(
        id,
        set_download_rule(&db, queries.clone(), None, true).await?
    );
    db.close().await?;
    let db = Database::connect(&url).await?;
    assert!(
        sync_download_rules::Entity::find()
            .one(&db)
            .await?
            .unwrap()
            .enabled
    );
    assert_eq!(id, set_download_rule(&db, queries, None, false).await?);
    let report = download_requested_audio(
        db,
        &RecommendationDbConnection::new(),
        Arc::new(FsIo::new()),
        dir.path().to_path_buf(),
        Uuid::new_v4(),
        &fixtures::unreachable_peer(),
        Arc::new(|| {}),
    )
    .await?;
    assert_eq!(report.requested, 0);
    assert_eq!(report.downloaded, 0);
    assert_eq!(report.pending, 0);
    Ok(())
}

#[tokio::test]
async fn interrupted_import_resumes_without_new_track_identity() -> Result<()> {
    use database::actions::{downloads::prepare_download, file::hash_file};
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("library.db");
    let url = format!("sqlite://{}", path.display());
    let db = fixtures::disk_database(&path, Uuid::new_v4()).await?;
    let fs = Arc::new(FsIo::new());
    let node = Uuid::new_v4();
    let library = Uuid::new_v4();
    let track = Uuid::new_v4();
    let fixture = fixtures::audio_fixture();
    let blob = hash_file(&fs, &fixture)?;
    let store = fixtures::open_library(db.clone(), node, Some(library)).await?;
    store
        .write_local(
            ObjectKey {
                collection: Collection::Tracks,
                id: track,
            },
            Some(support::audio("Fixture", &blob)),
        )
        .await?;
    let dest = prepare_download(&db, &fs, dir.path(), track, &blob, "ogg").await?;
    std::fs::copy(&fixture, &dest)?;
    fixtures::scan_library(
        &fs,
        &db,
        &node.to_string(),
        dir.path(),
        &store.record_store(),
    )
    .await?;
    store.head().await?;
    drop(store);
    db.close().await?;
    let db = Database::connect(&url).await?;
    assert_eq!(media_files::Entity::find().count(&db).await?, 1);
    let store = fixtures::open_library(db.clone(), node, Some(library)).await?;
    attach_download(
        &db,
        &store.record_store(),
        &fs,
        dir.path(),
        track,
        &blob,
        &dest,
    )
    .await?;
    assert_eq!(sync_download_jobs::Entity::find().count(&db).await?, 0);
    assert!(
        store
            .get(&ObjectKey {
                collection: Collection::Tracks,
                id: track
            })
            .await?
            .is_some()
    );
    let second = Uuid::new_v4();
    store
        .write_local(
            ObjectKey {
                collection: Collection::Tracks,
                id: second,
            },
            Some(support::audio("Second", &blob)),
        )
        .await?;
    let second_path = prepare_download(&db, &fs, dir.path(), second, &blob, "ogg").await?;
    assert_ne!(dest, second_path);
    std::fs::copy(&fixture, &second_path)?;
    fixtures::scan_library(
        &fs,
        &db,
        &node.to_string(),
        dir.path(),
        &store.record_store(),
    )
    .await?;
    attach_download(
        &db,
        &store.record_store(),
        &fs,
        dir.path(),
        second,
        &blob,
        &second_path,
    )
    .await?;
    assert_eq!(
        media_files::Entity::find()
            .filter(media_files::Column::SourceUuid.is_in([track.to_string(), second.to_string()]))
            .count(&db)
            .await?,
        2
    );
    Ok(())
}

#[tokio::test]
async fn removed_rule_does_not_download_stale_pending_job() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db = fixtures::disk_database(&dir.path().join("pending.db"), Uuid::new_v4()).await?;
    let fs = Arc::new(FsIo::new());
    let track = Uuid::new_v4();
    use database::actions::file::hash_file;
    let blob = hash_file(&fs, &fixtures::audio_fixture())?;
    let store = fixtures::open_library(db.clone(), Uuid::new_v4(), Some(Uuid::new_v4())).await?;
    store
        .write_local(
            ObjectKey {
                collection: Collection::Tracks,
                id: track,
            },
            Some(support::audio("Pending", &blob)),
        )
        .await?;
    let id = set_download_rule(&db, vec![("lib::all".into(), "true".into())], None, true).await?;
    prepare_download(&db, &fs, dir.path(), track, &blob, "ogg").await?;
    remove_download_rule(&db, &id).await?;
    let report = download_requested_audio(
        db.clone(),
        &RecommendationDbConnection::new(),
        fs,
        dir.path().to_path_buf(),
        Uuid::new_v4(),
        &fixtures::unreachable_peer(),
        Arc::new(|| {}),
    )
    .await?;
    assert_eq!(report.requested, 0);
    assert_eq!(sync_download_jobs::Entity::find().count(&db).await?, 1);
    Ok(())
}
