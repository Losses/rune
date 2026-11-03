#[path = "support/causal_library.rs"]
mod support;

use anyhow::Result;
use database::{
    actions::stats::{increase_played_through, set_liked},
    entities::{
        media_file_likes, media_file_playlists, media_file_stats, media_files, mixes, playlists,
    },
    sync::{
        LibrarySyncStore,
        model::{Collection, Mix, Payload, PlaylistEntry},
    },
};
use fsio::FsIo;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, ExprTrait, PaginatorTrait,
    QueryFilter,
    sea_query::{Alias, Expr, Query},
};
use std::{fs, sync::Arc};
use support::{Peer, exchange, fixtures, key, playlist};
use uuid::Uuid;

struct LegacyFixture {
    _root: tempfile::TempDir,
    db: DatabaseConnection,
    actor: Uuid,
    local: LibrarySyncStore,
    own_tracks: Vec<media_files::Model>,
    own_playlist: Uuid,
    own_mix: Uuid,
    remote_track: Uuid,
    remote_playlist: Uuid,
    remote_mix: Uuid,
}

async fn legacy_fixture() -> Result<LegacyFixture> {
    let root = tempfile::tempdir()?;
    let actor = Uuid::new_v4();
    let other = Uuid::new_v4();
    let legacy = Uuid::from_u128(0x59336769_da35_55f3_bf01_8180c38025d0);
    let db = fixtures::disk_database(&root.path().join("local.sqlite"), actor).await?;
    let peer_db = fixtures::disk_database(&root.path().join("peer.sqlite"), other).await?;
    let local = LibrarySyncStore::open(db.clone(), actor, Some(legacy)).await?;
    let remote = LibrarySyncStore::open(peer_db.clone(), other, Some(legacy)).await?;
    local.import_existing(actor).await?;
    remote.import_existing(other).await?;
    let audio_root = root.path().join("audio");
    fs::create_dir(&audio_root)?;
    for index in 0..2 {
        fs::copy(
            fixtures::audio_fixture_at(index),
            audio_root.join(format!("{index}.ogg")),
        )?;
    }
    fixtures::scan_library(
        &FsIo::new(),
        &db,
        &actor.to_string(),
        &audio_root,
        &local.record_store(),
    )
    .await?;
    let own_tracks = media_files::Entity::find().all(&db).await?;
    assert_eq!(own_tracks.len(), 2);
    let own_playlist = Uuid::new_v4();
    let own_mix = Uuid::new_v4();
    let remote_track = Uuid::new_v4();
    let remote_playlist = Uuid::new_v4();
    let remote_mix = Uuid::new_v4();
    for (store, list, mix, name) in [
        (&local, own_playlist, own_mix, "own"),
        (&remote, remote_playlist, remote_mix, "peer"),
    ] {
        store
            .write_local(key(Collection::Playlists, list), playlist(name))
            .await?;
        store
            .write_local(
                key(Collection::Mixes, mix),
                Some(Payload::Mix(Mix {
                    name: name.into(),
                    group: "test".into(),
                    mode: None,
                    scriptlet_mode: false,
                    locked: false,
                    queries: Vec::new(),
                })),
            )
            .await?;
        store
            .write_local(
                key(Collection::PlaylistEntries, Uuid::new_v4()),
                Some(Payload::PlaylistEntry(PlaylistEntry {
                    playlist_id: list,
                    track_id: own_tracks[0].hlc_uuid.parse()?,
                    position: 0,
                })),
            )
            .await?;
    }
    set_liked(&db, &local.record_store(), own_tracks[0].id, true).await?;
    increase_played_through(&db, &local.record_store(), own_tracks[0].id, actor).await?;
    remote
        .write_local(
            key(Collection::Tracks, remote_track),
            Some(support::track()),
        )
        .await?;
    let source: Peer = Arc::new(local.endpoint());
    let target: Peer = Arc::new(remote.endpoint());
    exchange(&source, &target).await?;
    let peer_tracks = media_files::Entity::find().all(&peer_db).await?;
    let peer_liked = peer_tracks
        .iter()
        .find(|row| row.hlc_uuid == own_tracks[1].hlc_uuid)
        .unwrap();
    set_liked(&peer_db, &remote.record_store(), peer_liked.id, true).await?;
    let peer_stats = peer_tracks
        .iter()
        .find(|row| row.hlc_uuid == own_tracks[0].hlc_uuid)
        .unwrap();
    increase_played_through(&peer_db, &remote.record_store(), peer_stats.id, other).await?;
    exchange(&target, &source).await?;
    assert_eq!(media_files::Entity::find().count(&db).await?, 3);
    assert_eq!(media_file_likes::Entity::find().count(&db).await?, 2);
    assert_eq!(media_file_stats::Entity::find().count(&db).await?, 2);
    Ok(LegacyFixture {
        _root: root,
        db,
        actor,
        local,
        own_tracks,
        own_playlist,
        own_mix,
        remote_track,
        remote_playlist,
        remote_mix,
    })
}

#[tokio::test]
async fn legacy_reset_preserves_only_local_sources_and_reopens_idempotently() -> Result<()> {
    let fixture = legacy_fixture().await?;
    let real = Uuid::new_v4();
    let old_identity = fixture.local.identity().await?;
    let store = LibrarySyncStore::open(fixture.db.clone(), fixture.actor, Some(real)).await?;
    assert_eq!(store.identity().await?.namespace, real);
    assert_ne!(store.identity().await?.replica, old_identity.replica);
    for table in [
        "sync_meta",
        "sync_candidates",
        "sync_journal",
        "sync_checkpoints",
        "sync_observed",
        "sync_manifests",
        "sync_chunks",
        "sync_manifest_edges",
        "sync_leases",
    ] {
        let query = Query::select()
            .expr(Expr::col(Alias::new("namespace")).count())
            .from(Alias::new(table))
            .and_where(
                Expr::col(Alias::new("namespace")).eq(old_identity.namespace.as_bytes().to_vec()),
            )
            .to_owned();
        let remaining: i64 = fixture
            .db
            .query_one(&query)
            .await?
            .unwrap()
            .try_get_by_index(0)?;
        assert_eq!(remaining, 0, "legacy rows remain in {table}");
    }
    let entries = media_file_playlists::Entity::find()
        .all(&fixture.db)
        .await?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].playlist_uuid, fixture.own_playlist.to_string());
    for original in &fixture.own_tracks {
        let row = media_files::Entity::find_by_id(original.id)
            .one(&fixture.db)
            .await?
            .unwrap();
        assert_eq!(row.hlc_uuid, original.hlc_uuid);
        assert!(row.local_available);
        assert!(
            store
                .get(&key(Collection::Tracks, row.hlc_uuid.parse()?))
                .await?
                .is_some()
        );
    }
    assert!(
        media_files::Entity::find()
            .filter(media_files::Column::HlcUuid.eq(fixture.remote_track.to_string()))
            .one(&fixture.db)
            .await?
            .is_none()
    );
    assert_eq!(
        media_file_likes::Entity::find().count(&fixture.db).await?,
        1
    );
    assert_eq!(
        media_file_likes::Entity::find()
            .one(&fixture.db)
            .await?
            .unwrap()
            .track_uuid,
        fixture.own_tracks[0].hlc_uuid
    );
    let stats = media_file_stats::Entity::find().all(&fixture.db).await?;
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].device_id, fixture.actor.to_string());
    assert_eq!(stats[0].played_through, 1);
    let own_track = fixture.own_tracks[0].hlc_uuid.parse::<Uuid>()?;
    assert_eq!(
        stats[0].hlc_uuid,
        Uuid::new_v5(&own_track, fixture.actor.as_bytes()).to_string()
    );
    assert!(
        store
            .get(&key(Collection::Likes, own_track))
            .await?
            .is_some()
    );
    assert!(
        store
            .get(&key(
                Collection::PlaybackStatistics,
                stats[0].hlc_uuid.parse()?
            ))
            .await?
            .is_some()
    );
    for (collection, own, remote) in [
        (
            Collection::Playlists,
            fixture.own_playlist,
            fixture.remote_playlist,
        ),
        (Collection::Mixes, fixture.own_mix, fixture.remote_mix),
    ] {
        assert!(store.get(&key(collection, own)).await?.is_some());
        assert!(store.get(&key(collection, remote)).await?.is_none());
    }
    assert!(
        playlists::Entity::find()
            .filter(playlists::Column::HlcUuid.eq(fixture.remote_playlist.to_string()))
            .one(&fixture.db)
            .await?
            .is_none()
    );
    assert!(
        mixes::Entity::find()
            .filter(mixes::Column::HlcUuid.eq(fixture.remote_mix.to_string()))
            .one(&fixture.db)
            .await?
            .is_none()
    );
    let head = store.head().await?;
    let identity = store.identity().await?;
    let reopened = LibrarySyncStore::open(fixture.db.clone(), fixture.actor, Some(real)).await?;
    reopened.import_existing(fixture.actor).await?;
    assert_eq!(reopened.head().await?, head);
    assert_eq!(reopened.identity().await?, identity);
    assert!(
        LibrarySyncStore::open(fixture.db, fixture.actor, Some(Uuid::new_v4()))
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn legacy_reset_drops_own_references_to_peer_only_tracks() -> Result<()> {
    let fixture = legacy_fixture().await?;
    let remote_row = media_files::Entity::find()
        .filter(media_files::Column::HlcUuid.eq(fixture.remote_track.to_string()))
        .one(&fixture.db)
        .await?
        .unwrap();
    fixture
        .local
        .write_local(
            key(Collection::PlaylistEntries, Uuid::new_v4()),
            Some(Payload::PlaylistEntry(PlaylistEntry {
                playlist_id: fixture.own_playlist,
                track_id: fixture.remote_track,
                position: 1,
            })),
        )
        .await?;
    set_liked(
        &fixture.db,
        &fixture.local.record_store(),
        remote_row.id,
        true,
    )
    .await?;
    let store =
        LibrarySyncStore::open(fixture.db.clone(), fixture.actor, Some(Uuid::new_v4())).await?;
    store.import_existing(fixture.actor).await?;
    let remote = fixture.remote_track.to_string();
    let entries = media_file_playlists::Entity::find()
        .all(&fixture.db)
        .await?;
    assert!(entries.iter().all(|entry| entry.track_uuid != remote));
    assert!(
        media_file_likes::Entity::find()
            .all(&fixture.db)
            .await?
            .iter()
            .all(|like| like.track_uuid != remote)
    );
    Ok(())
}
