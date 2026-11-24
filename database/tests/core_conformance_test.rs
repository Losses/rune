//! Cross-component convergence and session boundaries.
use anyhow::Result;
use database::sync::model::*;
use database::test_fixtures::causal_library as support;
use support::{Peer, exchange_scoped};
use sync::{self as p};
use uuid::Uuid;
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn key(n: u128) -> ObjectKey {
    support::key(Collection::Playlists, id(n))
}
fn scope(collections: &[Collection]) -> Scope {
    Scope {
        collections: collections.iter().copied().collect(),
    }
}
fn node(n: u128) -> Peer {
    support::memory(id(100), id(n)).unwrap()
}
async fn copy(to: &Peer, from: &Peer, selected: &Scope) -> Result<()> {
    exchange_scoped(from, to, selected.clone()).await?;
    Ok(())
}

#[tokio::test]
async fn four_devices_partition_remove_restore_and_duplicate_schedules() -> Result<()> {
    let selected = scope(&[Collection::Playlists]);
    for seed in 1..=200u64 {
        let peers = [node(1), node(2), node(3), node(4)];
        for peer in &peers {
            peer.write_local(key(1), support::playlist("initial"))
                .await?;
        }
        peers[0].write_local(key(1), None).await?;
        for peer in &peers[1..] {
            peer.write_local(key(1), support::playlist("offline edit"))
                .await?;
        }
        let mut random = seed;
        for _ in 0..40 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let a = ((random >> 32) % 4) as usize;
            let b = (a + 1 + (random % 3) as usize) % 4;
            copy(&peers[b], &peers[a], &selected).await?;
        }
        for a in 0..4 {
            for b in 0..4 {
                if a != b {
                    copy(&peers[b], &peers[a], &selected).await?;
                }
            }
        }
        let expected = peers[0].get(&key(1)).await?;
        for peer in &peers {
            assert_eq!(peer.get(&key(1)).await?, expected, "seed={seed}");
            assert_eq!(
                peer.candidates(
                    &key(1),
                    None,
                    p::PageLimits {
                        max_scanned: 8,
                        max_bytes: 65536
                    }
                )
                .await?
                .values
                .len(),
                3,
                "removal only covers observed assignments; seed={seed}"
            );
        }
        peers[0].write_local(key(1), None).await?;
        for peer in &peers[1..] {
            copy(peer, &peers[0], &selected).await?;
        }
        for peer in &peers {
            assert!(peer.get(&key(1)).await?.is_none());
        }
        let restored = peers[2]
            .write_local(key(1), support::playlist("explicit restore"))
            .await?;
        assert!(restored.revision.counter > 2);
        for a in 0..4 {
            for b in 0..4 {
                if a != b {
                    copy(&peers[b], &peers[a], &selected).await?;
                }
            }
        }
        let mut heads = Vec::new();
        for peer in &peers {
            assert_eq!(peer.get(&key(1)).await?, Some(restored.clone()));
            heads.push(peer.head().await?);
        }
        for a in 0..4 {
            for b in 0..4 {
                if a != b {
                    copy(&peers[b], &peers[a], &selected).await?;
                }
            }
        }
        for (peer, head) in peers.iter().zip(heads) {
            assert_eq!(peer.head().await?, head);
        }
    }
    Ok(())
}

#[tokio::test]
async fn new_peer_late_old_version_and_scope_changes_never_delete_data() -> Result<()> {
    let (a, b, c) = (node(1), node(2), node(3));
    let lists = scope(&[Collection::Playlists]);
    let likes = scope(&[Collection::Likes]);
    let both = scope(&[Collection::Playlists, Collection::Likes]);
    for n in 0..100 {
        a.write_local(key(1), support::playlist(&n.to_string()))
            .await?;
    }
    copy(&b, &a, &lists).await?;
    copy(&a, &c, &lists).await?;
    assert!(a.get(&key(1)).await?.is_some());
    let late = c
        .write_local(key(2), support::playlist("late old revision"))
        .await?;
    assert_eq!(late.revision.counter, 1);
    copy(&a, &c, &lists).await?;
    copy(&b, &a, &lists).await?;
    assert_eq!(b.get(&key(2)).await?, Some(late));
    a.write_local(
        ObjectKey::liked(id(9)),
        Some(Payload::Liked(Liked {
            track_id: id(9),
            liked: true,
            updated_at: "original".into(),
        })),
    )
    .await?;
    let phone = node(4);
    copy(&phone, &a, &likes).await?;
    assert!(phone.get(&key(1)).await?.is_none());
    copy(&phone, &a, &both).await?;
    assert!(phone.get(&key(1)).await?.is_some());
    copy(&phone, &a, &likes).await?;
    assert!(phone.get(&key(1)).await?.is_some());
    Ok(())
}

#[tokio::test]
async fn receiving_stable_references_before_parents_and_replaying_statistics() -> Result<()> {
    let (a, b) = (node(1), node(2));
    let relation = ObjectKey::track_album(id(6));
    let stats = support::key(
        Collection::PlaybackStatistics,
        Uuid::new_v5(&id(6), id(1).as_bytes()),
    );
    a.write_local(
        relation.clone(),
        Some(Payload::TrackAlbum(TrackAlbum {
            track_id: id(6),
            album_id: id(7),
            track_number: Some(1),
        })),
    )
    .await?;
    a.write_local(
        stats.clone(),
        Some(Payload::PlaybackStatistics(PlaybackStatistics {
            track_id: id(6),
            device_id: id(1),
            skipped: 0,
            played_through: 1,
            updated_at: "one".into(),
        })),
    )
    .await?;
    let selected = scope(&[Collection::TrackAlbums, Collection::PlaybackStatistics]);
    copy(&b, &a, &selected).await?;
    let head = b.head().await?;
    copy(&b, &a, &selected).await?;
    assert_eq!(b.head().await?, head);
    assert!(b.get(&relation).await?.is_some());
    assert!(b.get(&stats).await?.is_some());
    Ok(())
}

#[tokio::test]
async fn replacement_generation_bootstraps_and_stale_requests_are_rejected() -> Result<()> {
    let (a, b) = (node(1), node(2));
    let selected = scope(&[Collection::Playlists]);
    a.write_local(key(1), support::playlist("one")).await?;
    copy(&b, &a, &selected).await?;
    let old = a.identity().await?;
    let replacement = node(1);
    assert_ne!(old.generation, replacement.identity().await?.generation);
    let initial = replacement.head().await?;
    replacement
        .write_local(key(2), support::playlist("two"))
        .await?;
    let request = p::PageRequest {
        source: replacement.identity().await?,
        scope: selected.clone(),
        after: initial,
        through: replacement.head().await?,
        limits: support::limits().pages,
    };
    let p::PageRead::Page(page) = replacement.read_page(request.clone()).await? else {
        anyhow::bail!("fixture exceeded one page");
    };
    assert!(b.apply_page(&page, support::limits().apply).await.is_err());
    copy(&b, &replacement, &selected).await?;
    assert!(b.get(&key(1)).await?.is_some());
    assert!(b.get(&key(2)).await?.is_some());
    assert!(
        replacement
            .read_page(p::PageRequest {
                source: old,
                ..request
            })
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn fixed_session_boundary_defers_new_writes_without_losing_them() -> Result<()> {
    let (a, b) = (node(1), node(2));
    let selected = scope(&[Collection::Playlists]);
    copy(&b, &a, &selected).await?;
    let source = a.identity().await?;
    let ck = p::CheckpointKey {
        namespace: source.namespace,
        source_replica: source.replica,
        source_generation: source.generation,
        scope_id: p::scope_id(&selected)?,
    };
    let after = b.checkpoint(&ck).await?.unwrap();
    a.write_local(key(1), support::playlist("before scan"))
        .await?;
    let through = a.head().await?;
    a.write_local(key(99), support::playlist("arrived during scan"))
        .await?;
    let p::PageRead::Page(page) = a
        .read_page(p::PageRequest {
            source,
            scope: selected.clone(),
            after,
            through: through.clone(),
            limits: support::limits().pages,
        })
        .await?
    else {
        anyhow::bail!("fixture exceeded one page");
    };
    assert!(page.done);
    b.apply_page(&page, support::limits().apply).await?;
    assert_eq!(b.checkpoint(&ck).await?, Some(through));
    assert!(b.get(&key(99)).await?.is_none());
    copy(&b, &a, &selected).await?;
    assert!(b.get(&key(99)).await?.is_some());
    Ok(())
}

#[tokio::test]
async fn independent_like_creation_uses_one_track_identity() -> Result<()> {
    let (a, b) = (node(1), node(2));
    let track_id = id(42);
    let key = ObjectKey::liked(track_id);
    a.write_local(
        key.clone(),
        Some(Payload::Liked(Liked {
            track_id,
            liked: false,
            updated_at: "a".into(),
        })),
    )
    .await?;
    let winner = b
        .write_local(
            key.clone(),
            Some(Payload::Liked(Liked {
                track_id,
                liked: true,
                updated_at: "b".into(),
            })),
        )
        .await?;
    let selected = scope(&[Collection::Likes]);
    copy(&a, &b, &selected).await?;
    copy(&b, &a, &selected).await?;
    assert_eq!(a.get(&key).await?, Some(winner.clone()));
    assert_eq!(b.get(&key).await?, Some(winner));
    assert!(
        a.write_local(
            ObjectKey::liked(id(999)),
            Some(Payload::Liked(Liked {
                track_id,
                liked: true,
                updated_at: "invalid".into()
            }))
        )
        .await
        .is_err()
    );
    Ok(())
}
