#![cfg(all(feature = "sync-crash-probe", unix))]

use anyhow::{Context, Result, ensure};
use database::sync::model::*;
use database::test_fixtures::fixtures;
use database::{
    actions::{
        downloads::set_download_rule,
        file::{
            AudioAvailability, audio_availability, blob_path_for_track, catalog_track,
            get_media_files, hash_file, is_materialized, materialized_path, resolve_local_id,
        },
    },
    connection::{RecommendationDbConnection, initialize_db},
    entities::{media_files, sync_download_jobs},
    sync::model::{RuneModel, Scope},
};
use fsio::FsIo;
use http_request::sync::MetadataPeer;
use hub::server::sync::{
    client::sync_library,
    downloads::download_requested_audio,
    http::{BlobContext, SharedLibrary, blob_router},
    library_router,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, PaginatorTrait,
    QueryFilter, Statement,
};
use std::fs::File;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};
use sync::{ReceiveStore, SyncModel};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use uuid::Uuid;

async fn open_replica(path: &Path) -> Result<DatabaseConnection> {
    use sea_orm::{
        SqlxSqliteConnector,
        sqlx::{
            SqlitePool,
            sqlite::{SqliteConnectOptions, SqliteSynchronous},
        },
    };
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .synchronous(SqliteSynchronous::Extra);
    Ok(SqlxSqliteConnector::from_sqlx_sqlite_pool(
        SqlitePool::connect_with(options).await?,
    ))
}
async fn revision_counter(db: &DatabaseConnection) -> Result<i64> {
    use sea_orm::sea_query::{Alias, Query};
    let query = Query::select()
        .column(Alias::new("revision_counter"))
        .from(Alias::new("sync_meta"))
        .to_owned();
    Ok(db
        .query_one(&query)
        .await?
        .context("missing metadata")?
        .try_get("", "revision_counter")?)
}

#[test]
fn import_child() -> Result<()> {
    let Ok(db_path) = std::env::var("RUNE_TEST_DB") else {
        return Ok(());
    };
    let root = PathBuf::from(std::env::var("RUNE_TEST_ROOT")?);
    let node = Uuid::parse_str(&std::env::var("RUNE_TEST_NODE")?)?;
    let server = std::env::var("RUNE_TEST_SERVER")?;
    Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(async move {
            let db = open_replica(Path::new(&db_path)).await?;
            let fsio = Arc::new(FsIo::new());
            let report = download_requested_audio(
                db,
                &RecommendationDbConnection::new(),
                fsio,
                root,
                node,
                &MetadataPeer::new(&server, reqwest::Client::new()),
                Arc::new(|| {}),
            )
            .await?;
            ensure!(
                report.downloaded == 1,
                "child did not download selected audio"
            );
            Ok(())
        })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn killed_imports_recover_across_publication_stages() -> Result<()> {
    use std::os::unix::process::ExitStatusExt;
    let source_root = tempfile::tempdir()?;
    let fsio = Arc::new(FsIo::new());
    let source_node = Uuid::new_v4();
    let source_db = fixtures::initialized_memory(source_node).await?;
    for index in 0..2 {
        let bytes = std::fs::read(fixtures::audio_fixture_at(index))?;
        std::fs::write(source_root.path().join(format!("source{index}.ogg")), bytes)?;
    }
    let source = fixtures::open_library(source_db.clone(), source_node, None).await?;
    fixtures::scan_library(
        &fsio,
        &source_db,
        &source_node.to_string(),
        source_root.path(),
        &source.record_store(),
    )
    .await?;
    let source_files = media_files::Entity::find().all(&source_db).await?;
    let first = source_files
        .iter()
        .find(|row| row.file_name == "source0.ogg")
        .context("missing selected source")?;
    let second = source_files
        .iter()
        .find(|row| row.file_name == "source1.ogg")
        .context("missing other source")?;

    source.import_existing(source_node).await?;
    let selected_track: Uuid = first.source_uuid.parse()?;
    let other_track: Uuid = second.source_uuid.parse()?;
    let source_identity = source.identity().await?;
    let checkpoint = sync::CheckpointKey {
        namespace: source_identity.namespace,
        source_replica: source_identity.replica,
        source_generation: source_identity.generation,
        scope_id: sync::scope_id(&Scope {
            collections: RuneModel::collections().into_iter().collect(),
        })?,
    };
    let selected_blob = hash_file(&fsio, &source_root.path().join("source0.ogg"))?;
    let shared = Arc::new(RwLock::new(source));
    let app = library_router(SharedLibrary(shared.clone()), Arc::new(|| {})).merge(blob_router(
        BlobContext {
            store: shared,
            db: source_db.clone(),
            fsio: fsio.clone(),
            root: source_root.path().into(),
            node_id: source_node,
            upload_lock: Arc::new(Mutex::new(())),
            notify: Arc::new(|| {}),
        },
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move { axum::serve(listener, app).await });

    for stage in ["partial", "published", "scanned", "attached"] {
        let root = tempfile::tempdir()?;
        let db_path = root.path().join("replica.sqlite");
        let node = Uuid::new_v4();
        let db = open_replica(&db_path).await?;
        let synchronous: i64 = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "PRAGMA synchronous",
            ))
            .await?
            .context("missing synchronous setting")?
            .try_get("", "synchronous")?;
        assert_eq!(
            synchronous, 3,
            "replica must use SQLite EXTRA synchronous mode"
        );
        initialize_db(&db, &node.to_string()).await?;
        sync_library(db.clone(), node, &base, reqwest::Client::new()).await?;
        let selected = media_files::Entity::find()
            .filter(media_files::Column::SourceUuid.eq(selected_track.to_string()))
            .one(&db)
            .await?
            .context("missing selected catalog item")?
            .id;
        let other = media_files::Entity::find()
            .filter(media_files::Column::SourceUuid.eq(other_track.to_string()))
            .one(&db)
            .await?
            .context("missing other catalog item")?
            .id;
        set_download_rule(
            &db,
            vec![("lib::track".into(), selected.to_string())],
            None,
            true,
        )
        .await?;
        let before = fixtures::open_library(db.clone(), node, None).await?;
        let baseline_head = before.head().await?;
        let baseline_counter = revision_counter(&db).await?;
        let baseline_checkpoint = before.endpoint().checkpoint(&checkpoint).await?;
        assert!(baseline_checkpoint.is_some());
        drop(before);
        let final_path = blob_path_for_track(
            &fsio,
            root.path(),
            selected_track,
            &selected_blob.digest,
            "ogg",
        )?;
        db.close().await?;
        let marker = root.path().join("fault.marker");
        let log_path = root.path().join("child.log");
        let log = File::create(&log_path)?;
        let mut child = Command::new(std::env::current_exe()?)
            .args(["--exact", "import_child", "--nocapture"])
            .env("RUNE_TEST_DB", &db_path)
            .env("RUNE_TEST_ROOT", root.path())
            .env("RUNE_TEST_NODE", node.to_string())
            .env("RUNE_TEST_SERVER", &base)
            .env("RUNE_SYNC_FAULT_STAGE", stage)
            .env("RUNE_SYNC_FAULT_MARKER", &marker)
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .spawn()?;
        let reached = tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                if marker.exists() {
                    break true;
                }
                if child.try_wait().unwrap().is_some() {
                    break false;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap_or(false);
        if !reached {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!(
                "child did not reach {stage}: {}",
                std::fs::read_to_string(&log_path).unwrap_or_default()
            );
        }
        child.kill()?;
        let status = child.wait()?;
        assert_eq!(status.signal(), Some(9), "{stage}: expected SIGKILL");
        let db = open_replica(&db_path).await?;
        let integrity: String = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "PRAGMA integrity_check",
            ))
            .await?
            .unwrap()
            .try_get("", "integrity_check")?;
        assert_eq!(integrity, "ok", "{stage}");
        let local = fixtures::open_library(db.clone(), node, None).await?;
        assert_eq!(
            is_materialized(&db, &fsio, root.path(), selected).await?,
            stage == "attached",
            "{stage}"
        );
        assert!(
            !is_materialized(&db, &fsio, root.path(), other).await?,
            "{stage}"
        );
        assert_eq!(
            sync_download_jobs::Entity::find().count(&db).await?,
            if stage == "attached" { 0 } else { 1 },
            "{stage}"
        );
        assert_eq!(final_path.exists(), stage != "partial", "{stage}");
        if final_path.exists() {
            assert_eq!(hash_file(&fsio, &final_path)?, selected_blob, "{stage}");
        }
        if stage == "partial" {
            let partials = std::fs::read_dir(final_path.parent().unwrap())?
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.file_name().to_string_lossy().contains(".part-"))
                .collect::<Vec<_>>();
            assert_eq!(partials.len(), 1);
            let partial = std::fs::metadata(partials[0].path())?;
            assert!(partial.len() > 0 && partial.len() < selected_blob.size);
        }
        // A user or startup scan can run before the interrupted import is retried.
        fixtures::scan_library(
            &fsio,
            &db,
            &node.to_string(),
            root.path(),
            &local.record_store(),
        )
        .await?;
        assert_eq!(
            local.head().await?,
            baseline_head,
            "{stage}: head changed before retry"
        );
        assert_eq!(
            revision_counter(&db).await?,
            baseline_counter,
            "{stage}: counter changed before retry"
        );
        assert_eq!(
            local.endpoint().checkpoint(&checkpoint).await?,
            baseline_checkpoint,
            "{stage}: checkpoint changed before retry"
        );
        let record = local
            .get(&ObjectKey {
                collection: Collection::Tracks,
                id: selected_track,
            })
            .await?
            .context("selected record vanished")?;
        assert!(
            matches!(record.value, Some(Payload::Track(ref value)) if value.blob()? == Some(selected_blob.clone())),
            "{stage}: selected record changed"
        );
        assert_eq!(
            media_files::Entity::find().count(&db).await?,
            2,
            "{stage}: duplicate logical tracks"
        );
        let visible = get_media_files(&db, 0, 100).await?;
        let expected_rows = 2;
        assert_eq!(
            visible.len(),
            expected_rows,
            "{stage}: database rows must remain queryable"
        );
        let statuses = audio_availability(
            &db,
            &fsio,
            root.path(),
            &visible.iter().map(|row| row.id).collect::<Vec<_>>(),
        )
        .await?;
        let pending: Vec<_> = statuses
            .iter()
            .filter_map(|(id, status)| (*status == AudioAvailability::ImportPending).then_some(*id))
            .collect();
        if stage != "attached" {
            assert_eq!(
                pending.len(),
                1,
                "{stage}: staged row must be marked pending"
            );
            assert_eq!(
                materialized_path(&db, &fsio, root.path(), pending[0]).await?,
                None
            );
            assert_eq!(resolve_local_id(&db, pending[0]).await?, Some(selected));
            assert_eq!(
                catalog_track(&db, pending[0]).await?.map(|entry| entry.0),
                Some(selected_track)
            );
        } else {
            assert!(pending.is_empty());
        }
        let report = download_requested_audio(
            db.clone(),
            &RecommendationDbConnection::new(),
            fsio.clone(),
            root.path().into(),
            node,
            &MetadataPeer::new(&base, reqwest::Client::new()),
            Arc::new(|| {}),
        )
        .await?;
        assert_eq!(
            report.downloaded,
            if stage == "attached" { 0 } else { 1 },
            "{stage}"
        );
        assert!(
            is_materialized(&db, &fsio, root.path(), selected).await?,
            "{stage}"
        );
        assert!(
            !is_materialized(&db, &fsio, root.path(), other).await?,
            "{stage}: non-selected audio downloaded"
        );
        assert_eq!(hash_file(&fsio, &final_path)?, selected_blob, "{stage}");
        assert_eq!(
            sync_download_jobs::Entity::find().count(&db).await?,
            0,
            "{stage}"
        );
        assert_eq!(
            local.head().await?,
            baseline_head,
            "{stage}: head changed after retry"
        );
        assert_eq!(
            revision_counter(&db).await?,
            baseline_counter,
            "{stage}: counter changed after retry"
        );
        assert_eq!(
            local.endpoint().checkpoint(&checkpoint).await?,
            baseline_checkpoint,
            "{stage}: checkpoint changed after retry"
        );
        assert_eq!(
            media_files::Entity::find().count(&db).await?,
            2,
            "{stage}: duplicate logical tracks after retry"
        );
        assert_eq!(
            get_media_files(&db, 0, 100).await?.len(),
            2,
            "{stage}: duplicate exposed after retry"
        );
        db.close().await?;
    }
    server.abort();
    Ok(())
}
