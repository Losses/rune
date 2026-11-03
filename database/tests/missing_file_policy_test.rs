use anyhow::Result;
use database::{
    actions::metadata::{empty_progress_callback, scan_audio_file, scan_audio_library},
    entities::media_files,
    sync::SeaOrmSyncStore,
};
use fsio::FsIo;
use sea_orm::{Database, EntityTrait};
use uuid::Uuid;

#[tokio::test]
async fn missing_host_file_remove_everywhere_authors_removal() -> Result<()> {
    let node = Uuid::new_v4();
    let db = Database::connect("sqlite::memory:").await?;
    migration::up_with_actor(&db, Some(&node.to_string()), None).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let root = tempfile::tempdir()?;
    let path = root.path().join("track.ogg");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/startup_0.ogg"),
        &path,
    )?;
    let fs = FsIo::new();
    let id = scan_audio_file(&fs, &db, &node.to_string(), root.path(), &path, &store).await?;
    use database::entities::{media_file_likes, media_file_stats};
    use sea_orm::Set;
    let row = media_files::Entity::find_by_id(id).one(&db).await?.unwrap();
    let track_uuid = row.hlc_uuid;
    let key = database::sync::model::ObjectKey {
        collection: database::sync::model::Collection::Tracks,
        id: track_uuid.parse()?,
    };
    media_file_likes::Entity::insert(media_file_likes::ActiveModel {
        track_uuid: Set(track_uuid.clone()),
        hlc_uuid: Set(Uuid::new_v4().to_string()),
        liked: Set(true),
        updated_at: Set("2025-01-01T00:00:00Z".into()),
        ..Default::default()
    })
    .exec(&db)
    .await?;
    media_file_stats::Entity::insert(media_file_stats::ActiveModel {
        track_uuid: Set(track_uuid),
        hlc_uuid: Set(Uuid::new_v4().to_string()),
        device_id: Set(node.to_string()),
        skipped: Set(1),
        played_through: Set(2),
        updated_at: Set("2025-01-01T00:00:00Z".into()),
        ..Default::default()
    })
    .exec(&db)
    .await?;
    std::fs::remove_file(path)?;
    scan_audio_library(
        &fs,
        &db,
        &node.to_string(),
        root.path(),
        true,
        false,
        empty_progress_callback,
        None,
        &store,
        database::actions::metadata::MissingHostFilePolicy::RemoveEverywhere,
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

async fn unavailable_case(
    managed: bool,
    policy: database::actions::metadata::MissingHostFilePolicy,
) -> Result<()> {
    let node = Uuid::new_v4();
    let db = Database::connect("sqlite::memory:").await?;
    migration::up_with_actor(&db, Some(&node.to_string()), None).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let root = tempfile::tempdir()?;
    let path = root.path().join("track.ogg");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/startup_0.ogg"),
        &path,
    )?;
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
    let key = database::sync::model::ObjectKey {
        collection: database::sync::model::Collection::Tracks,
        id: row.hlc_uuid.parse()?,
    };
    let before = store.get_on(&db, &key).await?;
    std::fs::remove_file(path)?;
    scan_audio_library(
        &fs,
        &db,
        &node.to_string(),
        root.path(),
        true,
        false,
        empty_progress_callback,
        None,
        &store,
        policy,
    )
    .await?;
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
    unavailable_case(
        true,
        database::actions::metadata::MissingHostFilePolicy::RemoveEverywhere,
    )
    .await
}
#[tokio::test]
async fn missing_host_file_keep_as_remote_marks_unavailable() -> Result<()> {
    unavailable_case(
        false,
        database::actions::metadata::MissingHostFilePolicy::KeepAsRemote,
    )
    .await
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
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/startup_0.ogg"),
            &path,
        )?;
        ids.push(scan_audio_file(&fs, &db, &node.to_string(), root.path(), &path, &store).await?);
        if i < 6 {
            std::fs::remove_file(path)?;
        }
    }
    scan_audio_library(
        &fs,
        &db,
        &node.to_string(),
        root.path(),
        true,
        false,
        empty_progress_callback,
        None,
        &store,
        database::actions::metadata::MissingHostFilePolicy::RemoveEverywhere,
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

#[path = "support/causal_library.rs"]
mod support;
#[tokio::test]
async fn missing_host_removal_reaches_another_replica() -> Result<()> {
    let namespace = Uuid::new_v4();
    let node = Uuid::new_v4();
    let a = support::SqlLibrary::new(namespace, node).await?;
    let b = support::SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let path = a.root.path().join("track.ogg");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/startup_0.ogg"),
        &path,
    )?;
    let fs = FsIo::new();
    let store = a.library.record_store();
    let id = scan_audio_file(&fs, &a.db, &node.to_string(), a.root.path(), &path, &store).await?;
    let uuid = media_files::Entity::find_by_id(id)
        .one(&a.db)
        .await?
        .unwrap()
        .hlc_uuid;
    support::exchange(&a.peer, &b.peer).await?;
    use sea_orm::{ColumnTrait, QueryFilter};
    assert!(
        media_files::Entity::find()
            .filter(media_files::Column::HlcUuid.eq(&uuid))
            .one(&b.db)
            .await?
            .is_some()
    );
    std::fs::remove_file(path)?;
    scan_audio_library(
        &fs,
        &a.db,
        &node.to_string(),
        a.root.path(),
        true,
        false,
        empty_progress_callback,
        None,
        &store,
        database::actions::metadata::MissingHostFilePolicy::RemoveEverywhere,
    )
    .await?;
    support::exchange(&a.peer, &b.peer).await?;
    assert!(
        media_files::Entity::find()
            .filter(media_files::Column::HlcUuid.eq(&uuid))
            .one(&b.db)
            .await?
            .is_none()
    );
    Ok(())
}
