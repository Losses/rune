use anyhow::Result;
use database::sync::model::{
    BlobRef, Collection, MetadataTag, Mix, MixClause, ObjectKey, PageRequest, Payload, Scope, Track,
};
use database::{
    actions::{
        playlists::{add_item_to_playlist, create_playlist, get_all_playlists},
        stats::{get_liked, increase_played_through, set_liked},
    },
    sync::LibrarySyncStore,
};
use fsio::FsIo;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ConnectionTrait, Database, EntityTrait, ModelTrait};
use std::sync::Arc;
use sync::{CheckpointKey, ReceiveStore, SyncStore};
#[path = "support/causal_library.rs"]
mod support;
use database::entities::*;
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, IntoActiveModel, QueryFilter, Set, TransactionTrait};
use uuid::Uuid;

async fn setup() -> Result<(sea_orm::DatabaseConnection, tempfile::TempDir)> {
    let root = tempfile::tempdir()?;
    std::fs::write(root.path().join("song.ogg"), b"shared bytes")?;
    let mut options = sea_orm::ConnectOptions::new("sqlite::memory:");
    options
        .max_connections(1)
        .min_connections(1)
        .idle_timeout(None)
        .max_lifetime(None);
    let db = Database::connect(options).await?;
    let _ = migration::initialize_node_id(Uuid::new_v4().to_string());
    Migrator::up(&db, None).await?;
    Ok((db, root))
}
async fn file(db: &sea_orm::DatabaseConnection, id: i32) -> Result<()> {
    let row = support::local_file(db, id, Uuid::new_v4(), "song.ogg").await?;
    let mut row = row.into_active_model();
    row.content_hash = Set(Some(blake3::hash(b"shared bytes").to_hex().to_string()));
    row.content_size = Set(Some(12));
    row.update(db).await?;
    Ok(())
}
async fn track_uuid(db: &sea_orm::DatabaseConnection, id: i32) -> Result<String> {
    Ok(media_files::Entity::find_by_id(id)
        .one(db)
        .await?
        .unwrap()
        .hlc_uuid)
}
async fn marker(db: &sea_orm::DatabaseConnection) -> Result<i32> {
    Ok(media_files::Entity::find()
        .filter(media_files::Column::LocalAvailable.eq(false))
        .one(db)
        .await?
        .unwrap()
        .id)
}
async fn entry_file(db: &sea_orm::DatabaseConnection) -> Result<i32> {
    let entry = media_file_playlists::Entity::find().one(db).await?.unwrap();
    Ok(media_files::Entity::find()
        .filter(media_files::Column::HlcUuid.eq(entry.track_uuid))
        .one(db)
        .await?
        .unwrap()
        .id)
}
async fn played(db: &sea_orm::DatabaseConnection, id: i32) -> Result<i32> {
    Ok(media_file_stats::Entity::find()
        .filter(media_file_stats::Column::TrackUuid.eq(track_uuid(db, id).await?))
        .all(db)
        .await?
        .iter()
        .map(|r| r.played_through)
        .sum())
}
async fn parent(
    db: &sea_orm::DatabaseConnection,
    kind: Collection,
    id: i32,
    name: &str,
    group: &str,
) -> Result<String> {
    use migration::catalog_identity::{self, ParentKind};
    macro_rules! insert {
        ($entity:ident, $kind:expr) => {{
            let uuid = catalog_identity::parent($kind, name).to_string();
            $entity::Entity::insert($entity::ActiveModel {
                id: Set(id),
                name: Set(name.into()),
                group: Set(group.into()),
                hlc_uuid: Set(uuid.clone()),
                ..Default::default()
            })
            .exec(db)
            .await?;
            uuid
        }};
    }
    Ok(match kind {
        Collection::Artists => insert!(artists, ParentKind::Artist),
        Collection::Genres => insert!(genres, ParentKind::Genre),
        Collection::Albums => insert!(albums, ParentKind::Album),
        _ => anyhow::bail!("unsupported fixture parent"),
    })
}
async fn member(
    db: &sea_orm::DatabaseConnection,
    kind: Collection,
    file: i32,
    parent_id: i32,
    number: Option<i32>,
) -> Result<()> {
    use migration::catalog_identity::{self, ParentKind};
    let track = track_uuid(db, file).await?;
    match kind {
        Collection::Artists => {
            let uuid = artists::Entity::find_by_id(parent_id)
                .one(db)
                .await?
                .unwrap()
                .hlc_uuid;
            media_file_artists::Entity::insert(media_file_artists::ActiveModel {
                hlc_uuid: Set(catalog_identity::membership(
                    track.parse()?,
                    ParentKind::Artist,
                    uuid.parse()?,
                )
                .to_string()),
                track_uuid: Set(track),
                artist_uuid: Set(uuid),
                ..Default::default()
            })
            .exec(db)
            .await?;
        }
        Collection::Genres => {
            let uuid = genres::Entity::find_by_id(parent_id)
                .one(db)
                .await?
                .unwrap()
                .hlc_uuid;
            media_file_genres::Entity::insert(media_file_genres::ActiveModel {
                hlc_uuid: Set(catalog_identity::membership(
                    track.parse()?,
                    ParentKind::Genre,
                    uuid.parse()?,
                )
                .to_string()),
                track_uuid: Set(track),
                genre_uuid: Set(uuid),
                ..Default::default()
            })
            .exec(db)
            .await?;
        }
        Collection::Albums => {
            let uuid = albums::Entity::find_by_id(parent_id)
                .one(db)
                .await?
                .unwrap()
                .hlc_uuid;
            media_file_albums::Entity::insert(media_file_albums::ActiveModel {
                hlc_uuid: Set(track.clone()),
                track_uuid: Set(track),
                album_uuid: Set(uuid),
                track_number: Set(number),
                ..Default::default()
            })
            .exec(db)
            .await?;
        }
        _ => anyhow::bail!("unsupported fixture membership"),
    }
    Ok(())
}
async fn title(db: &sea_orm::DatabaseConnection, id: i32, value: &str) -> Result<()> {
    media_metadata::Entity::insert(media_metadata::ActiveModel {
        file_id: Set(id),
        hlc_uuid: Set(Uuid::new_v4().to_string()),
        position: Set(0),
        meta_key: Set("track_title".into()),
        meta_value: Set(value.into()),
        ..Default::default()
    })
    .exec(db)
    .await?;
    Ok(())
}
fn peer(library: &LibrarySyncStore) -> support::Peer {
    Arc::new(library.endpoint())
}
async fn transfer(a: &LibrarySyncStore, b: &LibrarySyncStore) -> Result<()> {
    support::exchange(&peer(a), &peer(b)).await?;
    Ok(())
}
fn audio(title: impl Into<String>, blob: Option<BlobRef>) -> Payload {
    Payload::Track(Track {
        extension: "ogg".into(),
        content_hash: blob.as_ref().map(|b| b.digest.clone()),
        content_size: blob.map(|b| b.size.try_into().unwrap()),
        duration: "0".to_owned().try_into().unwrap(),
        sample_rate: 0,
        cover_art_id: None,
        metadata: vec![MetadataTag {
            id: Uuid::new_v4(),
            position: 0,
            key: "track_title".into(),
            value: title.into(),
        }],
    })
}
#[tokio::test]
async fn business_actions_replicate_with_distinct_local_keys_and_deletions() -> Result<()> {
    let (db1, _root1) = setup().await?;
    let (db2, _root2) = setup().await?;
    file(&db1, 10).await?;
    file(&db2, 77).await?;
    let track_id = Uuid::new_v4();
    for (db, id) in [(&db1, 10), (&db2, 77)] {
        media_files::Entity::update_many()
            .col_expr(
                media_files::Column::HlcUuid,
                Expr::value(track_id.to_string()),
            )
            .filter(media_files::Column::Id.eq(id))
            .exec(db)
            .await?;
    }
    let lib = Uuid::new_v4();
    let node = Uuid::new_v4();
    let a = LibrarySyncStore::open(db1.clone(), node, Some(lib)).await?;
    let b = LibrarySyncStore::open(db2.clone(), Uuid::new_v4(), Some(lib)).await?;
    a.import_existing(a.record_store().author()).await?;
    b.head().await?;
    set_liked(&db1, &a.record_store(), 10, true).await?;

    let playlist = create_playlist(
        &db1,
        &a.record_store(),
        node,
        "Likes list".into(),
        "group".into(),
    )
    .await?;
    add_item_to_playlist(&db1, &a.record_store(), node, playlist.id, 10, None).await?;
    transfer(&a, &b).await?;
    assert!(get_liked(&db2, 77).await?);
    let projected = get_all_playlists(&db2).await?;
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].name, "Likes list");
    assert_eq!(i64::from(entry_file(&db2).await?), i64::from(77));
    increase_played_through(&db1, &a.record_store(), 10, a.record_store().author()).await?;
    transfer(&a, &b).await?;
    transfer(&a, &b).await?;
    assert_eq!(played(&db2, 77).await?, 1);
    database::actions::playlists::remove_playlist(&db1, &a.record_store(), playlist.id).await?;
    transfer(&a, &b).await?;
    assert!(get_all_playlists(&db2).await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn pending_track_dependencies_project_after_scanning() -> Result<()> {
    let (db1, _root1) = setup().await?;
    let (db2, root2) = setup().await?;
    file(&db1, 10).await?;
    let lib = Uuid::new_v4();
    let node = Uuid::new_v4();
    let a = LibrarySyncStore::open(db1.clone(), node, Some(lib)).await?;
    let b = LibrarySyncStore::open(db2.clone(), Uuid::new_v4(), Some(lib)).await?;
    a.import_existing(a.record_store().author()).await?;
    set_liked(&db1, &a.record_store(), 10, true).await?;

    let playlist = create_playlist(
        &db1,
        &a.record_store(),
        node,
        "Pending".into(),
        "group".into(),
    )
    .await?;
    add_item_to_playlist(&db1, &a.record_store(), node, playlist.id, 10, None).await?;
    transfer(&a, &b).await?;
    assert_eq!(get_all_playlists(&db2).await?.len(), 1);
    let marker = marker(&db2).await?;
    assert!(get_liked(&db2, marker).await?);
    let (track, blob, _) = database::actions::file::catalog_track(&db2, marker)
        .await?
        .unwrap();
    let target = database::actions::downloads::prepare_download(
        &db2,
        &FsIo::new(),
        root2.path(),
        track,
        &blob,
        "ogg",
    )
    .await?;
    std::fs::copy(root2.path().join("song.ogg"), &target)?;
    database::actions::file::attach_download(
        &db2,
        &b.record_store(),
        &FsIo::new(),
        root2.path(),
        track,
        &blob,
        &target,
    )
    .await?;
    assert!(get_liked(&db2, marker).await?);
    assert_eq!(i64::from(entry_file(&db2).await?), i64::from(marker));
    Ok(())
}

#[tokio::test]
async fn albums_and_relationships_project_after_track_arrival() -> Result<()> {
    let (db1, _root1) = setup().await?;
    let (db2, root2) = setup().await?;
    file(&db1, 10).await?;
    parent(&db1, Collection::Albums, 42, "Album", "Artist").await?;
    member(&db1, Collection::Albums, 10, 42, Some(3)).await?;
    let lib = Uuid::new_v4();
    let a = LibrarySyncStore::open(db1.clone(), Uuid::new_v4(), Some(lib)).await?;
    let b = LibrarySyncStore::open(db2.clone(), Uuid::new_v4(), Some(lib)).await?;
    a.import_existing(a.record_store().author()).await?;
    transfer(&a, &b).await?;
    let marker = marker(&db2).await?;
    let (track, blob, _) = database::actions::file::catalog_track(&db2, marker)
        .await?
        .unwrap();
    let target = database::actions::downloads::prepare_download(
        &db2,
        &FsIo::new(),
        root2.path(),
        track,
        &blob,
        "ogg",
    )
    .await?;
    std::fs::copy(root2.path().join("song.ogg"), &target)?;
    database::actions::file::attach_download(
        &db2,
        &b.record_store(),
        &FsIo::new(),
        root2.path(),
        track,
        &blob,
        &target,
    )
    .await?;
    let link = media_file_albums::Entity::find().one(&db2).await?.unwrap();
    assert_eq!(link.track_uuid, track.to_string());
    assert_eq!(link.track_number, Some(3));
    let album = link.find_related(albums::Entity).one(&db2).await?.unwrap();
    assert_eq!(album.name, "Album");
    Ok(())
}

#[tokio::test]
async fn cache_eviction_preserves_replicated_likes_and_entries() -> Result<()> {
    let (db, _root) = setup().await?;
    file(&db, 10).await?;
    let node = Uuid::new_v4();
    let store = LibrarySyncStore::open(db.clone(), node, None).await?;
    set_liked(&db, &store.record_store(), 10, true).await?;
    let playlist = create_playlist(
        &db,
        &store.record_store(),
        node,
        "Keep".into(),
        "group".into(),
    )
    .await?;
    add_item_to_playlist(&db, &store.record_store(), node, playlist.id, 10, None).await?;
    store.head().await?;
    let head = store.head().await?;
    media_files::Entity::update_many()
        .col_expr(media_files::Column::LocalAvailable, Expr::value(false))
        .filter(media_files::Column::Id.eq(10))
        .exec(&db)
        .await?;
    assert_eq!(store.head().await?, head);
    assert!(get_liked(&db, 10).await?);
    assert_eq!(i64::from(entry_file(&db).await?), i64::from(10));
    Ok(())
}

#[tokio::test]
async fn library_namespace_persists_and_mismatch_preserves_state() -> Result<()> {
    let (db, _root) = setup().await?;
    let _fs = Arc::new(FsIo::new());
    let node = Uuid::new_v4();
    let namespace = Uuid::new_v4();
    let store = LibrarySyncStore::open(db.clone(), node, Some(namespace)).await?;
    let track_id = Uuid::new_v4();
    store
        .write_local(
            ObjectKey {
                collection: Collection::Tracks,
                id: track_id,
            },
            Some(audio("record", None)),
        )
        .await?;
    let head = store.head().await?;
    assert!(
        LibrarySyncStore::open(db.clone(), node, Some(Uuid::new_v4()))
            .await
            .is_err()
    );
    let reopened = LibrarySyncStore::open(db, node, None).await?;
    assert_eq!(reopened.identity().await?.namespace, namespace);
    assert_eq!(reopened.head().await?, head);
    assert!(
        reopened
            .get(&ObjectKey {
                collection: Collection::Tracks,
                id: track_id
            })
            .await?
            .is_some()
    );
    Ok(())
}

#[tokio::test]
async fn unrelated_namespace_is_rejected_without_erasure() -> Result<()> {
    let (db1, _root1) = setup().await?;
    let (db2, _root2) = setup().await?;
    let _fs = Arc::new(FsIo::new());
    let a = LibrarySyncStore::open(db1, Uuid::new_v4(), Some(Uuid::new_v4())).await?;
    let b = LibrarySyncStore::open(db2, Uuid::new_v4(), Some(Uuid::new_v4())).await?;
    let track_id = Uuid::new_v4();
    a.write_local(
        ObjectKey {
            collection: Collection::Tracks,
            id: track_id,
        },
        Some(audio("record", None)),
    )
    .await?;
    let head = a.head().await?;
    assert!(transfer(&a, &b).await.is_err());
    assert_eq!(a.head().await?, head);
    assert!(
        a.get(&ObjectKey {
            collection: Collection::Tracks,
            id: track_id
        })
        .await?
        .is_some()
    );
    assert_eq!(b.head().await?.sequence, 0);
    Ok(())
}

#[tokio::test]
async fn failed_business_projection_rolls_back_records_and_progress() -> Result<()> {
    let (db1, _root1) = setup().await?;
    let (db2, _root2) = setup().await?;
    file(&db1, 10).await?;
    file(&db2, 77).await?;
    let track_id = Uuid::new_v4();
    for (db, id) in [(&db1, 10), (&db2, 77)] {
        media_files::Entity::update_many()
            .col_expr(
                media_files::Column::HlcUuid,
                Expr::value(track_id.to_string()),
            )
            .filter(media_files::Column::Id.eq(id))
            .exec(db)
            .await?;
    }
    let _fs = Arc::new(FsIo::new());
    let lib = Uuid::new_v4();
    let a = LibrarySyncStore::open(db1.clone(), Uuid::new_v4(), Some(lib)).await?;
    let b = LibrarySyncStore::open(db2.clone(), Uuid::new_v4(), Some(lib)).await?;
    a.import_existing(a.record_store().author()).await?;
    b.head().await?;
    set_liked(&db1, &a.record_store(), 10, true).await?;
    db2.execute_unprepared("CREATE TRIGGER reject_liked BEFORE INSERT ON media_file_likes BEGIN SELECT RAISE(ABORT,'injected projection failure'); END").await?;
    assert!(transfer(&a, &b).await.is_err());
    let source = a.identity().await?;
    let checkpoint = CheckpointKey {
        namespace: source.namespace,
        source_replica: source.replica,
        source_generation: source.generation,
        scope_id: sync::scope_id(&support::scope())?,
    };
    assert_eq!(b.endpoint().checkpoint(&checkpoint).await?, None);
    assert!(!get_liked(&db2, 77).await?);
    db2.execute_unprepared("DROP TRIGGER reject_liked").await?;
    transfer(&a, &b).await?;
    assert!(get_liked(&db2, 77).await?);
    Ok(())
}

#[tokio::test]
async fn separate_identical_files_keep_independent_likes() -> Result<()> {
    let (db, root) = setup().await?;
    file(&db, 10).await?;
    std::fs::write(root.path().join("second.ogg"), b"shared bytes")?;
    support::local_file(&db, 20, Uuid::new_v4(), "second.ogg").await?;
    parent(&db, Collection::Albums, 41, "First", "Artist").await?;
    parent(&db, Collection::Albums, 42, "Second", "Artist").await?;
    member(&db, Collection::Albums, 10, 41, Some(1)).await?;
    member(&db, Collection::Albums, 20, 42, Some(2)).await?;
    let store = LibrarySyncStore::open(db.clone(), Uuid::new_v4(), None).await?;
    set_liked(&db, &store.record_store(), 10, true).await?;
    store.head().await?;
    assert!(!get_liked(&db, 20).await?);
    let stable = store.head().await?;
    assert_eq!(store.head().await?, stable);
    increase_played_through(
        &db,
        &store.record_store(),
        20,
        store.record_store().author(),
    )
    .await?;
    let changed = store.head().await?;
    assert!(changed.sequence > stable.sequence);
    assert!(get_liked(&db, 10).await?);
    assert!(!get_liked(&db, 20).await?);
    assert_eq!(store.head().await?, changed);
    Ok(())
}

#[tokio::test]
async fn stale_download_does_not_replace_current_track_mapping() -> Result<()> {
    let (db, root) = setup().await?;
    file(&db, 10).await?;
    let fs = Arc::new(FsIo::new());
    let store = LibrarySyncStore::open(db.clone(), Uuid::new_v4(), None).await?;
    store.head().await?;
    let track = Uuid::new_v4();
    let old = database::actions::file::hash_file(&fs, &root.path().join("song.ogg"))?;
    let newer = BlobRef {
        digest: "a".repeat(64),
        size: old.size,
    };
    store
        .write_local(
            ObjectKey {
                collection: Collection::Tracks,
                id: track,
            },
            Some(audio("new", Some(newer))),
        )
        .await?;
    assert!(
        database::actions::file::attach_download(
            &db,
            &store.record_store(),
            &FsIo::new(),
            root.path(),
            track,
            &old,
            &root.path().join("song.ogg")
        )
        .await
        .is_err()
    );
    let track_id = track_uuid(&db, 10).await?;
    assert_ne!(track_id, track.to_string());
    Ok(())
}

#[tokio::test]
async fn attaching_changed_content_keeps_playlist_and_identity() -> Result<()> {
    let (db, root) = setup().await?;
    file(&db, 10).await?;
    let fs = Arc::new(FsIo::new());
    let node = Uuid::new_v4();
    let store = LibrarySyncStore::open(db.clone(), node, None).await?;
    store.import_existing(node).await?;

    let list = create_playlist(
        &db,
        &store.record_store(),
        node,
        "Changed".into(),
        "group".into(),
    )
    .await?;
    add_item_to_playlist(&db, &store.record_store(), node, list.id, 10, None).await?;
    let file = database::entities::media_files::Entity::find_by_id(10)
        .one(&db)
        .await?
        .unwrap();
    let track: Uuid = file.hlc_uuid.parse()?;
    let original = root.path().join("updated.ogg");
    std::fs::write(&original, b"new bytes")?;
    let blob = database::actions::file::hash_file(&fs, &original)?;
    store
        .write_local(
            ObjectKey {
                collection: Collection::Tracks,
                id: track,
            },
            Some(audio("changed", Some(blob.clone()))),
        )
        .await?;
    let path =
        database::actions::downloads::prepare_download(&db, &fs, root.path(), track, &blob, "ogg")
            .await?;
    std::fs::copy(&original, &path)?;
    db.execute_unprepared("CREATE TRIGGER reject_attach BEFORE UPDATE ON media_files WHEN OLD.id=10 BEGIN SELECT RAISE(ABORT,'injected attachment failure'); END").await?;
    assert!(
        database::actions::file::attach_download(
            &db,
            &store.record_store(),
            &FsIo::new(),
            root.path(),
            track,
            &blob,
            &path
        )
        .await
        .is_err()
    );
    assert_eq!(
        database::actions::file::resolve_local_id(&db, 10).await?,
        Some(10)
    );
    db.execute_unprepared("DROP TRIGGER reject_attach").await?;
    database::actions::file::attach_download(
        &db,
        &store.record_store(),
        &FsIo::new(),
        root.path(),
        track,
        &blob,
        &path,
    )
    .await?;
    assert_eq!(
        database::actions::file::materialized_path(&db, &FsIo::new(), root.path(), 10).await?,
        Some(path)
    );
    assert_eq!(i64::from(entry_file(&db).await?), i64::from(10));
    assert_eq!(
        database::entities::media_files::Entity::find_by_id(10)
            .one(&db)
            .await?
            .unwrap()
            .hlc_uuid,
        track.to_string()
    );
    Ok(())
}

#[tokio::test]
async fn changed_content_sync_preserves_track_identity_and_playlist() -> Result<()> {
    let (db1, root1) = setup().await?;
    let (db2, root2) = setup().await?;
    file(&db1, 10).await?;
    file(&db2, 77).await?;
    let track = Uuid::new_v4();
    for (db, id) in [(&db1, 10), (&db2, 77)] {
        media_files::Entity::update_many()
            .col_expr(media_files::Column::HlcUuid, Expr::value(track.to_string()))
            .filter(media_files::Column::Id.eq(id))
            .exec(db)
            .await?;
    }
    let namespace = Uuid::new_v4();
    let node1 = Uuid::new_v4();
    let node2 = Uuid::new_v4();
    let fs = Arc::new(FsIo::new());
    let a = LibrarySyncStore::open(db1.clone(), node1, Some(namespace)).await?;
    let b = LibrarySyncStore::open(db2.clone(), node2, Some(namespace)).await?;
    a.import_existing(node1).await?;

    let list = create_playlist(
        &db1,
        &a.record_store(),
        node1,
        "Changes".into(),
        "group".into(),
    )
    .await?;
    add_item_to_playlist(&db1, &a.record_store(), node1, list.id, 10, None).await?;
    transfer(&a, &b).await?;
    let updated = root1.path().join("updated.ogg");
    std::fs::write(&updated, b"new changed bytes")?;
    let blob = database::actions::file::hash_file(&fs, &updated)?;
    a.write_local(
        ObjectKey {
            collection: Collection::Tracks,
            id: track,
        },
        Some(audio("updated", Some(blob.clone()))),
    )
    .await?;
    transfer(&a, &b).await?;
    assert!(!database::actions::file::is_materialized(&db2, &FsIo::new(), root2.path(), 77).await?);
    assert!(
        database::actions::file::remote_required_ids(&db2, &FsIo::new(), root2.path(), &[77])
            .await?
            .contains(&77)
    );
    let path = database::actions::downloads::prepare_download(
        &db2,
        &fs,
        root2.path(),
        track,
        &blob,
        "ogg",
    )
    .await?;
    std::fs::copy(&updated, &path)?;
    database::actions::file::attach_download(
        &db2,
        &b.record_store(),
        &FsIo::new(),
        root2.path(),
        track,
        &blob,
        &path,
    )
    .await?;
    assert_eq!(
        database::actions::file::resolve_local_id(&db2, 77).await?,
        Some(77)
    );
    assert_eq!(
        database::actions::file::materialized_path(&db2, &FsIo::new(), root2.path(), 77).await?,
        Some(path)
    );
    assert_eq!(i64::from(entry_file(&db2).await?), i64::from(77));
    set_liked(&db2, &b.record_store(), 77, true).await?;
    increase_played_through(&db2, &b.record_store(), 77, node2).await?;
    assert!(get_liked(&db2, 77).await?);
    assert_eq!(played(&db2, 77).await?, 1);
    assert!(
        !database::actions::file::remote_required_ids(&db2, &FsIo::new(), root2.path(), &[77])
            .await?
            .contains(&77)
    );
    Ok(())
}

#[tokio::test]
async fn attachment_keeps_player_references_across_scans() -> Result<()> {
    use database::actions::{
        file::get_media_files,
        metadata::scan_audio_library,
        playback_queue::{list_playback_queue, replace_playback_queue},
    };
    let root = tempfile::tempdir()?;
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let original = std::fs::read(fixtures.join("startup_0.ogg"))?;
    std::fs::write(root.path().join("original.ogg"), &original)?;
    let mut options = sea_orm::ConnectOptions::new("sqlite::memory:");
    options
        .max_connections(1)
        .min_connections(1)
        .idle_timeout(None)
        .max_lifetime(None);
    let db = Database::connect(options).await?;
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
    )
    .await?;
    let old = get_media_files(&db, 0, 10).await?.pop().unwrap();
    let track: Uuid = old.hlc_uuid.parse()?;

    store.import_existing(node).await?;
    set_liked(&db, &store.record_store(), old.id, true).await?;
    increase_played_through(&db, &store.record_store(), old.id, node).await?;
    replace_playback_queue(&db, vec![old.id, old.id]).await?;

    let playlist = create_playlist(
        &db,
        &store.record_store(),
        node,
        "Playback".into(),
        "group".into(),
    )
    .await?;
    add_item_to_playlist(&db, &store.record_store(), node, playlist.id, old.id, None).await?;
    database::actions::search::add_term(
        &db,
        database::actions::collection::CollectionQueryType::Track,
        old.id,
        "uniquesupersededname",
    )
    .await?;
    let search = database::actions::search::search_for(
        &db,
        "uniquesupersededname",
        Some(vec![
            database::actions::collection::CollectionQueryType::Track,
        ]),
        10,
    )
    .await?;
    assert!(
        search
            .values()
            .any(|items| items.contains(&i64::from(old.id)))
    );
    let updated = fixtures.join("startup_1.ogg");
    let blob = database::actions::file::hash_file(&fs, &updated)?;
    store
        .write_local(
            ObjectKey {
                collection: Collection::Tracks,
                id: track,
            },
            Some(audio("updated", Some(blob.clone()))),
        )
        .await?;
    let path =
        database::actions::downloads::prepare_download(&db, &fs, root.path(), track, &blob, "ogg")
            .await?;
    std::fs::copy(updated, &path)?;
    database::actions::file::attach_download(
        &db,
        &store.record_store(),
        &FsIo::new(),
        root.path(),
        track,
        &blob,
        &path,
    )
    .await?;
    for force in [false, true] {
        scan_audio_library(
            &fs,
            &db,
            &node.to_string(),
            root.path(),
            true,
            force,
            |_| {},
            None,
            &store.record_store(),
        )
        .await?;
        assert_eq!(list_playback_queue(&db).await?, vec![old.id, old.id]);
        assert!(get_liked(&db, old.id).await?);
        assert_eq!(
            database::actions::file::resolve_local_id(&db, old.id).await?,
            Some(old.id)
        );
        assert_eq!(std::fs::read(root.path().join("original.ogg"))?, original);
    }
    let search = database::actions::search::search_for(
        &db,
        "uniquesupersededname",
        Some(vec![
            database::actions::collection::CollectionQueryType::Track,
        ]),
        10,
    )
    .await?;
    assert!(
        search
            .values()
            .all(|items| !items.contains(&i64::from(old.id)))
    );
    assert_eq!(i64::from(entry_file(&db).await?), i64::from(old.id));
    assert_eq!(played(&db, old.id).await?, 1);
    Ok(())
}

#[tokio::test]
async fn metadata_only_tracks_are_browsable_and_materialize_in_place() -> Result<()> {
    use database::{actions::mixes::query_mix_media_files, connection::RecommendationDbConnection};
    let (db1, _root1) = setup().await?;
    let (db2, root2) = setup().await?;
    file(&db1, 10).await?;
    parent(&db1, Collection::Artists, 11, "Catalog Artist", "C").await?;
    parent(&db1, Collection::Genres, 12, "Catalog Genre", "C").await?;
    parent(&db1, Collection::Albums, 13, "Catalog Album", "C").await?;
    member(&db1, Collection::Artists, 10, 11, None).await?;
    member(&db1, Collection::Genres, 10, 12, None).await?;
    member(&db1, Collection::Albums, 10, 13, Some(2)).await?;
    title(&db1, 10, "Catalog Title").await?;
    let lib = Uuid::new_v4();
    let fs = Arc::new(FsIo::new());
    let a = LibrarySyncStore::open(db1, Uuid::new_v4(), Some(lib)).await?;
    let b = LibrarySyncStore::open(db2.clone(), Uuid::new_v4(), Some(lib)).await?;
    a.import_existing(a.record_store().author()).await?;
    transfer(&a, &b).await?;
    let id = marker(&db2).await?;
    assert!(!database::actions::file::is_materialized(&db2, &FsIo::new(), root2.path(), id).await?);
    let (track, blob, _) = database::actions::file::catalog_track(&db2, id)
        .await?
        .unwrap();
    let artist = artists::Entity::find().one(&db2).await?.unwrap();
    let genre = genres::Entity::find().one(&db2).await?.unwrap();
    let album = albums::Entity::find().one(&db2).await?.unwrap();
    let recommend = RecommendationDbConnection::new();
    for (op, filter) in [
        ("lib::artist", artist.id),
        ("lib::genre", genre.id),
        ("lib::album", album.id),
    ] {
        let found = query_mix_media_files(
            &db2,
            &recommend,
            vec![(op.into(), filter.to_string())],
            0,
            20,
        )
        .await?;
        assert_eq!(found.iter().map(|f| f.id).collect::<Vec<_>>(), vec![id]);
    }
    database::actions::metadata::scan_audio_library(
        &fs,
        &db2,
        &b.record_store().author().to_string(),
        root2.path(),
        true,
        false,
        |_| {},
        None,
        &b.record_store(),
    )
    .await?;
    assert!(
        database::actions::file::catalog_track(&db2, id)
            .await?
            .is_some()
    );
    let path = database::actions::downloads::prepare_download(
        &db2,
        &fs,
        root2.path(),
        track,
        &blob,
        "ogg",
    )
    .await?;
    std::fs::copy(root2.path().join("song.ogg"), &path)?;
    database::actions::file::attach_download(
        &db2,
        &b.record_store(),
        &FsIo::new(),
        root2.path(),
        track,
        &blob,
        &path,
    )
    .await?;
    assert!(database::actions::file::is_materialized(&db2, &FsIo::new(), root2.path(), id).await?);
    assert_eq!(
        database::actions::file::catalog_track(&db2, id)
            .await?
            .map(|entry| entry.0),
        Some(track),
        "content identity remains available after local attachment"
    );
    assert!(
        database::actions::file::get_file_by_id(&db2, id)
            .await?
            .is_some()
    );
    assert_eq!(
        database::actions::file::resolve_local_id(&db2, id).await?,
        Some(id)
    );
    Ok(())
}

#[tokio::test]
async fn remote_metadata_edits_project_onto_existing_audio_without_feedback() -> Result<()> {
    let (db1, _root1) = setup().await?;
    let (db2, _root2) = setup().await?;
    file(&db1, 10).await?;
    file(&db2, 77).await?;
    let track = Uuid::new_v4();
    for (db, id) in [(&db1, 10), (&db2, 77)] {
        media_files::Entity::update_many()
            .col_expr(media_files::Column::HlcUuid, Expr::value(track.to_string()))
            .filter(media_files::Column::Id.eq(id))
            .exec(db)
            .await?;
    }
    let _fs = Arc::new(FsIo::new());
    let lib = Uuid::new_v4();
    let a = LibrarySyncStore::open(db1.clone(), Uuid::new_v4(), Some(lib)).await?;
    let b = LibrarySyncStore::open(db2.clone(), Uuid::new_v4(), Some(lib)).await?;
    a.import_existing(a.record_store().author()).await?;
    b.head().await?;
    title(&db1, 10, "Edited title").await?;
    parent(&db1, Collection::Artists, 11, "Edited Artist", "E").await?;
    member(&db1, Collection::Artists, 10, 11, None).await?;
    {
        let tx = db1.begin().await?;
        let artist = artists::Entity::find_by_id(11).one(&tx).await?.unwrap();
        let link = media_file_artists::Entity::find().one(&tx).await?.unwrap();
        for (collection, id) in [
            (Collection::Artists, artist.hlc_uuid),
            (Collection::FileArtists, link.hlc_uuid),
        ] {
            database::sync::model::author_source_on(
                &tx,
                &a.record_store(),
                ObjectKey {
                    collection,
                    id: id.parse()?,
                },
            )
            .await?;
        }
        tx.commit().await?;
    }

    {
        use sea_orm::TransactionTrait;
        let tx = db1.begin().await?;
        let row = database::entities::media_files::Entity::find_by_id(10)
            .one(&tx)
            .await?
            .unwrap();
        database::actions::metadata::author_indexed_track_row(&tx, &a.record_store(), row).await?;
        tx.commit().await?;
    }
    transfer(&a, &b).await?;
    let row = media_metadata::Entity::find()
        .filter(media_metadata::Column::FileId.eq(77))
        .filter(media_metadata::Column::MetaKey.eq("track_title"))
        .one(&db2)
        .await?
        .unwrap();
    assert_eq!(row.meta_value, "Edited title");
    let link = media_file_artists::Entity::find()
        .filter(media_file_artists::Column::TrackUuid.eq(track.to_string()))
        .one(&db2)
        .await?
        .unwrap();
    assert_eq!(
        link.find_related(artists::Entity)
            .one(&db2)
            .await?
            .unwrap()
            .name,
        "Edited Artist"
    );
    media_metadata::Entity::update_many()
        .col_expr(
            media_metadata::Column::MetaValue,
            Expr::value("Second title"),
        )
        .filter(media_metadata::Column::FileId.eq(10))
        .filter(media_metadata::Column::MetaKey.eq("track_title"))
        .exec(&db1)
        .await?;
    {
        use sea_orm::TransactionTrait;
        let tx = db1.begin().await?;
        let row = database::entities::media_files::Entity::find_by_id(10)
            .one(&tx)
            .await?
            .unwrap();
        database::actions::metadata::author_indexed_track_row(&tx, &a.record_store(), row).await?;
        tx.commit().await?;
    }
    transfer(&a, &b).await?;
    let link = media_file_artists::Entity::find()
        .filter(media_file_artists::Column::TrackUuid.eq(track.to_string()))
        .one(&db2)
        .await?
        .unwrap();
    assert_eq!(
        link.find_related(artists::Entity)
            .one(&db2)
            .await?
            .unwrap()
            .name,
        "Edited Artist"
    );
    let head = b.head().await?;
    assert_eq!(head, b.head().await?);
    Ok(())
}

#[tokio::test]
async fn cache_cleanup_keeps_distinct_catalog_ids_and_references() -> Result<()> {
    use database::actions::{
        file::get_media_files_count,
        metadata::scan_audio_library,
        playback_queue::{list_playback_queue, replace_playback_queue},
    };
    let (db, root) = setup().await?;
    file(&db, 10).await?;
    std::fs::write(root.path().join("second.ogg"), b"shared bytes")?;
    support::local_file(&db, 20, Uuid::new_v4(), "second.ogg").await?;
    let fs = Arc::new(FsIo::new());
    let node = Uuid::new_v4();
    let store = LibrarySyncStore::open(db.clone(), node, None).await?;
    let playlist = create_playlist(
        &db,
        &store.record_store(),
        node,
        "Eviction".into(),
        "group".into(),
    )
    .await?;
    add_item_to_playlist(&db, &store.record_store(), node, playlist.id, 10, None).await?;
    replace_playback_queue(&db, vec![10]).await?;
    store.head().await?;
    std::fs::remove_file(root.path().join("song.ogg"))?;
    scan_audio_library(
        &fs,
        &db,
        &node.to_string(),
        root.path(),
        true,
        false,
        |_| {},
        None,
        &store.record_store(),
    )
    .await?;
    assert_eq!(get_media_files_count(&db).await?, 2);
    assert_eq!(list_playback_queue(&db).await?, vec![10]);
    assert_eq!(i64::from(entry_file(&db).await?), i64::from(10));
    std::fs::remove_file(root.path().join("second.ogg"))?;
    scan_audio_library(
        &fs,
        &db,
        &node.to_string(),
        root.path(),
        true,
        false,
        |_| {},
        None,
        &store.record_store(),
    )
    .await?;
    assert_eq!(get_media_files_count(&db).await?, 2);
    assert!(!database::actions::file::is_materialized(&db, &FsIo::new(), root.path(), 10).await?);
    assert_eq!(
        database::actions::file::resolve_local_id(&db, 10).await?,
        Some(10)
    );
    assert_eq!(list_playback_queue(&db).await?, vec![10]);
    Ok(())
}

#[tokio::test]
async fn saved_mix_uses_stable_references_across_local_ids() -> Result<()> {
    use database::{
        actions::mixes::{get_mix_queries_by_mix_id, query_mix_media_files},
        connection::RecommendationDbConnection,
    };
    let (db1, _root1) = setup().await?;
    let (db2, _root2) = setup().await?;
    file(&db1, 10).await?;
    file(&db2, 77).await?;
    let track = Uuid::new_v4();
    for (db, id) in [(&db1, 10), (&db2, 77)] {
        media_files::Entity::update_many()
            .col_expr(media_files::Column::HlcUuid, Expr::value(track.to_string()))
            .filter(media_files::Column::Id.eq(id))
            .exec(db)
            .await?;
    }
    parent(&db1, Collection::Artists, 11, "Shared Artist", "S").await?;
    parent(&db1, Collection::Genres, 12, "Shared Genre", "S").await?;
    parent(&db1, Collection::Albums, 13, "Shared Album", "S").await?;
    member(&db1, Collection::Artists, 10, 11, None).await?;
    member(&db1, Collection::Genres, 10, 12, None).await?;
    member(&db1, Collection::Albums, 10, 13, Some(2)).await?;
    parent(&db2, Collection::Artists, 11, "Other Artist", "O").await?;
    parent(&db2, Collection::Genres, 12, "Other Genre", "O").await?;
    parent(&db2, Collection::Albums, 13, "Other Album", "O").await?;
    let lib = Uuid::new_v4();
    let node = Uuid::new_v4();
    let _fs = Arc::new(FsIo::new());
    let a = LibrarySyncStore::open(db1.clone(), node, Some(lib)).await?;
    let b = LibrarySyncStore::open(db2.clone(), Uuid::new_v4(), Some(lib)).await?;
    a.import_existing(node).await?;

    let playlist = create_playlist(
        &db1,
        &a.record_store(),
        node,
        "Shared playlist".into(),
        "S".into(),
    )
    .await?;
    add_item_to_playlist(&db1, &a.record_store(), node, playlist.id, 10, None).await?;
    let clauses: Vec<(String, String)> = vec![
        ("lib::track".into(), "10".into()),
        ("lib::album".into(), "13".into()),
        ("lib::artist".into(), "11".into()),
        ("lib::genre".into(), "12".into()),
        ("lib::playlist".into(), playlist.id.to_string()),
        ("filter::liked".into(), "true".into()),
    ];
    let mix = database::actions::mixes::create_mix(
        &db1,
        &a.record_store(),
        &node.to_string(),
        "Portable mix".into(),
        "S".into(),
        false,
        2,
        false,
        clauses.clone(),
    )
    .await?;
    database::actions::mixes::replace_mix_queries(
        &db1,
        &a.record_store(),
        mix.id,
        &mix.hlc_uuid,
        clauses,
        Some(3),
    )
    .await?;
    set_liked(&db1, &a.record_store(), 10, true).await?;
    transfer(&a, &b).await?;
    let mix = mixes::Entity::find()
        .filter(mixes::Column::Name.eq("Portable mix"))
        .one(&db2)
        .await?
        .unwrap();
    let local = mix.id;
    let queries = get_mix_queries_by_mix_id(&db2, local).await?;
    assert_eq!(queries.len(), 6);
    let stored = mix_queries::Entity::find()
        .filter(mix_queries::Column::MixUuid.eq(&mix.hlc_uuid))
        .all(&db2)
        .await?;
    assert!(stored.iter().all(|q| q.group == 3));
    let params = queries
        .iter()
        .map(|q| (q.operator.clone(), q.parameter.clone()))
        .collect::<Vec<_>>();
    assert!(params.contains(&("lib::track".into(), "77".into())));
    assert_ne!(
        params.iter().find(|q| q.0 == "lib::artist").unwrap().1,
        "11"
    );
    assert_ne!(params.iter().find(|q| q.0 == "lib::genre").unwrap().1, "12");
    assert_ne!(params.iter().find(|q| q.0 == "lib::album").unwrap().1, "13");
    let found =
        query_mix_media_files(&db2, &RecommendationDbConnection::new(), params, 0, 20).await?;
    assert_eq!(found.iter().map(|f| f.id).collect::<Vec<_>>(), vec![77]);
    let head = b.head().await?;
    assert_eq!(b.head().await?, head);
    Ok(())
}

#[tokio::test]
async fn mix_waits_for_dependencies_and_directory_paths_stay_local() -> Result<()> {
    let (db, _root) = setup().await?;
    let lib = Uuid::new_v4();
    let target = LibrarySyncStore::open(db.clone(), Uuid::new_v4(), Some(lib)).await?;
    let source = support::memory(lib, Uuid::new_v4())?;
    let track = Uuid::new_v4();
    let mix = Uuid::new_v4();
    source
        .write_local(
            ObjectKey {
                collection: Collection::Mixes,
                id: mix,
            },
            Some(Payload::Mix(Mix {
                name: "Await track".into(),
                group: "Group".into(),
                mode: Some(1),
                scriptlet_mode: false,
                locked: false,
                queries: vec![MixClause {
                    operator: "lib::track".into(),
                    group: 0,
                    parameter: String::new(),
                    target_uuid: Some(track),
                    id: Uuid::new_v4(),
                    position: 0,
                }],
            })),
        )
        .await?;
    let scope = Scope {
        collections: [Collection::Mixes, Collection::Tracks]
            .into_iter()
            .collect(),
    };
    support::exchange_scoped(&source, &peer(&target), scope.clone()).await?;
    assert!(
        mixes::Entity::find()
            .filter(mixes::Column::Name.eq("Await track"))
            .one(&db)
            .await?
            .is_some()
    );
    let pending = target.pending_mixes().await?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].0, mix);
    assert!(pending[0].1.contains(&track.to_string()));
    source
        .write_local(
            ObjectKey {
                collection: Collection::Tracks,
                id: track,
            },
            Some(audio("Later", None)),
        )
        .await?;
    support::exchange_scoped(&source, &peer(&target), scope.clone()).await?;
    assert!(
        mixes::Entity::find()
            .filter(mixes::Column::Name.eq("Await track"))
            .one(&db)
            .await?
            .is_some()
    );
    assert!(target.pending_mixes().await?.is_empty());
    database::actions::mixes::create_mix(
        &db,
        &target.record_store(),
        &target.record_store().author().to_string(),
        "Private directory".into(),
        "Group".into(),
        false,
        0,
        false,
        vec![("lib::directory.deep".into(), "/private/music".into())],
    )
    .await?;
    let head = target.head().await?;
    let page = target
        .endpoint()
        .read_page(PageRequest {
            source: target.identity().await?,
            scope: Scope {
                collections: [Collection::Mixes].into_iter().collect(),
            },
            after: sync::ReceiveProgress {
                sequence: 0,
                anchor: sync::initial_anchor(&target.identity().await?)?,
            },
            through: head.clone(),
            limits: support::limits().pages,
        })
        .await?;
    assert!(!serde_json::to_string(&page)?.contains("/private/music"));
    assert!(serde_json::to_string(&page)?.contains("lib::directory.deep"));
    Ok(())
}

#[tokio::test]
async fn streaming_lookup_uses_verified_signature_and_rechecks_changes() -> Result<()> {
    let (db, root) = setup().await?;
    file(&db, 10).await?;
    let fs = Arc::new(FsIo::new());
    let store = LibrarySyncStore::open(db.clone(), Uuid::new_v4(), None).await?;
    store.head().await?;
    let blob = database::actions::file::hash_file(&fs, &root.path().join("song.ogg"))?;
    assert_eq!(
        database::actions::file::cached_blob_path(&db, &FsIo::new(), root.path(), &blob.digest)
            .await?,
        Some(root.path().join("song.ogg"))
    );
    std::fs::write(root.path().join("song.ogg"), b"different bytes")?;
    media_files::Entity::update_many()
        .col_expr(media_files::Column::LastModified, Expr::value("changed"))
        .filter(media_files::Column::Id.eq(10))
        .exec(&db)
        .await?;
    assert_eq!(
        database::actions::file::cached_blob_path(&db, &FsIo::new(), root.path(), &blob.digest)
            .await?,
        None
    );
    Ok(())
}

#[tokio::test]
async fn pending_projection_survives_database_restart() -> Result<()> {
    let root = tempfile::tempdir()?;
    let dbfile = root.path().join("catalog.sqlite");
    let url = format!("sqlite://{}?mode=rwc", dbfile.display());
    let db = Database::connect(&url).await?;
    let _ = migration::initialize_node_id(Uuid::new_v4().to_string());
    Migrator::up(&db, None).await?;
    let lib = Uuid::new_v4();
    let node = Uuid::new_v4();
    let _fs = Arc::new(FsIo::new());
    let target = LibrarySyncStore::open(db.clone(), node, Some(lib)).await?;
    let source = support::memory(lib, Uuid::new_v4())?;
    let track = Uuid::new_v4();
    let mix = Uuid::new_v4();
    source
        .write_local(
            ObjectKey {
                collection: Collection::Mixes,
                id: mix,
            },
            Some(Payload::Mix(Mix {
                name: "Pending restart".into(),
                group: "Group".into(),
                mode: Some(1),
                scriptlet_mode: false,
                locked: false,
                queries: vec![MixClause {
                    operator: "lib::track".into(),
                    group: 0,
                    parameter: String::new(),
                    target_uuid: Some(track),
                    id: Uuid::new_v4(),
                    position: 0,
                }],
            })),
        )
        .await?;
    let scope = Scope {
        collections: [Collection::Mixes, Collection::Tracks]
            .into_iter()
            .collect(),
    };
    support::exchange_scoped(&source, &peer(&target), scope.clone()).await?;
    assert_eq!(target.pending_mixes().await?.len(), 1);
    drop(target);
    db.close().await?;
    let db = Database::connect(&url).await?;
    let target = LibrarySyncStore::open(db.clone(), node, Some(lib)).await?;
    assert_eq!(target.pending_mixes().await?.len(), 1);
    source
        .write_local(
            ObjectKey {
                collection: Collection::Tracks,
                id: track,
            },
            Some(audio("Parent", None)),
        )
        .await?;
    support::exchange_scoped(&source, &peer(&target), scope.clone()).await?;
    assert!(target.pending_mixes().await?.is_empty());
    assert!(
        mixes::Entity::find()
            .filter(mixes::Column::Name.eq("Pending restart"))
            .one(&db)
            .await?
            .is_some()
    );
    Ok(())
}

#[tokio::test]
async fn received_record_materializes_without_head_side_effects() -> Result<()> {
    let (db, _root) = setup().await?;
    let lib = Uuid::new_v4();
    let target = LibrarySyncStore::open(db.clone(), Uuid::new_v4(), Some(lib)).await?;
    let source = support::memory(lib, Uuid::new_v4())?;
    let track = Uuid::new_v4();
    source
        .write_local(
            ObjectKey {
                collection: Collection::Tracks,
                id: track,
            },
            Some(audio("Materialized", None)),
        )
        .await?;
    let scope = Scope {
        collections: [Collection::Tracks].into_iter().collect(),
    };
    support::exchange_scoped(&source, &peer(&target), scope.clone()).await?;
    assert!(
        media_files::Entity::find()
            .filter(media_files::Column::HlcUuid.eq(track.to_string()))
            .one(&db)
            .await?
            .is_some()
    );
    let head = target.head().await?;
    assert_eq!(target.head().await?, head);
    assert!(target.pending_references().await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn catalog_track_removal_finishes_projection_once() -> Result<()> {
    let (db, _root) = setup().await?;
    let lib = Uuid::new_v4();
    let target = LibrarySyncStore::open(db.clone(), Uuid::new_v4(), Some(lib)).await?;
    let source = support::memory(lib, Uuid::new_v4())?;
    let track = Uuid::new_v4();
    let key = ObjectKey {
        collection: Collection::Tracks,
        id: track,
    };
    source
        .write_local(key.clone(), Some(audio("Removed", None)))
        .await?;
    let scope = Scope {
        collections: [Collection::Tracks].into_iter().collect(),
    };
    support::exchange_scoped(&source, &peer(&target), scope.clone()).await?;
    source.write_local(key, None).await?;
    support::exchange_scoped(&source, &peer(&target), scope.clone()).await?;
    assert!(
        media_files::Entity::find()
            .filter(media_files::Column::HlcUuid.eq(track.to_string()))
            .one(&db)
            .await?
            .is_none()
    );
    for _ in 0..2 {
        target.head().await?;
        assert!(target.pending_references().await?.is_empty());
    }
    Ok(())
}

#[tokio::test]
async fn attached_track_id_remains_stable_after_database_reopen() -> Result<()> {
    let root = tempfile::tempdir()?;
    let url = format!(
        "sqlite://{}?mode=rwc",
        root.path().join("library.sqlite").display()
    );
    std::fs::write(root.path().join("song.ogg"), b"old bytes")?;
    let db = Database::connect(&url).await?;
    let _ = migration::initialize_node_id(Uuid::new_v4().to_string());
    Migrator::up(&db, None).await?;
    file(&db, 10).await?;
    let node = Uuid::new_v4();
    let lib = Uuid::new_v4();
    let fs = Arc::new(FsIo::new());
    let store = LibrarySyncStore::open(db.clone(), node, Some(lib)).await?;
    store.head().await?;
    let track_id = track_uuid(&db, 10).await?;
    let track: Uuid = track_id.parse()?;
    let path = root.path().join("updated.ogg");
    std::fs::write(&path, b"updated bytes")?;
    let blob = database::actions::file::hash_file(&fs, &path)?;
    store
        .write_local(
            ObjectKey {
                collection: Collection::Tracks,
                id: track,
            },
            Some(audio(blob.digest.clone(), Some(blob.clone()))),
        )
        .await?;
    let destination =
        database::actions::downloads::prepare_download(&db, &fs, root.path(), track, &blob, "ogg")
            .await?;
    std::fs::copy(&path, &destination)?;
    let path = destination;
    database::actions::file::attach_download(
        &db,
        &store.record_store(),
        &FsIo::new(),
        root.path(),
        track,
        &blob,
        &path,
    )
    .await?;
    assert_eq!(
        database::actions::file::materialized_path(&db, &FsIo::new(), root.path(), 10).await?,
        Some(path.clone())
    );
    drop(store);
    db.close().await?;
    let db = Database::connect(&url).await?;
    let _store = LibrarySyncStore::open(db.clone(), node, Some(lib)).await?;
    assert_eq!(
        database::actions::file::resolve_local_id(&db, 10).await?,
        Some(10)
    );
    assert_eq!(
        database::actions::file::materialized_path(&db, &FsIo::new(), root.path(), 10).await?,
        Some(path)
    );
    Ok(())
}

#[tokio::test]
async fn staged_download_remains_queryable_but_not_playable() -> Result<()> {
    use database::{
        actions::{
            collection::CollectionQueryType,
            file::{
                get_file_by_id, get_files_by_ids, get_media_files, get_media_files_count,
                list_files,
            },
            mixes::query_mix_media_files,
            search::{add_term, search_for},
        },
        connection::RecommendationDbConnection,
    };
    let (db, root) = setup().await?;
    file(&db, 10).await?;
    let staged = Uuid::new_v4();
    let mut staged_row = support::local_file(&db, 20, staged, "staged.ogg")
        .await?
        .into_active_model();
    staged_row.local_available = Set(false);
    staged_row.update(&db).await?;
    let staged_blob = BlobRef {
        digest: "a".repeat(64),
        size: 1,
    };
    database::actions::downloads::prepare_download(
        &db,
        &FsIo::new(),
        root.path(),
        staged,
        &staged_blob,
        "ogg",
    )
    .await?;
    add_term(&db, CollectionQueryType::Track, 20, "hiddenstagedentry").await?;
    assert_eq!(get_media_files_count(&db).await?, 2);
    assert_eq!(
        get_media_files(&db, 0, 10)
            .await?
            .iter()
            .map(|f| f.id)
            .collect::<Vec<_>>(),
        vec![10, 20]
    );
    assert!(get_file_by_id(&db, 20).await?.is_some());
    assert_eq!(get_files_by_ids(&db, &[20]).await?.len(), 1);
    assert_eq!(list_files(&db, 10).await?.len(), 2);
    let mix = query_mix_media_files(
        &db,
        &RecommendationDbConnection::new(),
        vec![("lib::all".into(), "true".into())],
        0,
        10,
    )
    .await?;
    assert_eq!(mix.iter().map(|f| f.id).collect::<Vec<_>>(), vec![10, 20]);
    assert!(
        search_for(
            &db,
            "hiddenstagedentry",
            Some(vec![CollectionQueryType::Track]),
            10
        )
        .await?
        .values()
        .any(|items| items.contains(&20))
    );
    let _store = LibrarySyncStore::open(db.clone(), Uuid::new_v4(), None).await?;
    assert_eq!(
        database::actions::file::audio_availability(&db, &FsIo::new(), root.path(), &[20])
            .await?
            .get(&20),
        Some(&database::actions::file::AudioAvailability::ImportPending)
    );
    assert_eq!(
        database::actions::file::materialized_path(&db, &FsIo::new(), root.path(), 20).await?,
        None
    );
    assert_eq!(
        database::actions::file::resolve_local_id(&db, 20).await?,
        Some(20)
    );
    assert_eq!(database::actions::file::catalog_track(&db, 20).await?, None);
    Ok(())
}

#[tokio::test]
async fn availability_distinguishes_local_reference_missing_and_pending() -> Result<()> {
    use database::actions::file::AudioAvailability;
    let (db, root) = setup().await?;
    file(&db, 10).await?;
    let staged = Uuid::new_v4();
    let mut staged_row = support::local_file(&db, 20, staged, "staged.ogg")
        .await?
        .into_active_model();
    staged_row.local_available = Set(false);
    staged_row.update(&db).await?;
    let staged_blob = BlobRef {
        digest: "a".repeat(64),
        size: 1,
    };
    database::actions::downloads::prepare_download(
        &db,
        &FsIo::new(),
        root.path(),
        staged,
        &staged_blob,
        "ogg",
    )
    .await?;
    let store = LibrarySyncStore::open(db.clone(), Uuid::new_v4(), None).await?;
    let key = ObjectKey {
        collection: Collection::Tracks,
        id: Uuid::new_v4(),
    };
    store
        .write_local(
            key.clone(),
            Some(audio(
                "remote",
                Some(BlobRef {
                    digest: "a".repeat(64),
                    size: 10,
                }),
            )),
        )
        .await?;
    let remote = media_files::Entity::find()
        .filter(media_files::Column::HlcUuid.eq(key.id.to_string()))
        .one(&db)
        .await?
        .unwrap()
        .id;
    support::local_file(&db, 30, Uuid::new_v4(), "missing.ogg").await?;
    let states = database::actions::file::audio_availability(
        &db,
        &FsIo::new(),
        root.path(),
        &[10, 20, 30, remote],
    )
    .await?;
    assert_eq!(states.get(&10), Some(&AudioAvailability::Local));
    assert_eq!(states.get(&20), Some(&AudioAvailability::ImportPending));
    assert_eq!(states.get(&30), Some(&AudioAvailability::Unavailable));
    assert_eq!(states.get(&remote), Some(&AudioAvailability::RemoteOnly));
    assert_eq!(
        database::actions::file::materialized_path(&db, &FsIo::new(), root.path(), 20).await?,
        None
    );
    assert_eq!(
        database::actions::file::resolve_local_id(&db, 20).await?,
        Some(20)
    );
    Ok(())
}

#[tokio::test]
async fn explicit_catalog_membership_import_preserves_source_identity() -> Result<()> {
    use migration::catalog_identity::{self, ParentKind};
    let (db1, _root1) = setup().await?;
    let (db2, _root2) = setup().await?;
    file(&db1, 10).await?;
    let file = database::entities::media_files::Entity::find_by_id(10)
        .one(&db1)
        .await?
        .unwrap();
    let file_id: Uuid = file.hlc_uuid.parse()?;
    let artist = catalog_identity::parent(ParentKind::Artist, "Artist");
    let membership = catalog_identity::membership(file_id, ParentKind::Artist, artist);
    parent(&db1, Collection::Artists, 13, "Artist", "A").await?;
    member(&db1, Collection::Artists, 10, 13, None).await?;
    let namespace = Uuid::new_v4();
    let a = LibrarySyncStore::open(db1.clone(), Uuid::new_v4(), Some(namespace)).await?;
    let b = LibrarySyncStore::open(db2.clone(), Uuid::new_v4(), Some(namespace)).await?;
    a.import_existing(a.record_store().author()).await?;
    transfer(&a, &b).await?;
    let row = database::entities::media_file_artists::Entity::find()
        .one(&db2)
        .await?
        .unwrap();
    assert_eq!(row.hlc_uuid, membership.to_string());
    let parent = database::entities::artists::Entity::find()
        .filter(artists::Column::HlcUuid.eq(row.artist_uuid.clone()))
        .one(&db2)
        .await?
        .unwrap();
    assert_eq!(parent.hlc_uuid, artist.to_string());
    assert_eq!(parent.name, "Artist");
    a.import_existing(a.record_store().author()).await?;
    transfer(&a, &b).await?;
    assert_eq!(
        database::entities::media_file_artists::Entity::find()
            .all(&db2)
            .await?
            .len(),
        1
    );
    Ok(())
}
