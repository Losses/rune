use anyhow::Result;
use database::{
    actions::{
        downloads::set_download_rule,
        file::attach_download,
        metadata::{MissingHostFilePolicy, ScanOptions},
    },
    connection::RecommendationDbConnection,
    entities::{media_files, sync_download_jobs, sync_download_rules},
    sync::model::{Collection, MetadataTag, ObjectKey, Payload, Track},
};
use fsio::FsIo;
use hub::server::sync::downloads::download_requested_audio;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ColumnTrait, Database, EntityTrait, PaginatorTrait, QueryFilter};
use std::sync::Arc;
use uuid::Uuid;

#[tokio::test]
async fn rules_persist_and_empty_rules_do_not_contact_peer() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("rules.db");
    std::fs::File::create(&db_path)?;
    let url = format!("sqlite://{}", db_path.display());
    let db = Database::connect(&url).await?;
    let _ = migration::initialize_node_id(Uuid::new_v4().to_string());
    Migrator::up(&db, None).await?;
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
        &http_request::sync::MetadataPeer::new("http://127.0.0.1:1", reqwest::Client::new()),
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
    use database::{
        actions::metadata::scan_audio_library,
        actions::{downloads::prepare_download, file::hash_file},
        sync::LibrarySyncStore,
    };
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("library.db");
    std::fs::File::create(&path)?;
    let url = format!("sqlite://{}", path.display());
    let db = Database::connect(&url).await?;
    let _ = migration::initialize_node_id(Uuid::new_v4().to_string());
    Migrator::up(&db, None).await?;
    let fs = Arc::new(FsIo::new());
    let node = Uuid::new_v4();
    let library = Uuid::new_v4();
    let track = Uuid::new_v4();
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");
    let blob = hash_file(&fs, &fixture)?;
    let store = LibrarySyncStore::open(db.clone(), node, Some(library)).await?;
    store
        .write_local(
            ObjectKey {
                collection: Collection::Tracks,
                id: track,
            },
            Some(Payload::Track(Track {
                extension: "ogg".into(),
                content_hash: Some(blob.digest.clone()),
                content_size: Some(blob.size.try_into()?),
                duration: "0".to_owned().try_into().map_err(anyhow::Error::msg)?,
                sample_rate: 0,
                cover_art_id: None,
                metadata: vec![MetadataTag {
                    id: Uuid::new_v4(),
                    position: 0,
                    key: "track_title".into(),
                    value: "Fixture".into(),
                }],
            })),
        )
        .await?;
    let dest = prepare_download(&db, &fs, dir.path(), track, &blob, "ogg").await?;
    std::fs::copy(&fixture, &dest)?;
    scan_audio_library(
        &fs,
        &db,
        &node.to_string(),
        dir.path(),
        ScanOptions {
            cleanup: false,
            force: false,
            missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
            cancel_token: None,
        },
        |_| {},
        &store.record_store(),
    )
    .await?;
    store.head().await?;
    drop(store);
    db.close().await?;
    let db = Database::connect(&url).await?;
    assert_eq!(media_files::Entity::find().count(&db).await?, 1);
    let store = LibrarySyncStore::open(db.clone(), node, Some(library)).await?;
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
            Some(Payload::Track(Track {
                extension: "ogg".into(),
                content_hash: Some(blob.digest.clone()),
                content_size: Some(blob.size.try_into()?),
                duration: "0".to_owned().try_into().map_err(anyhow::Error::msg)?,
                sample_rate: 0,
                cover_art_id: None,
                metadata: vec![MetadataTag {
                    id: Uuid::new_v4(),
                    position: 0,
                    key: "track_title".into(),
                    value: "Second".into(),
                }],
            })),
        )
        .await?;
    let second_path = prepare_download(&db, &fs, dir.path(), second, &blob, "ogg").await?;
    assert_ne!(dest, second_path);
    std::fs::copy(&fixture, &second_path)?;
    scan_audio_library(
        &fs,
        &db,
        &node.to_string(),
        dir.path(),
        ScanOptions {
            cleanup: false,
            force: false,
            missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
            cancel_token: None,
        },
        |_| {},
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
            .filter(media_files::Column::HlcUuid.is_in([track.to_string(), second.to_string()]))
            .count(&db)
            .await?,
        2
    );
    Ok(())
}
