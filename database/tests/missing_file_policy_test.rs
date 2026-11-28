use anyhow::Result;
use database::{
    actions::{
        logging::list_log,
        metadata::{
            EmptyLibraryCleanupDeferred, MissingHostFilePolicy, confirm_empty_library_cleanup,
            preview_empty_library_cleanup, scan_audio_file,
        },
    },
    entities::{media_files, source_tags as media_metadata},
    sync::{
        SeaOrmSyncStore,
        model::{Collection, ObjectKey},
    },
};
use fsio::FsIo;
use sea_orm::{ColumnTrait, ConnectionTrait, Database, EntityTrait, PaginatorTrait, QueryFilter};
use std::fs;
use support::fixtures;
use uuid::Uuid;

#[tokio::test]
async fn missing_host_file_remove_everywhere_authors_removal() -> Result<()> {
    let node = Uuid::new_v4();
    let db = Database::connect("sqlite::memory:").await?;
    migration::up_with_actor(&db, Some(&node.to_string()), None).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let root = tempfile::tempdir()?;
    let path = root.path().join("track.ogg");
    fs::copy(fixtures::audio_fixture(), &path)?;
    let fs = FsIo::new();
    let id = scan_audio_file(&fs, &db, &node.to_string(), root.path(), &path, &store).await?;
    use database::entities::{media_file_likes, media_file_stats};
    use sea_orm::Set;
    let row = media_files::Entity::find_by_id(id).one(&db).await?.unwrap();
    let track_uuid = row.source_uuid;
    let key = ObjectKey {
        collection: Collection::Tracks,
        id: track_uuid.parse()?,
    };
    media_file_likes::Entity::insert(media_file_likes::ActiveModel {
        track_uuid: Set(track_uuid.clone()),
        source_uuid: Set(Uuid::new_v4().to_string()),
        liked: Set(true),
        updated_at: Set("2025-01-01T00:00:00Z".into()),
        ..Default::default()
    })
    .exec(&db)
    .await?;
    media_file_stats::Entity::insert(media_file_stats::ActiveModel {
        track_uuid: Set(track_uuid),
        source_uuid: Set(Uuid::new_v4().to_string()),
        device_id: Set(node.to_string()),
        skipped: Set(1),
        played_through: Set(2),
        updated_at: Set("2025-01-01T00:00:00Z".into()),
        ..Default::default()
    })
    .exec(&db)
    .await?;
    fs::remove_file(path)?;
    // A nonempty root still uses the ordinary Remove Everywhere scan.
    fs::create_dir(root.path().join("still-mounted"))?;
    fixtures::cleanup_scan(
        &fs,
        &db,
        &node.to_string(),
        root.path(),
        &store,
        MissingHostFilePolicy::RemoveEverywhere,
    )
    .await?;
    assert!(
        media_files::Entity::find_by_id(id)
            .one(&db)
            .await?
            .is_none()
    );
    assert!(
        store
            .get_on(&db, &key)
            .await?
            .is_none_or(|record| record.value.is_none())
    );
    assert!(media_file_likes::Entity::find().one(&db).await?.is_some());
    assert!(media_file_stats::Entity::find().one(&db).await?.is_some());
    Ok(())
}

async fn unavailable_case(managed: bool, policy: MissingHostFilePolicy) -> Result<()> {
    let node = Uuid::new_v4();
    let db = Database::connect("sqlite::memory:").await?;
    migration::up_with_actor(&db, Some(&node.to_string()), None).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let root = tempfile::tempdir()?;
    let path = root.path().join("track.ogg");
    fs::copy(fixtures::audio_fixture(), &path)?;
    let fs = FsIo::new();
    let id = scan_audio_file(&fs, &db, &node.to_string(), root.path(), &path, &store).await?;
    if managed {
        use sea_orm::{ColumnTrait, QueryFilter, sea_query::Expr};
        media_files::Entity::update_many()
            .col_expr(media_files::Column::Directory, Expr::value("Rune Sync"))
            .filter(media_files::Column::Id.eq(id))
            .exec(&db)
            .await?;
    }
    let row = media_files::Entity::find_by_id(id).one(&db).await?.unwrap();
    let key = ObjectKey {
        collection: Collection::Tracks,
        id: row.source_uuid.parse()?,
    };
    let before = store.get_on(&db, &key).await?;
    fs::remove_file(path)?;
    fixtures::cleanup_scan(&fs, &db, &node.to_string(), root.path(), &store, policy).await?;
    assert!(
        !media_files::Entity::find_by_id(id)
            .one(&db)
            .await?
            .unwrap()
            .local_available
    );
    assert_eq!(before, store.get_on(&db, &key).await?);
    Ok(())
}

#[tokio::test]
async fn missing_managed_file_is_only_unavailable() -> Result<()> {
    unavailable_case(true, MissingHostFilePolicy::RemoveEverywhere).await
}
#[tokio::test]
async fn missing_host_file_keep_as_remote_marks_unavailable() -> Result<()> {
    unavailable_case(false, MissingHostFilePolicy::KeepAsRemote).await
}
#[tokio::test]
async fn mass_missing_host_files_are_not_removed() -> Result<()> {
    let node = Uuid::new_v4();
    let db = Database::connect("sqlite::memory:").await?;
    migration::up_with_actor(&db, Some(&node.to_string()), None).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let root = tempfile::tempdir()?;
    let fs = FsIo::new();
    let mut ids = Vec::new();
    for i in 0..10 {
        let path = root.path().join(format!("track{i}.ogg"));
        fs::copy(fixtures::audio_fixture(), &path)?;
        ids.push(scan_audio_file(&fs, &db, &node.to_string(), root.path(), &path, &store).await?);
    }
    // Keep identical copies present during scanning so they cannot be treated as moves.
    for i in 0..6 {
        fs::remove_file(root.path().join(format!("track{i}.ogg")))?;
    }
    fixtures::cleanup_scan(
        &fs,
        &db,
        &node.to_string(),
        root.path(),
        &store,
        MissingHostFilePolicy::RemoveEverywhere,
    )
    .await?;
    for (i, id) in ids.into_iter().enumerate() {
        assert_eq!(
            media_files::Entity::find_by_id(id)
                .one(&db)
                .await?
                .unwrap()
                .local_available,
            i >= 6
        );
    }
    Ok(())
}

#[tokio::test]
async fn an_empty_listing_preserves_host_tracks_and_sync_values() -> Result<()> {
    let mut observations = Vec::new();
    let mut outcomes = Vec::new();
    for count in [1, 5, 6] {
        let node = Uuid::new_v4();
        let db = Database::connect("sqlite::memory:").await?;
        migration::up_with_actor(&db, Some(&node.to_string()), None).await?;
        let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
        let temp = tempfile::tempdir()?;
        let library = temp.path().join("library");
        fs::create_dir(&library)?;
        let fs = FsIo::new();
        let mut tracks = Vec::new();
        for i in 0..count {
            let path = library.join(format!("track{i}.ogg"));
            fs::copy(fixtures::audio_fixture(), &path)?;
            let id = scan_audio_file(&fs, &db, &node.to_string(), &library, &path, &store).await?;
            let row = media_files::Entity::find_by_id(id).one(&db).await?.unwrap();
            let key = ObjectKey {
                collection: Collection::Tracks,
                id: row.source_uuid.parse()?,
            };
            let value = store.get_on(&db, &key).await?.unwrap();
            assert!(value.value.is_some());
            tracks.push((key, value));
        }

        // NoOpFsIo returns a successful empty listing without touching the files.
        let outcome = fixtures::cleanup_scan(
            &FsIo::new_noop(),
            &db,
            &node.to_string(),
            &library,
            &store,
            MissingHostFilePolicy::RemoveEverywhere,
        )
        .await;
        assert_eq!(fs::read_dir(&library)?.count(), count);
        let mut unchanged = 0;
        for (key, value) in tracks {
            if store.get_on(&db, &key).await? == Some(value) {
                unchanged += 1;
            }
        }
        let available = media_files::Entity::find()
            .filter(media_files::Column::LocalAvailable.eq(true))
            .count(&db)
            .await?;
        observations.push((count, available, unchanged));
        outcomes.push(outcome);
    }
    assert_eq!(observations, [(1, 1, 1), (5, 5, 5), (6, 6, 6)]);
    for outcome in outcomes {
        assert!(outcome.is_err_and(|error| error.is::<EmptyLibraryCleanupDeferred>()));
    }
    Ok(())
}

use database::test_fixtures::causal_library as support;
#[tokio::test]
async fn missing_host_removal_reaches_another_replica() -> Result<()> {
    let namespace = Uuid::new_v4();
    let node = Uuid::new_v4();
    let a = support::SqlLibrary::new(namespace, node).await?;
    let b = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let path = a.root.path().join("track.ogg");
    fs::copy(fixtures::audio_fixture(), &path)?;
    let fs = FsIo::new();
    let store = a.sync_store.record_store();
    let id = scan_audio_file(&fs, &a.db, &node.to_string(), a.root.path(), &path, &store).await?;
    let uuid = media_files::Entity::find_by_id(id)
        .one(&a.db)
        .await?
        .unwrap()
        .source_uuid;
    support::exchange(&a.peer, &b.peer).await?;
    use sea_orm::{ColumnTrait, QueryFilter};
    assert!(
        media_files::Entity::find()
            .filter(media_files::Column::SourceUuid.eq(&uuid))
            .one(&b.db)
            .await?
            .is_some()
    );
    fs::remove_file(&path)?;
    let index = a.root.path().join(".rune");
    fs::create_dir(&index)?;
    fs::write(index.join(".redirect"), namespace.to_string())?;
    let hidden_item = a.root.path().join(".keep");
    fs::write(&hidden_item, "fixture")?;
    assert!(
        preview_empty_library_cleanup(&fs, &a.db, a.root.path())
            .await?
            .is_none()
    );
    fs::remove_file(hidden_item)?;
    let visible_dir = a.root.path().join("visible");
    fs::create_dir(&visible_dir)?;
    assert!(
        preview_empty_library_cleanup(&fs, &a.db, a.root.path())
            .await?
            .is_none()
    );
    fs::remove_dir(visible_dir)?;
    let outcome = fixtures::cleanup_scan(
        &fs,
        &a.db,
        &node.to_string(),
        a.root.path(),
        &store,
        MissingHostFilePolicy::RemoveEverywhere,
    )
    .await;
    assert!(outcome.is_err_and(|error| error.is::<EmptyLibraryCleanupDeferred>()));
    let preview = preview_empty_library_cleanup(&fs, &a.db, a.root.path())
        .await?
        .unwrap();
    assert_eq!(preview.count, 1);
    assert!(
        !confirm_empty_library_cleanup(
            &fs,
            &a.db,
            a.root.path(),
            &store,
            &preview.fingerprint,
            || false,
            || true
        )
        .await?
    );
    fs::copy(fixtures::audio_fixture(), &path)?;
    assert!(
        !confirm_empty_library_cleanup(
            &fs,
            &a.db,
            a.root.path(),
            &store,
            &preview.fingerprint,
            || true,
            || true
        )
        .await?
    );
    assert!(
        media_files::Entity::find_by_id(id)
            .one(&a.db)
            .await?
            .is_some()
    );
    fs::remove_file(path)?;
    assert!(
        !confirm_empty_library_cleanup(
            &fs,
            &a.db,
            a.root.path(),
            &store,
            &preview.fingerprint,
            || true,
            || false,
        )
        .await?
    );
    assert!(
        media_files::Entity::find_by_id(id)
            .one(&a.db)
            .await?
            .is_some()
    );
    let key = ObjectKey {
        collection: Collection::Tracks,
        id: uuid.parse()?,
    };
    assert!(store.get_on(&a.db, &key).await?.unwrap().value.is_some());
    assert!(
        confirm_empty_library_cleanup(
            &fs,
            &a.db,
            a.root.path(),
            &store,
            &preview.fingerprint,
            || true,
            || true
        )
        .await?
    );
    support::exchange(&a.peer, &b.peer).await?;
    assert!(
        media_files::Entity::find()
            .filter(media_files::Column::SourceUuid.eq(&uuid))
            .one(&b.db)
            .await?
            .is_none()
    );
    Ok(())
}

/// A library folder that is gone would otherwise look like one whose tracks
/// were all deleted, so the scan fails and leaves the catalog as it is.
#[tokio::test]
async fn missing_library_root_fails_the_scan_and_keeps_its_tracks() -> Result<()> {
    let node = Uuid::new_v4();
    let db = Database::connect("sqlite::memory:").await?;
    migration::up_with_actor(&db, Some(&node.to_string()), None).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("library");
    fs::create_dir(&root)?;
    fs::copy(fixtures::audio_fixture(), root.join("track.ogg"))?;
    let fs = FsIo::new();
    let actor = node.to_string();
    let scan = || {
        fixtures::cleanup_scan(
            &fs,
            &db,
            &actor,
            &root,
            &store,
            MissingHostFilePolicy::RemoveEverywhere,
        )
    };
    assert_eq!(scan().await?, 1);
    let indexed = media_files::Entity::find().all(&db).await?;

    fs::rename(&root, temp.path().join("offline"))?;
    let scanned = scan().await;

    assert!(
        scanned.is_err(),
        "a missing library root was reported as a successful scan: {scanned:?}"
    );
    assert_eq!(media_files::Entity::find().all(&db).await?, indexed);
    Ok(())
}

/// A successful walk can be followed by a provider failure while reading a track
/// or checking which rows to remove. Neither failure makes missing rows deletable.
#[tokio::test]
async fn a_document_provider_error_after_the_walk_keeps_previously_indexed_tracks() -> Result<()> {
    use database::actions::metadata::{ScanOptions, scan_audio_library};
    use fsio::failing_read::{DocumentOperation, unavailable_document};

    let node = Uuid::new_v4();
    let actor = node.to_string();
    let db = Database::connect("sqlite::memory:").await?;
    migration::up_with_actor(&db, Some(&actor), None).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let root = tempfile::tempdir()?;
    let removed = root.path().join("removed.ogg");
    let inaccessible = root.path().join("unavailable.ogg");
    fs::copy(fixtures::audio_fixture(), &removed)?;
    fs::copy(fixtures::audio_fixture(), &inaccessible)?;
    let scan = async |fs: FsIo, force| {
        scan_audio_library(
            &fs,
            &db,
            &actor,
            root.path(),
            ScanOptions {
                cleanup: true,
                force,
                missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
                cancel_token: None,
            },
            |_| {},
            &store,
        )
        .await
    };
    assert_eq!(scan(FsIo::new(), false).await?, 2);
    let indexed = media_files::Entity::find().all(&db).await?;
    fs::remove_file(&removed)?;

    for operation in [
        DocumentOperation::ModifiedTime,
        DocumentOperation::Open,
        DocumentOperation::Exists,
    ] {
        let fs = unavailable_document(&inaccessible, operation);
        let error = scan(fs, operation == DocumentOperation::Open)
            .await
            .expect_err("scan should report unavailable folder");
        assert!(
            fsio::is_document_provider_error(&error),
            "{operation:?}: {error:#}"
        );
        assert_eq!(
            media_files::Entity::find().all(&db).await?,
            indexed,
            "{operation:?} changed existing rows"
        );
    }
    scan(FsIo::new(), false).await?;
    let remaining = media_files::Entity::find().all(&db).await?;
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].file_name, "unavailable.ogg");
    Ok(())
}

#[tokio::test]
async fn empty_library_root_scans_successfully() -> Result<()> {
    let node = Uuid::new_v4();
    let db = Database::connect("sqlite::memory:").await?;
    migration::up_with_actor(&db, Some(&node.to_string()), None).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let root = tempfile::tempdir()?;
    let scanned = fixtures::cleanup_scan(
        &FsIo::new(),
        &db,
        &node.to_string(),
        root.path(),
        &store,
        MissingHostFilePolicy::RemoveEverywhere,
    )
    .await?;
    assert_eq!(scanned, 0);
    Ok(())
}

/// One rejected removal must leave that entry and its metadata intact, keep
/// cleaning the other entries, and report the failed entry in the app log.
#[tokio::test]
async fn rejected_missing_file_removal_is_logged_and_rolled_back() -> Result<()> {
    let node = Uuid::new_v4();
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("library");
    fs::create_dir(&root)?;
    let db = fixtures::disk_database(&temp.path().join("library.sqlite"), node).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let fs = FsIo::new();
    let blocked_path = root.join("blocked.ogg");
    let removed_path = root.join("removed.ogg");
    fs::copy(fixtures::audio_fixture(), &blocked_path)?;
    fs::copy(fixtures::audio_fixture(), &removed_path)?;
    let actor = node.to_string();
    let blocked_id = scan_audio_file(&fs, &db, &actor, &root, &blocked_path, &store).await?;
    let removed_id = scan_audio_file(&fs, &db, &actor, &root, &removed_path, &store).await?;
    let metadata_before = media_metadata::Entity::find()
        .filter(media_metadata::Column::FileId.eq(blocked_id))
        .count(&db)
        .await?;
    assert!(metadata_before > 0);

    db.execute_unprepared(&format!(
        "CREATE TRIGGER reject_cleanup BEFORE DELETE ON media_files \
         WHEN OLD.id = {blocked_id} \
         BEGIN SELECT RAISE(ABORT, 'cleanup rejected by test trigger'); END"
    ))
    .await?;
    fs::remove_file(&blocked_path)?;
    fs::remove_file(&removed_path)?;
    fs::create_dir(root.join("still-mounted"))?;

    let scanned = fixtures::cleanup_scan(
        &fs,
        &db,
        &actor,
        &root,
        &store,
        MissingHostFilePolicy::RemoveEverywhere,
    )
    .await?;
    assert_eq!(scanned, 0);

    assert!(
        media_files::Entity::find_by_id(blocked_id)
            .one(&db)
            .await?
            .is_some(),
        "the rejected entry must be rolled back, not left half removed"
    );
    assert_eq!(
        media_metadata::Entity::find()
            .filter(media_metadata::Column::FileId.eq(blocked_id))
            .count(&db)
            .await?,
        metadata_before,
        "metadata deleted before the rejected removal must be rolled back"
    );
    assert!(
        media_files::Entity::find_by_id(removed_id)
            .one(&db)
            .await?
            .is_none(),
        "the other missing entry must still be removed"
    );

    let failures: Vec<_> = list_log(&db, 0, 100)
        .await?
        .into_iter()
        .filter(|log| {
            log.level == "WARNING" && log.domain == "actions::metadata::clean_up_database"
        })
        .collect();
    assert_eq!(
        failures.len(),
        1,
        "the app log must contain one cleanup failure"
    );
    assert!(
        failures[0]
            .detail
            .contains(&format!("Failed to clean up missing file {blocked_id}:")),
        "the app log must identify the failed entry: {}",
        failures[0].detail
    );
    assert!(
        failures[0]
            .detail
            .contains("cleanup rejected by test trigger"),
        "the app log must include the failure reason: {}",
        failures[0].detail
    );

    db.execute_unprepared("DROP TRIGGER reject_cleanup").await?;
    fixtures::cleanup_scan(
        &fs,
        &db,
        &actor,
        &root,
        &store,
        MissingHostFilePolicy::RemoveEverywhere,
    )
    .await?;
    assert!(
        media_files::Entity::find_by_id(blocked_id)
            .one(&db)
            .await?
            .is_none(),
        "retry after removing the fault must remove the entry"
    );
    assert_eq!(
        list_log(&db, 0, 100)
            .await?
            .into_iter()
            .filter(|log| log.level == "WARNING"
                && log.domain == "actions::metadata::clean_up_database")
            .count(),
        1,
        "a successful retry must not add another failure"
    );
    db.close().await?;
    Ok(())
}
