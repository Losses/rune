#[path = "support/causal_library.rs"]
mod support;
use anyhow::Result;
use database::{
    actions::playlists,
    sync::{
        LibrarySyncStore,
        model::{Collection, Scope},
    },
};
use migration::{Migrator, MigratorTrait};
use sea_orm::{Database, EntityTrait, PaginatorTrait, QueryFilter, Set};
use std::sync::Arc;
use uuid::Uuid;
#[tokio::test]
async fn commands_publish_playlist_and_occurrence_removals() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    let peer_db = Database::connect("sqlite::memory:").await?;
    let node = Uuid::new_v4();
    let ns = Uuid::new_v4();
    let _ = migration::initialize_node_id(node.to_string());
    Migrator::up(&db, None).await?;
    Migrator::up(&peer_db, None).await?;
    let a = LibrarySyncStore::open(db.clone(), node, Some(ns)).await?;
    let b = LibrarySyncStore::open(peer_db.clone(), Uuid::new_v4(), Some(ns)).await?;
    let file = Uuid::new_v4();
    for conn in [&db, &peer_db] {
        database::entities::media_files::Entity::insert(
            database::entities::media_files::ActiveModel {
                file_name: Set("song".into()),
                directory: Set(String::new()),
                extension: Set("ogg".into()),
                file_hash: Set(String::new()),
                last_modified: Set(String::new()),
                sample_rate: Set(1),
                duration: Set(1.into()),
                hlc_uuid: Set(file.to_string()),
                ..Default::default()
            },
        )
        .exec(conn)
        .await?;
    }
    a.import_basic().await?;
    let record_store = a.record_store();
    let list = playlists::create_playlist(&db, &record_store, "test".into(), "t".into()).await?;
    playlists::update_playlist(&db, &record_store, list.id, Some("renamed".into()), None).await?;
    let first = playlists::add_item_to_playlist(&db, &record_store, list.id, 1, None).await?;
    let second = playlists::add_item_to_playlist(&db, &record_store, list.id, 1, None).await?;
    assert_ne!(first.hlc_uuid, second.hlc_uuid);
    playlists::reorder_playlist_item_position(
        &db,
        &record_store,
        list.id,
        second.hlc_uuid.parse()?,
        0,
    )
    .await?;
    let scope = Scope {
        collections: [
            Collection::Tracks,
            Collection::Playlists,
            Collection::PlaylistEntries,
        ]
        .into_iter()
        .collect(),
    };
    support::exchange_scoped(
        &(Arc::new(a.endpoint()) as support::Peer),
        &(Arc::new(b.endpoint()) as support::Peer),
        scope.clone(),
    )
    .await?;
    assert_eq!(b.pending_references().await?.len(), 0);
    let remote_playlist = database::entities::playlists::Entity::find()
        .one(&peer_db)
        .await?
        .unwrap();
    assert_eq!(remote_playlist.name, "renamed");
    let remote_entries = database::entities::media_file_playlists::Entity::find()
        .all(&peer_db)
        .await?;
    assert_eq!(remote_entries.len(), 2);
    assert!(
        remote_entries
            .iter()
            .any(|entry| entry.hlc_uuid == second.hlc_uuid && entry.position == 0)
    );
    playlists::remove_item_from_playlist(&db, &record_store, list.id, first.hlc_uuid.parse()?)
        .await?;
    support::exchange_scoped(
        &(Arc::new(a.endpoint()) as support::Peer),
        &(Arc::new(b.endpoint()) as support::Peer),
        scope.clone(),
    )
    .await?;
    assert_eq!(
        database::entities::media_file_playlists::Entity::find()
            .count(&peer_db)
            .await?,
        1
    );
    let mix = database::actions::mixes::create_mix(
        &db,
        &record_store,
        "dependent".into(),
        "d".into(),
        false,
        0,
        false,
        vec![("lib::playlist".into(), list.id.to_string())],
    )
    .await?;
    playlists::remove_playlist(&db, &record_store, list.id).await?;
    support::exchange_scoped(
        &(Arc::new(a.endpoint()) as support::Peer),
        &(Arc::new(b.endpoint()) as support::Peer),
        scope,
    )
    .await?;
    assert_eq!(
        database::entities::playlists::Entity::find()
            .count(&peer_db)
            .await?,
        0
    );
    assert_eq!(
        database::entities::media_file_playlists::Entity::find()
            .count(&peer_db)
            .await?,
        1
    );
    use sea_orm::ColumnTrait;
    assert!(
        database::entities::mix_queries::Entity::find()
            .filter(database::entities::mix_queries::Column::MixUuid.eq(mix.hlc_uuid.clone()))
            .all(&db)
            .await?
            .len()
            == 1
    );
    assert!(
        a.pending_references()
            .await?
            .iter()
            .any(|(key, _)| key.collection == Collection::Mixes
                && key.id.to_string() == mix.hlc_uuid)
    );
    assert!(a.head().await?.sequence > 0);
    Ok(())
}

#[tokio::test]
async fn occurrence_cursor_preserves_duplicates_and_commands_address_one_uuid() -> Result<()> {
    use database::{
        actions::playlists::list_occurrences,
        entities::{media_file_playlists as entries, media_files},
    };
    use sea_orm::{ColumnTrait, QueryOrder, Set, sea_query::Expr};
    let db = Database::connect("sqlite::memory:").await?;
    let actor = Uuid::new_v4();
    let _ = migration::initialize_node_id(actor.to_string());
    Migrator::up(&db, None).await?;
    let root = tempfile::tempdir()?;
    let store = LibrarySyncStore::open(db.clone(), actor, Some(Uuid::new_v4())).await?;
    let file = media_files::Entity::insert(media_files::ActiveModel {
        file_name: Set("remote.flac".into()),
        directory: Set(String::new()),
        extension: Set("flac".into()),
        file_hash: Set(String::new()),
        last_modified: Set(String::new()),
        sample_rate: Set(44100),
        duration: Set(1.into()),
        local_available: Set(false),
        hlc_uuid: Set(Uuid::new_v4().to_string()),
        ..Default::default()
    })
    .exec_with_returning(&db)
    .await?;
    let record_store = store.record_store();
    let playlist =
        playlists::create_playlist(&db, &record_store, "Occurrences".into(), "O".into()).await?;
    let other = playlists::create_playlist(&db, &record_store, "Other".into(), "O".into()).await?;
    let mut ids = Vec::new();
    for _ in 0..5 {
        ids.push(
            playlists::add_item_to_playlist(&db, &record_store, playlist.id, file.id, Some(7))
                .await?
                .hlc_uuid,
        );
    }
    ids.sort();
    let mut cursor = None;
    let mut seen = Vec::new();
    loop {
        let page = list_occurrences(&db, playlist.id, cursor, 2).await?;
        assert!(page.entries.iter().all(|entry| {
            entry.position == 7
                && entry
                    .file
                    .as_ref()
                    .is_some_and(|row| row.id == file.id && !row.local_available)
        }));
        seen.extend(page.entries.into_iter().map(|entry| entry.occurrence_id));
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen, ids);
    let selected: Uuid = ids[2].parse()?;
    let unchanged = store.head().await?;
    playlists::reorder_playlist_item_position(&db, &record_store, playlist.id, selected, 7).await?;
    assert_eq!(store.head().await?, unchanged);
    assert!(
        playlists::remove_item_from_playlist(&db, &record_store, other.id, selected)
            .await
            .is_err()
    );
    assert!(
        playlists::reorder_playlist_item_position(&db, &record_store, other.id, selected, 0)
            .await
            .is_err()
    );
    assert_eq!(store.head().await?, unchanged);
    playlists::reorder_playlist_item_position(&db, &record_store, playlist.id, selected, -4)
        .await?;
    assert_eq!(
        list_occurrences(&db, playlist.id, None, 2).await?.entries[0].occurrence_id,
        ids[2]
    );
    playlists::remove_item_from_playlist(&db, &record_store, playlist.id, selected).await?;
    let remaining = entries::Entity::find()
        .filter(entries::Column::PlaylistUuid.eq(playlist.hlc_uuid.clone()))
        .order_by_asc(entries::Column::HlcUuid)
        .all(&db)
        .await?;
    assert_eq!(remaining.len(), 4);
    assert!(
        remaining
            .iter()
            .all(|entry| entry.position == 7 && entry.hlc_uuid != ids[2])
    );

    // Import keeps repeated lines and reports paths that cannot be resolved uniquely.
    media_files::Entity::insert(media_files::ActiveModel {
        file_name: Set("remote.flac".into()),
        directory: Set("other".into()),
        extension: Set("flac".into()),
        file_hash: Set(String::new()),
        last_modified: Set(String::new()),
        sample_rate: Set(44100),
        duration: Set(1.into()),
        hlc_uuid: Set(Uuid::new_v4().to_string()),
        ..Default::default()
    })
    .exec(&db)
    .await?;
    let import_path = root.path().join("list.m3u8");
    tokio::fs::write(
        &import_path,
        "#EXTM3U\nremote.flac\nremote.flac\nmissing.flac\n",
    )
    .await?;
    let (imported, result) = playlists::create_m3u8_playlist(
        &db,
        &record_store,
        "Import".into(),
        "I".into(),
        &import_path,
    )
    .await?;
    assert_eq!(result.matched_ids.len(), 2);
    assert_eq!(result.unmatched_paths, vec!["missing.flac"]);
    assert_eq!(result.ambiguous_paths, vec!["remote.flac", "remote.flac"]);
    let imported_entries = list_occurrences(&db, imported.id, None, 10).await?.entries;
    assert_eq!(imported_entries.len(), 2);
    assert_ne!(
        imported_entries[0].occurrence_id,
        imported_entries[1].occurrence_id
    );

    let peer_db = Database::connect("sqlite::memory:").await?;
    Migrator::up(&peer_db, None).await?;
    let peer = LibrarySyncStore::open(
        peer_db.clone(),
        Uuid::new_v4(),
        Some(record_store.namespace()),
    )
    .await?;
    support::exchange_scoped(
        &(Arc::new(store.endpoint()) as support::Peer),
        &(Arc::new(peer.endpoint()) as support::Peer),
        Scope {
            collections: [Collection::Playlists, Collection::PlaylistEntries]
                .into_iter()
                .collect(),
        },
    )
    .await?;
    let remote_import = database::entities::playlists::Entity::find()
        .filter(database::entities::playlists::Column::HlcUuid.eq(imported.hlc_uuid.clone()))
        .one(&peer_db)
        .await?
        .unwrap();
    let remote_entries = list_occurrences(&peer_db, remote_import.id, None, 10)
        .await?
        .entries;
    assert_eq!(remote_entries.len(), 2);
    assert!(remote_entries.iter().all(|entry| entry.file.is_none()));
    assert!(imported_entries.iter().all(|entry| {
        remote_entries.iter().any(|remote| {
            remote.occurrence_id == entry.occurrence_id && remote.track_id == entry.track_id
        })
    }));

    // Clock exhaustion occurs after the source mutation inside the command's transaction.
    use sea_orm::{
        ConnectionTrait,
        sea_query::{Alias, ExprTrait, Query},
    };
    db.execute(
        &Query::update()
            .table(Alias::new("sync_meta"))
            .value(Alias::new("revision_counter"), i64::MAX)
            .and_where(
                Expr::col(Alias::new("namespace")).eq(store
                    .record_store()
                    .namespace()
                    .as_bytes()
                    .to_vec()),
            )
            .to_owned(),
    )
    .await?;
    let history_before = store.head().await?;
    let count_before = database::entities::playlists::Entity::find()
        .count(&db)
        .await?;
    assert!(
        playlists::create_m3u8_playlist(
            &db,
            &record_store,
            "failed".into(),
            "I".into(),
            &import_path
        )
        .await
        .is_err()
    );
    assert_eq!(
        database::entities::playlists::Entity::find()
            .count(&db)
            .await?,
        count_before
    );
    assert_eq!(store.head().await?, history_before);
    assert!(
        playlists::remove_item_from_playlist(&db, &record_store, playlist.id, ids[0].parse()?)
            .await
            .is_err()
    );
    assert!(
        entries::Entity::find()
            .filter(entries::Column::HlcUuid.eq(ids[0].clone()))
            .one(&db)
            .await?
            .is_some()
    );
    Ok(())
}
