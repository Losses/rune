use anyhow::{Context, Result};
use database::test_fixtures::causal_library as support;
use database::{
    actions::{
        downloads::{DownloadReport, prepare_download, remove_download_rule, set_download_rule},
        file::{attach_download, hash_file, is_materialized},
        mixes::{create_mix, remove_mix},
    },
    connection::RecommendationDbConnection,
    entities::{media_files, sync_download_jobs, sync_download_rules},
    sync::{
        LibrarySyncStore,
        model::{BlobRef, Collection, ObjectKey, RuneModel},
    },
};
use fsio::FsIo;
use http_request::sync::{MetadataPeer, router};
use hub::server::sync::downloads::download_requested_audio;
use reqwest::Client;
use sea_orm::{
    ColumnTrait, Database, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
};
use std::{fs, path::PathBuf, sync::Arc};
use support::fixtures;
use tempfile::TempDir;
use tokio::{net::TcpListener, task::JoinHandle};
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
    fs::copy(&fixture, &dest)?;
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
    fs::copy(&fixture, &second_path)?;
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
    let fixture = OfflineJobs::new().await?;
    fixture.reserve("Pending").await?;
    let id = set_download_rule(
        &fixture.db,
        vec![("lib::all".into(), "true".into())],
        None,
        true,
    )
    .await?;
    remove_download_rule(&fixture.db, &id).await?;
    let report = fixture.run(&fixtures::unreachable_peer()).await?;
    assert_eq!(report.requested, 0);
    assert_eq!(
        sync_download_jobs::Entity::find()
            .count(&fixture.db)
            .await?,
        0,
        "the last selecting rule has been removed"
    );
    Ok(())
}

struct OfflineJobs {
    root: TempDir,
    db: DatabaseConnection,
    fsio: Arc<FsIo>,
    store: LibrarySyncStore,
    node_id: Uuid,
    blob: BlobRef,
}

impl OfflineJobs {
    async fn new() -> Result<Self> {
        let root = tempfile::tempdir()?;
        let node_id = Uuid::new_v4();
        let db = fixtures::disk_database(&root.path().join("library.db"), node_id).await?;
        let fsio = Arc::new(FsIo::new());
        let store = fixtures::open_library(db.clone(), node_id, Some(Uuid::new_v4())).await?;
        let blob = hash_file(&fsio, &fixtures::audio_fixture())?;
        Ok(Self {
            root,
            db,
            fsio,
            store,
            node_id,
            blob,
        })
    }

    async fn reserve(&self, title: &str) -> Result<(Uuid, PathBuf)> {
        let track = Uuid::new_v4();
        self.store
            .write_local(
                ObjectKey {
                    collection: Collection::Tracks,
                    id: track,
                },
                Some(support::audio(title, &self.blob)),
            )
            .await?;
        let path = prepare_download(
            &self.db,
            &self.fsio,
            self.root.path(),
            track,
            &self.blob,
            "ogg",
        )
        .await?;
        Ok((track, path))
    }

    async fn file_id(&self, track: Uuid) -> Result<i32> {
        Ok(media_files::Entity::find()
            .filter(media_files::Column::SourceUuid.eq(track.to_string()))
            .one(&self.db)
            .await?
            .context("track projected into the catalog")?
            .id)
    }

    async fn metadata_peer(&self) -> Result<(MetadataPeer<RuneModel>, JoinHandle<()>)> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let router = router::<RuneModel>(Arc::new(self.store.endpoint()));
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.expect("serve metadata");
        });
        Ok((
            MetadataPeer::new(format!("http://{address}"), Client::new()),
            server,
        ))
    }

    async fn run(&self, peer: &MetadataPeer<RuneModel>) -> Result<DownloadReport> {
        download_requested_audio(
            self.db.clone(),
            &RecommendationDbConnection::new(),
            self.fsio.clone(),
            self.root.path().to_path_buf(),
            self.node_id,
            peer,
            Arc::new(|| {}),
        )
        .await
    }
}

#[tokio::test]
async fn overlap_keeps_pending_work_and_disabling_last_rule_preserves_completed_audio() -> Result<()>
{
    let fixture = OfflineJobs::new().await?;
    let (kept, _) = fixture.reserve("Still selected").await?;
    fixture.reserve("No longer selected").await?;
    let (completed, destination) = fixture.reserve("Completed").await?;
    let audio = fs::read(fixtures::audio_fixture())?;
    fs::write(&destination, &audio)?;
    attach_download(
        &fixture.db,
        &fixture.store.record_store(),
        &fixture.fsio,
        fixture.root.path(),
        completed,
        &fixture.blob,
        &destination,
    )
    .await?;

    let all = set_download_rule(
        &fixture.db,
        vec![("lib::all".into(), "true".into())],
        None,
        true,
    )
    .await?;
    let one_track = vec![(
        "lib::track".into(),
        fixture.file_id(kept).await?.to_string(),
    )];
    set_download_rule(&fixture.db, one_track.clone(), None, true).await?;
    remove_download_rule(&fixture.db, &all).await?;

    let (peer, server) = fixture.metadata_peer().await?;
    let report = fixture.run(&peer).await?;
    assert_eq!(report.requested, 1);
    assert_eq!(report.pending, 1);
    let pending = sync_download_jobs::Entity::find().all(&fixture.db).await?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].file_uuid, kept.to_string());

    set_download_rule(&fixture.db, one_track, None, false).await?;
    let report = fixture.run(&fixtures::unreachable_peer()).await?;
    assert_eq!(report.requested, 0);
    assert_eq!(
        sync_download_jobs::Entity::find()
            .count(&fixture.db)
            .await?,
        0
    );
    assert_eq!(fs::read(destination)?, audio);
    assert!(
        is_materialized(
            &fixture.db,
            &fixture.fsio,
            fixture.root.path(),
            fixture.file_id(completed).await?,
        )
        .await?
    );
    server.abort();
    Ok(())
}

#[tokio::test]
async fn unresolved_saved_mix_keeps_pending_work() -> Result<()> {
    let fixture = OfflineJobs::new().await?;
    fixture.reserve("Selection unresolved").await?;
    let mix = create_mix(
        &fixture.db,
        &fixture.store.record_store(),
        "Offline mix".into(),
        "Examples".into(),
        None,
        false,
        vec![("lib::all".into(), "true".into())],
    )
    .await?;
    set_download_rule(&fixture.db, Vec::new(), Some(mix.id), true).await?;
    remove_mix(&fixture.db, &fixture.store.record_store(), mix.id).await?;

    let (peer, server) = fixture.metadata_peer().await?;
    let report = fixture.run(&peer).await?;
    assert_eq!(report.failed_rules, 1);
    assert_eq!(report.requested, 0);
    assert_eq!(
        sync_download_jobs::Entity::find()
            .count(&fixture.db)
            .await?,
        1
    );
    server.abort();
    Ok(())
}
