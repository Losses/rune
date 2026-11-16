use anyhow::Result;
use database::test_fixtures::causal_library as support;
use database::{
    actions::{metadata::scan_audio_file, stats::set_liked},
    entities::media_files,
    sync::model::{Collection, Liked, ObjectKey, Payload, RuneObjectKey, Scope},
};
use fsio::FsIo;
use migration::{Migrator, MigratorTrait};
use sea_orm::{Database, EntityTrait};
use std::sync::Arc;
use support::fixtures;
use sync::protocol::SyncError;
use uuid::Uuid;

#[tokio::test]
async fn new_node_opening_portable_library_can_publish_existing_state() -> Result<()> {
    let root = tempfile::tempdir()?;
    let database_path = root.path().join("library.db");
    let url = format!("sqlite://{}", database_path.display());
    let fixture = fixtures::audio_fixture();
    let audio = root.path().join("source.ogg");
    std::fs::copy(fixture, &audio)?;
    let node_a = Uuid::new_v4();
    let db = fixtures::disk_database(&database_path, node_a).await?;
    let node_b = Uuid::new_v4();
    let library = Uuid::new_v4();
    let fsio = Arc::new(FsIo::new());
    let node_a_store = fixtures::open_library(db.clone(), node_a, Some(library)).await?;
    let file = scan_audio_file(
        &fsio,
        &db,
        &node_a.to_string(),
        root.path(),
        &audio,
        &node_a_store.record_store(),
    )
    .await?;
    let track_id = Uuid::parse_str(
        &media_files::Entity::find_by_id(file)
            .one(&db)
            .await?
            .unwrap()
            .source_uuid,
    )?;
    let track_key = ObjectKey {
        collection: Collection::Tracks,
        id: track_id,
    };
    let like_key = ObjectKey::liked(track_id);
    node_a_store.import_existing(node_a).await?;
    set_liked(&db, &node_a_store.record_store(), file, true).await?;
    let head_a = node_a_store.head().await?;
    assert!(node_a_store.get(&track_key).await?.is_some());
    assert!(matches!(
        node_a_store.get(&like_key).await?.and_then(|r| r.value),
        Some(Payload::Liked(Liked { liked: true, .. }))
    ));
    drop(node_a_store);
    db.close().await?;

    let db = Database::connect(&url).await?;
    let node_b_store = Arc::new(fixtures::open_library(db, node_b, Some(library)).await?);
    let head_b = node_b_store.head().await?;
    let b_track = node_b_store.get(&track_key).await?;
    let b_like = node_b_store.get(&like_key).await?;
    eprintln!(
        "node_a_head={head_a:?} node_b_head={head_b:?} node_b_track={} node_b_like={}",
        b_track.is_some(),
        b_like.is_some()
    );
    let receiver = support::memory(library, Uuid::new_v4())?;
    let scope = Scope {
        collections: [Collection::Tracks, Collection::Likes]
            .into_iter()
            .collect(),
    };
    support::exchange_scoped(
        &(Arc::new(node_b_store.endpoint()) as support::Peer),
        &receiver,
        scope,
    )
    .await?;
    assert_eq!(
        head_a, head_b,
        "opening the same library does not reauthor source rows"
    );
    assert!(
        b_track.is_some(),
        "new node must publish the existing track"
    );
    assert!(
        matches!(
            b_like.and_then(|r| r.value),
            Some(Payload::Liked(Liked { liked: true, .. }))
        ),
        "new node must publish the existing like"
    );
    assert!(receiver.get(&track_key).await?.is_some());
    assert!(receiver.get(&like_key).await?.is_some());
    Ok(())
}

#[tokio::test]
async fn copied_library_with_distinct_actors_is_rejected_as_same_replica() -> Result<()> {
    use database::{
        actions::{
            playlists::create_playlist,
            stats::{get_liked, increase_skipped},
        },
        entities::{media_file_stats, media_files, playlists},
    };
    use sea_orm::{ConnectionTrait, PaginatorTrait, QueryOrder};
    let root = tempfile::tempdir()?;
    let a_path = root.path().join("a.db");
    let b_path = root.path().join("b.db");
    let a_url = format!("sqlite://{}?mode=rwc", a_path.display());
    let b_url = format!("sqlite://{}?mode=rwc", b_path.display());
    let actor_a = Uuid::new_v4();
    let actor_b = Uuid::new_v4();
    let namespace = Uuid::new_v4();
    let _ = migration::initialize_node_id(actor_a.to_string());
    let db = Database::connect(&a_url).await?;
    Migrator::up(&db, None).await?;
    let initial = fixtures::open_library(db.clone(), actor_a, Some(namespace)).await?;
    let fsio = FsIo::new();
    let audio = root.path().join("source.ogg");
    std::fs::copy(fixtures::audio_fixture(), &audio)?;
    let file = scan_audio_file(
        &fsio,
        &db,
        &actor_a.to_string(),
        root.path(),
        &audio,
        &initial.record_store(),
    )
    .await?;
    set_liked(&db, &initial.record_store(), file, true).await?;
    increase_skipped(&db, &initial.record_store(), file, actor_a).await?;
    let track_uuid = media_files::Entity::find_by_id(file)
        .one(&db)
        .await?
        .unwrap()
        .source_uuid;
    db.execute_unprepared("PRAGMA wal_checkpoint(TRUNCATE)")
        .await?;
    drop(initial);
    db.close().await?;
    std::fs::copy(&a_path, &b_path)?;
    let a_db = Database::connect(&a_url).await?;
    let b_db = Database::connect(&b_url).await?;
    let a = fixtures::open_library(a_db.clone(), actor_a, Some(namespace)).await?;
    let b = fixtures::open_library(b_db.clone(), actor_b, Some(namespace)).await?;
    let a_identity = a.identity().await?;
    let b_identity = b.identity().await?;
    assert_eq!(a_identity.replica, b_identity.replica);
    assert_eq!(a_identity.generation, b_identity.generation);
    increase_skipped(&a_db, &a.record_store(), file, actor_a).await?;
    increase_skipped(&b_db, &b.record_store(), file, actor_b).await?;
    increase_skipped(&b_db, &b.record_store(), file, actor_b).await?;
    set_liked(&a_db, &a.record_store(), file, false).await?;
    let a_list =
        create_playlist(&a_db, &a.record_store(), "A playlist".into(), "test".into()).await?;
    let b_list =
        create_playlist(&b_db, &b.record_store(), "B playlist".into(), "test".into()).await?;
    let c_path = root.path().join("c.db");
    let c_db = Database::connect(format!("sqlite://{}?mode=rwc", c_path.display())).await?;
    Migrator::up(&c_db, None).await?;
    let c = fixtures::open_library(c_db.clone(), Uuid::new_v4(), Some(namespace)).await?;
    let a_peer: support::Peer = Arc::new(a.endpoint());
    let b_peer: support::Peer = Arc::new(b.endpoint());
    let scope = Scope {
        collections: [
            Collection::Tracks,
            Collection::Likes,
            Collection::Playlists,
            Collection::PlaybackStatistics,
        ]
        .into_iter()
        .collect(),
    };
    // Independent writers do not make a copied replica safe to synchronize.
    // The Coordinator rejects the equal replica/generation before reading history.
    let head_a = a_peer.head().await?;
    let head_b = b_peer.head().await?;
    let third_identity = c.identity().await?;
    assert_ne!(third_identity.replica, a_identity.replica);
    let error = support::exchange_scoped(&a_peer, &b_peer, scope.clone())
        .await
        .unwrap_err();
    assert!(
        matches!(
            error.downcast_ref::<sync::protocol::SyncError>(),
            Some(SyncError::IncompatibleModel)
        ),
        "unexpected rejection: {error:#}"
    );
    assert_eq!(a_peer.head().await?, head_a);
    assert_eq!(b_peer.head().await?, head_b);
    let track_id: Uuid = track_uuid.parse()?;
    let own_a = ObjectKey {
        collection: Collection::PlaybackStatistics,
        id: Uuid::new_v5(&track_id, actor_a.as_bytes()),
    };
    let own_b = ObjectKey {
        collection: Collection::PlaybackStatistics,
        id: Uuid::new_v5(&track_id, actor_b.as_bytes()),
    };
    assert!(a_peer.get(&own_a).await?.is_some());
    assert!(a_peer.get(&own_b).await?.is_none());
    assert!(b_peer.get(&own_b).await?.is_some());
    assert!(
        b_peer
            .get(&ObjectKey {
                collection: Collection::Playlists,
                id: a_list.source_uuid.parse()?
            })
            .await?
            .is_none()
    );
    assert!(
        a_peer
            .get(&ObjectKey {
                collection: Collection::Playlists,
                id: b_list.source_uuid.parse()?
            })
            .await?
            .is_none()
    );
    for (db, expected_a, expected_b, liked) in [(&a_db, 2, None, false), (&b_db, 1, Some(2), true)]
    {
        let file = media_files::Entity::find().one(db).await?.unwrap();
        assert_eq!(get_liked(db, file.id).await?, liked);
        let stats = media_file_stats::Entity::find()
            .order_by_asc(media_file_stats::Column::DeviceId)
            .all(db)
            .await?;
        assert_eq!(
            stats
                .iter()
                .find(|s| s.device_id == actor_a.to_string())
                .unwrap()
                .skipped,
            expected_a
        );
        assert_eq!(
            stats
                .iter()
                .find(|s| s.device_id == actor_b.to_string())
                .map(|s| s.skipped),
            expected_b
        );
        assert_eq!(playlists::Entity::find().count(db).await?, 1);
    }
    Ok(())
}
