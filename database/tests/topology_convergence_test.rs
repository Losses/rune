//! Eight replicas converge using only the scheduled graph before arbitrary-pair checks.
use migration::catalog_identity::ParentKind;
use std::time::Instant;
#[path = "support/causal_library.rs"]
mod support;
use anyhow::{Context, Result, ensure};
use database::actions::mixes::executable_queries_on;
use database::{entities::*, sync::model::*};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};
use std::collections::{BTreeMap, BTreeSet};
use support::{Peer, SqlLibrary, exchange, key, playlist};
use sync::{Candidate, ReceiveProgress};
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn edges(shape: usize) -> Vec<(usize, usize)> {
    match shape {
        0 => (0..7).map(|i| (i, i + 1)).collect(),
        1 => (0..8).map(|i| (i, (i + 1) % 8)).collect(),
        2 => (1..8).map(|i| (0, i)).collect(),
        _ => vec![
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 0),
            (3, 4),
            (4, 5),
            (5, 6),
            (6, 4),
            (6, 7),
        ],
    }
}
type Expected = BTreeMap<ObjectKey, Vec<Candidate<RuneModel>>>;
async fn candidates(peer: &Peer, key: &ObjectKey) -> Result<Vec<Candidate<RuneModel>>> {
    let page = peer
        .candidates(
            key,
            None,
            sync::PageLimits {
                max_scanned: 16,
                max_bytes: 65536,
            },
        )
        .await?;
    ensure!(page.done, "candidate assertion exceeded its bound");
    let mut values = page.values;
    values.sort_by_key(|value| value.dot);
    Ok(values)
}
async fn author(
    peer: &Peer,
    expected: &mut Expected,
    key: ObjectKey,
    value: Option<Payload>,
) -> Result<()> {
    peer.write_local(key.clone(), value).await?;
    expected.insert(key.clone(), candidates(peer, &key).await?);
    Ok(())
}
async fn heads(peers: &[Peer]) -> Result<Vec<ReceiveProgress>> {
    let mut result = Vec::new();
    for peer in peers {
        result.push(peer.head().await?);
    }
    Ok(result)
}
async fn matches(peers: &[Peer], expected: &Expected) -> Result<bool> {
    for peer in peers {
        for (key, values) in expected {
            if &candidates(peer, key).await? != values {
                return Ok(false);
            }
            let winner = values
                .iter()
                .max_by_key(|value| value.revision)
                .map(|value| &value.payload);
            if peer.get(key).await?.and_then(|row| row.value).as_ref() != winner {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
async fn converge(peers: &[Peer], links: &[(usize, usize)], expected: &Expected) -> Result<()> {
    for round in 0..40 {
        let before = heads(peers).await?;
        for &(a, b) in links {
            exchange(&peers[a], &peers[b]).await?;
            exchange(&peers[b], &peers[a]).await?;
        }
        let agrees = matches(peers, expected).await?;
        let after = heads(peers).await?;
        if std::env::var_os("RUNE_SYNC_TRACE").is_some() {
            eprintln!(
                "convergence round={round} candidates_agree={agrees} heads={:?}",
                after.iter().map(|v| v.sequence).collect::<Vec<_>>()
            );
        }
        if agrees && after == before {
            return Ok(());
        }
        ensure!(
            round < 39,
            "graph did not converge to its independent candidate oracle"
        );
    }
    unreachable!()
}
async fn scenario(peers: &[Peer], shape: usize, seed: u64) -> Result<()> {
    let mut expected = Expected::new();
    let conflict = key(Collection::Playlists, id(1));
    author(
        &peers[0],
        &mut expected,
        conflict.clone(),
        playlist("common"),
    )
    .await?;
    // The eighth device remains disconnected throughout the partitioned writes.
    for i in 0..6 {
        exchange(&peers[i], &peers[i + 1]).await?;
    }
    let track = id(88);
    let album = migration::catalog_identity::parent(ParentKind::Album, "Parent album");
    author(
        &peers[0],
        &mut expected,
        key(Collection::TrackAlbums, track),
        Some(Payload::TrackAlbum(TrackAlbum {
            track_id: track,
            album_id: album,
            track_number: Some(4),
        })),
    )
    .await?;
    // The destination must accept a relationship even though neither target exists.
    exchange(&peers[0], &peers[1]).await?;
    assert!(
        peers[1]
            .get(&key(Collection::TrackAlbums, track))
            .await?
            .is_some()
    );
    assert!(
        peers[1]
            .get(&key(Collection::Tracks, track))
            .await?
            .is_none()
    );

    let mut concurrent = Vec::new();
    let mut likes = Vec::new();
    for (index, peer) in peers.iter().take(7).enumerate() {
        peer.write_local(conflict.clone(), playlist(&format!("initial-{index}")))
            .await?;
        peer.write_local(
            conflict.clone(),
            if index == 0 {
                None
            } else {
                playlist(&format!("edit-{index}"))
            },
        )
        .await?;
        concurrent.extend(candidates(peer, &conflict).await?);
        author(
            peer,
            &mut expected,
            key(Collection::Playlists, id(10 + index as u128)),
            playlist(&format!("unique-{index}")),
        )
        .await?;
        let like = key(Collection::Likes, track);
        peer.write_local(
            like.clone(),
            Some(Payload::Liked(Liked {
                track_id: track,
                liked: index % 2 == 0,
                updated_at: "2026-11-01T00:00:00Z".into(),
            })),
        )
        .await?;
        likes.extend(candidates(peer, &like).await?);
        let device = peer.local_author().await?.author;
        author(
            peer,
            &mut expected,
            key(
                Collection::PlaybackStatistics,
                Uuid::new_v5(&track, device.as_bytes()),
            ),
            Some(Payload::PlaybackStatistics(PlaybackStatistics {
                track_id: track,
                device_id: device,
                skipped: index as i32,
                played_through: 2,
                updated_at: "2026-11-01T00:00:00Z".into(),
            })),
        )
        .await?;
    }
    concurrent.sort_by_key(|value| value.dot);
    likes.sort_by_key(|value| value.dot);
    assert_eq!(
        concurrent.len(),
        6,
        "removal must preserve unseen concurrent assignments"
    );
    expected.insert(conflict.clone(), concurrent);
    expected.insert(key(Collection::Likes, track), likes);
    author(
        &peers[2],
        &mut expected,
        key(Collection::Tracks, track),
        Some(support::track()),
    )
    .await?;
    author(
        &peers[3],
        &mut expected,
        key(Collection::Albums, album),
        Some(Payload::Album(Album {
            name: "Parent album".into(),
            group: "test".into(),
        })),
    )
    .await?;
    for (occurrence, position) in [(id(90), 0), (id(91), 1)] {
        author(
            &peers[4],
            &mut expected,
            key(Collection::PlaylistEntries, occurrence),
            Some(Payload::PlaylistEntry(PlaylistEntry {
                playlist_id: id(1),
                track_id: track,
                position,
            })),
        )
        .await?;
    }
    author(
        &peers[5],
        &mut expected,
        key(Collection::Mixes, id(95)),
        Some(Payload::Mix(Mix {
            name: "Reference mix".into(),
            group: "test".into(),
            mode: Some(0),
            scriptlet_mode: false,
            locked: false,
            queries: vec![MixClause {
                id: id(96),
                position: 0,
                operator: "lib::track".into(),
                parameter: String::new(),
                target_uuid: Some(track),
                group: 0,
            }],
        })),
    )
    .await?;
    let mut random = seed;
    let links = edges(shape);
    let mut contacted = BTreeSet::new();
    for step in 0..24 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let active = edges((shape + step / 8) % 4);
        let (a, b) = active[random as usize % active.len()];
        if a != 7 && b != 7 {
            contacted.insert((a.min(b), a.max(b)));
            let (a, b) = if random & 1 == 0 { (a, b) } else { (b, a) };
            exchange(&peers[a], &peers[b]).await?;
        }
    }
    converge(peers, &links, &expected).await?;
    // An observed removal now removes all six candidates; occurrences remain independent.
    author(&peers[0], &mut expected, conflict.clone(), None).await?;
    converge(peers, &links, &expected).await?;
    for peer in peers {
        assert!(peer.get(&conflict).await?.is_none());
        assert!(
            peer.get(&key(Collection::PlaylistEntries, id(90)))
                .await?
                .is_some()
        );
    }
    author(&peers[0], &mut expected, conflict, playlist("restored")).await?;
    converge(peers, &links, &expected).await?;
    let stable = heads(peers).await?;
    for _ in 0..32 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let (a, b) = links[random as usize % links.len()];
        exchange(&peers[a], &peers[b]).await?;
    }
    assert_eq!(
        heads(peers).await?,
        stable,
        "settled graph republished history"
    );
    // First contact between any pair must also leave accepted state and history unchanged.
    for a in 0..8 {
        for b in a + 1..8 {
            exchange(&peers[a], &peers[b]).await?;
            exchange(&peers[b], &peers[a]).await?;
        }
    }
    assert!(
        contacted.len() < 28,
        "no previously disconnected pair was tested"
    );
    assert_eq!(
        heads(peers).await?,
        stable,
        "arbitrary-pair exchange republished history"
    );
    assert!(matches(peers, &expected).await?);
    Ok(())
}
#[tokio::test]
async fn memory_peer_topologies_settle() -> Result<()> {
    for shape in 0..4 {
        for seed in 1..=12 {
            let peers = (0..8)
                .map(|i| support::memory(id(100), id(1000 + i)))
                .collect::<Result<Vec<_>>>()?;
            scenario(&peers, shape, seed)
                .await
                .with_context(|| format!("memory shape {shape}, seed {seed}"))?;
        }
    }
    Ok(())
}
async fn sqlite_scenario(shape: usize, seed: u64) -> Result<()> {
    let started = Instant::now();
    eprintln!("SQLite topology shape={shape} seed={seed} replicas=8 start");
    let mut libraries = Vec::new();
    for index in 0..8 {
        let local = SqlLibrary::new(id(100), id(1000 + index)).await?;
        // Different local primary keys must not enter portable relationships.
        for offset in 0..index {
            let mut value = support::track();
            if let Payload::Track(track) = &mut value {
                track.metadata.clear();
            }
            RuneModel::project(
                &local.db,
                &key(Collection::Tracks, id(10000 + offset)),
                Some(&value),
            )
            .await?;
        }
        libraries.push(local);
    }
    let peers: Vec<_> = libraries.iter().map(|local| local.peer.clone()).collect();
    scenario(&peers, shape, seed)
        .await
        .with_context(|| format!("SQLite shape {shape}, seed {seed}"))?;
    let mut local_ids = BTreeSet::new();
    for local in &libraries {
        let track = media_files::Entity::find()
            .filter(media_files::Column::HlcUuid.eq(id(88).to_string()))
            .one(&local.db)
            .await?
            .context("track missing")?;
        local_ids.insert(track.id);
        assert!(!track.local_available);
        assert_eq!(media_file_stats::Entity::find().count(&local.db).await?, 7);
        assert_eq!(
            media_file_stats::Entity::find()
                .all(&local.db)
                .await?
                .iter()
                .map(|row| row.skipped)
                .sum::<i32>(),
            21
        );
        assert_eq!(media_file_likes::Entity::find().count(&local.db).await?, 1);
        let entries = media_file_playlists::Entity::find()
            .order_by_asc(media_file_playlists::Column::Position)
            .all(&local.db)
            .await?;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].track_uuid, entries[1].track_uuid);
        assert_ne!(entries[0].hlc_uuid, entries[1].hlc_uuid);
        let album = media_file_albums::Entity::find()
            .one(&local.db)
            .await?
            .context("album link missing")?;
        assert_eq!(album.track_uuid, id(88).to_string());
        assert_eq!(album.track_number, Some(4));
        assert!(local.library.pending_references().await?.is_empty());
        let queries = executable_queries_on(&local.db, &id(95).to_string()).await?;
        assert_eq!(queries, vec![("lib::track".into(), track.id.to_string())]);
        assert_eq!(
            playlists::Entity::find()
                .filter(playlists::Column::Name.eq("restored"))
                .count(&local.db)
                .await?,
            1
        );
    }
    assert_eq!(
        local_ids.len(),
        8,
        "primary-key offset fixture was ineffective"
    );
    eprintln!(
        "SQLite topology shape={shape} seed={seed} elapsed_ms={} complete",
        started.elapsed().as_millis()
    );
    Ok(())
}

async fn sqlite_case(case: usize) -> Result<()> {
    sqlite_scenario(case / 4, (case % 4 + 1) as u64).await
}

seq_macro::seq!(case in 0..16 {
    #[tokio::test]
    async fn sqlite_graph_~case() -> Result<()> {
        sqlite_case(case).await
    }
});
