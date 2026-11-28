use anyhow::Result;
use database::{
    actions::{
        fingerprint::{
            ComputedFingerprint, ComputedSimilarity, assign_computed_fingerprint,
            assign_computed_similarity, default_similarity_configuration, get_duplicate_files,
            mark_duplicate_files,
        },
        metadata::{MissingHostFilePolicy, ScanOptions, scan_audio_library},
    },
    entities::{media_file_fingerprint, media_files},
    sync::model::{Collection, ObjectKey},
    test_fixtures::fixtures,
};
use fsio::FsIo;
use sea_orm::{
    ConnectOptions, Database, DatabaseConnection, EntityTrait, QueryOrder,
    sqlx::sqlite::SqliteOperation,
};
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use sync_orm::causal_progress;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

async fn single_connection_database(node: Uuid) -> Result<DatabaseConnection> {
    let mut options = ConnectOptions::new("sqlite::memory:");
    options.max_connections(1);
    let db = Database::connect(options).await?;
    migration::up_with_actor(&db, Some(&node.to_string()), None).await?;
    Ok(db)
}

// The isolated pool has one connection, so the worker uses the observed handle.
async fn cancel_on_write(
    db: &DatabaseConnection,
    table: &'static str,
    operation: SqliteOperation,
    token: CancellationToken,
) -> Result<Arc<AtomicUsize>> {
    let writes = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&writes);
    let mut connection = db.get_sqlite_connection_pool().acquire().await?;
    connection
        .lock_handle()
        .await?
        .set_update_hook(move |write| {
            if write.table == table && write.operation == operation {
                observed.fetch_add(1, Ordering::Relaxed);
                token.cancel();
            }
        });
    Ok(writes)
}

#[tokio::test]
async fn cleanup_cancellation_finishes_only_the_started_removal() -> Result<()> {
    let node = Uuid::new_v4();
    let root = tempfile::tempdir()?;
    let fsio = FsIo::new();
    let db = single_connection_database(node).await?;
    let library = fixtures::open_library(db.clone(), node, None).await?;
    let store = library.record_store();
    let names = ["one.ogg", "two.ogg"];
    fixtures::scan_audio_fixtures(&fsio, &db, &node.to_string(), root.path(), &store, &names)
        .await?;
    let files = media_files::Entity::find().all(&db).await?;
    assert_eq!(files.len(), 2);
    let mut tracks = Vec::new();
    for file in files {
        let key = ObjectKey {
            collection: Collection::Tracks,
            id: file.source_uuid.parse()?,
        };
        assert!(
            store
                .get_on(&db, &key)
                .await?
                .is_some_and(|row| row.value.is_some())
        );
        tracks.push((file.id, key));
    }
    for name in names {
        fs::remove_file(root.path().join(name))?;
    }
    // Keep the root nonempty so the ordinary missing-file cleanup starts.
    fs::create_dir(root.path().join("other-media"))?;

    let token = CancellationToken::new();
    let deletions =
        cancel_on_write(&db, "media_files", SqliteOperation::Delete, token.clone()).await?;
    scan_audio_library(
        &fsio,
        &db,
        &node.to_string(),
        root.path(),
        ScanOptions {
            cleanup: true,
            force: false,
            missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
            cancel_token: Some(token.clone()),
        },
        |_| {},
        &store,
    )
    .await?;

    assert!(
        token.is_cancelled(),
        "missing-file cleanup never deleted a row"
    );
    let remaining = media_files::Entity::find().all(&db).await?;
    assert_eq!(
        remaining.len(),
        1,
        "cleanup removed a later row after cancellation"
    );
    assert_eq!(deletions.load(Ordering::Relaxed), 1);
    for (file_id, key) in tracks {
        let live = store
            .get_on(&db, &key)
            .await?
            .is_some_and(|row| row.value.is_some());
        assert_eq!(live, remaining[0].id == file_id);
    }
    Ok(())
}

#[tokio::test]
async fn duplicate_mark_cancellation_preserves_the_previous_result_and_audio() -> Result<()> {
    let node = Uuid::new_v4();
    let root = tempfile::tempdir()?;
    let fsio = FsIo::new();
    let db = single_connection_database(node).await?;
    let library = fixtures::open_library(db.clone(), node, None).await?;
    let store = library.record_store();
    let names = ["one.ogg", "two.ogg", "three.ogg", "four.ogg"];
    fixtures::scan_audio_fixtures(&fsio, &db, &node.to_string(), root.path(), &store, &names)
        .await?;
    let files = media_files::Entity::find().all(&db).await?;
    assert_eq!(files.len(), 4);

    let fingerprint = vec![1, 0, 0, 0, 2, 0, 0, 0];
    for file in &files {
        assign_computed_fingerprint(
            &db,
            &store,
            file,
            ComputedFingerprint {
                algorithm: "chromaprint".into(),
                configuration: "rusty-chromaprint-0.3.0/default".into(),
                format_version: 1,
                bytes: fingerprint.clone(),
                is_duplicated: 0,
            },
        )
        .await?;
    }
    for pair in files.as_chunks::<2>().0 {
        assign_computed_similarity(
            &db,
            &store,
            &pair[0],
            &pair[1],
            ComputedSimilarity {
                first_fingerprint: &fingerprint,
                second_fingerprint: &fingerprint,
                algorithm: "chromaprint-match".into(),
                configuration: default_similarity_configuration(),
                format_version: 1,
                score: 0.9,
            },
        )
        .await?;
    }

    assert_eq!(
        mark_duplicate_files(&db, 0.85, |_, _| {}, None, &store).await?,
        2
    );
    let before = media_file_fingerprint::Entity::find()
        .order_by_asc(media_file_fingerprint::Column::Id)
        .all(&db)
        .await?;
    let head = causal_progress::head_on(&db, store.namespace()).await?;
    let audio = fs::read(root.path().join(names[0]))?;

    let token = CancellationToken::new();
    let publications =
        cancel_on_write(&db, "sync_journal", SqliteOperation::Insert, token.clone()).await?;
    mark_duplicate_files(&db, 0.95, |_, _| {}, Some(&token), &store).await?;

    assert!(
        token.is_cancelled(),
        "duplicate marking never journaled a change"
    );
    assert_eq!(
        get_duplicate_files(&db).await?.len(),
        2,
        "cancelled marking changed the previously committed result"
    );
    assert_eq!(publications.load(Ordering::Relaxed), 1);
    assert_eq!(
        media_file_fingerprint::Entity::find()
            .order_by_asc(media_file_fingerprint::Column::Id)
            .all(&db)
            .await?,
        before
    );
    assert_eq!(
        causal_progress::head_on(&db, store.namespace())
            .await?
            .sequence,
        head.sequence
    );
    for name in names {
        assert_eq!(fs::read(root.path().join(name))?, audio);
    }
    Ok(())
}
