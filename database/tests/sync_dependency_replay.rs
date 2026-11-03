use support::fixtures;

#[path = "support/causal_library.rs"]
mod support;
use anyhow::Result;
use database::entities::{artists, media_files};
use database::{
    actions::{
        mixes::{executable_mix_queries, query_mix_media_files},
        stats::get_liked,
    },
    connection::RecommendationDbConnection,
    entities::{media_file_artists, media_file_playlists, mix_queries, mixes},
    sync::model::*,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use support::{Peer, SqlLibrary, key};
use sync::{self as p, ReceiveStore};
use uuid::Uuid;
fn scope(collections: &[Collection]) -> Scope {
    Scope {
        collections: collections.iter().copied().collect(),
    }
}
async fn page(
    source: &Peer,
    selected: &Scope,
    after: p::ReceiveProgress,
) -> Result<p::ChangePage<RuneModel>> {
    let p::PageRead::Page(page) = source
        .read_page(p::PageRequest {
            source: source.identity().await?,
            scope: selected.clone(),
            after,
            through: source.head().await?,
            limits: p::PageLimits {
                max_scanned: 1,
                max_bytes: 65536,
            },
        })
        .await?
    else {
        anyhow::bail!("fixture exceeded one page");
    };
    Ok(page)
}

#[tokio::test]
async fn reference_before_parent_survives_endpoint_restart_and_parent_removal() -> Result<()> {
    let ns = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let local = SqlLibrary::new(ns, actor).await?;
    let source = support::memory(ns, Uuid::new_v4())?;
    let selected = scope(&[Collection::Mixes, Collection::Playlists]);
    let after = local.initialize_from(&source, &selected).await?;
    let parent = Uuid::new_v4();
    let mix = Uuid::new_v4();
    source
        .write_local(
            key(Collection::Mixes, mix),
            Some(Payload::Mix(Mix {
                name: "pending".into(),
                group: "p".into(),
                mode: None,
                scriptlet_mode: false,
                locked: false,
                queries: vec![MixClause {
                    id: Uuid::new_v4(),
                    position: 0,
                    operator: "lib::playlist".into(),
                    parameter: String::new(),
                    target_uuid: Some(parent),
                    group: 0,
                }],
            })),
        )
        .await?;
    source
        .write_local(
            key(Collection::Playlists, parent),
            support::playlist("parent"),
        )
        .await?;
    let first = page(&source, &selected, after).await?;
    assert!(!first.done);
    local
        .peer
        .apply_page(&first, support::limits().apply)
        .await?;
    assert_eq!(local.library.pending_mixes().await?.len(), 1);
    let restarted = fixtures::open_library(local.db.clone(), actor, Some(ns)).await?;
    let second = page(&source, &selected, first.next).await?;
    restarted
        .endpoint()
        .apply_page(&second, support::limits().apply)
        .await?;
    assert!(restarted.pending_mixes().await?.is_empty());
    let row = mixes::Entity::find()
        .filter(mixes::Column::HlcUuid.eq(mix.to_string()))
        .one(&local.db)
        .await?
        .unwrap();
    let clauses = mix_queries::Entity::find()
        .filter(mix_queries::Column::MixUuid.eq(mix.to_string()))
        .all(&local.db)
        .await?;
    assert_eq!(clauses.len(), 1);
    source
        .write_local(key(Collection::Playlists, parent), None)
        .await?;
    let removed = page(&source, &selected, second.next).await?;
    restarted
        .endpoint()
        .apply_page(&removed, support::limits().apply)
        .await?;
    assert_eq!(restarted.pending_mixes().await?.len(), 1);
    assert_eq!(
        mix_queries::Entity::find()
            .filter(mix_queries::Column::MixUuid.eq(mix.to_string()))
            .all(&local.db)
            .await?,
        clauses
    );
    assert!(executable_mix_queries(&local.db, row.id).await.is_err());
    let reject = key(Collection::Playlists, Uuid::new_v4());
    source
        .write_local(reject.clone(), support::playlist("reject"))
        .await?;
    local.db.execute_unprepared("CREATE TRIGGER reject_playlist BEFORE INSERT ON playlists WHEN NEW.name = 'reject' BEGIN SELECT RAISE(ABORT, 'injected'); END").await?;
    let failed = page(&source, &selected, removed.next.clone()).await?;
    assert!(
        restarted
            .endpoint()
            .apply_page(&failed, support::limits().apply)
            .await
            .is_err()
    );
    let identity = source.identity().await?;
    let checkpoint = p::CheckpointKey {
        namespace: ns,
        source_replica: identity.replica,
        source_generation: identity.generation,
        scope_id: p::scope_id(&selected)?,
    };
    assert_eq!(
        restarted.endpoint().checkpoint(&checkpoint).await?,
        Some(removed.next)
    );
    assert!(restarted.get(&reject).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn changed_entry_does_not_show_stale_parent_while_waiting() -> Result<()> {
    let ns = Uuid::new_v4();
    let local = SqlLibrary::new(ns, Uuid::new_v4()).await?;
    let source = support::memory(ns, Uuid::new_v4())?;
    let selected = scope(&[Collection::Playlists, Collection::PlaylistEntries]);
    let mut after = local.initialize_from(&source, &selected).await?;
    let track = Uuid::new_v4();
    support::local_file(&local.db, 1, track, "song").await?;
    let old = Uuid::new_v4();
    let new = Uuid::new_v4();
    let entry = Uuid::new_v4();
    let child = |playlist_id| {
        Some(Payload::PlaylistEntry(PlaylistEntry {
            playlist_id,
            track_id: track,
            position: 0,
        }))
    };
    source
        .write_local(key(Collection::Playlists, old), support::playlist("old"))
        .await?;
    source
        .write_local(key(Collection::PlaylistEntries, entry), child(old))
        .await?;
    source
        .write_local(key(Collection::PlaylistEntries, entry), child(new))
        .await?;
    source
        .write_local(key(Collection::Playlists, new), support::playlist("new"))
        .await?;
    for sequence in 1..=4 {
        let page = page(&source, &selected, after).await?;
        local
            .peer
            .apply_page(&page, support::limits().apply)
            .await?;
        after = page.next;
        if sequence >= 2 {
            let row = media_file_playlists::Entity::find()
                .one(&local.db)
                .await?
                .unwrap();
            assert_eq!(row.hlc_uuid, entry.to_string());
            assert_eq!(
                row.playlist_uuid,
                if sequence == 2 { old } else { new }.to_string()
            );
        }
        if sequence == 3 {
            assert_eq!(local.library.pending_references().await?.len(), 1);
        }
    }
    assert!(local.library.pending_references().await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn file_artist_keeps_membership_while_parents_arrive_and_return() -> Result<()> {
    use migration::catalog_identity::{self, ParentKind};
    let ns = Uuid::new_v4();
    let local = SqlLibrary::new(ns, Uuid::new_v4()).await?;
    let source = support::memory(ns, Uuid::new_v4())?;
    let selected = scope(&[
        Collection::Tracks,
        Collection::Artists,
        Collection::FileArtists,
    ]);
    let mut after = local.initialize_from(&source, &selected).await?;
    let track = Uuid::new_v4();
    let artist = catalog_identity::parent(ParentKind::Artist, "Exact name");
    let membership = catalog_identity::membership(track, ParentKind::Artist, artist);
    let artist_value = || {
        Some(Payload::Artist(Artist {
            name: "Exact name".into(),
            group: "E".into(),
        }))
    };
    source
        .write_local(
            key(Collection::FileArtists, membership),
            Some(Payload::FileArtist(FileArtist {
                file_id: track,
                artist_id: artist,
            })),
        )
        .await?;
    source
        .write_local(key(Collection::Artists, artist), artist_value())
        .await?;
    source
        .write_local(key(Collection::Tracks, track), Some(support::track()))
        .await?;
    let mut link = None;
    for sequence in 1..=3 {
        let page = page(&source, &selected, after).await?;
        local
            .peer
            .apply_page(&page, support::limits().apply)
            .await?;
        after = page.next;
        let rows = media_file_artists::Entity::find().all(&local.db).await?;
        assert_eq!(rows.len(), 1);
        if let Some(previous) = &link {
            assert_eq!(&rows[0], previous);
        } else {
            link = Some(rows[0].clone());
        }
        assert_eq!(
            local.library.pending_references().await?.len(),
            3 - sequence
        );
    }
    source
        .write_local(key(Collection::Artists, artist), None)
        .await?;
    let removed = page(&source, &selected, after).await?;
    local
        .peer
        .apply_page(&removed, support::limits().apply)
        .await?;
    assert_eq!(
        media_file_artists::Entity::find().one(&local.db).await?,
        link
    );
    assert_eq!(local.library.pending_references().await?.len(), 1);
    source
        .write_local(key(Collection::Artists, artist), artist_value())
        .await?;
    let restored = page(&source, &selected, removed.next).await?;
    local
        .peer
        .apply_page(&restored, support::limits().apply)
        .await?;
    assert_eq!(
        media_file_artists::Entity::find().one(&local.db).await?,
        link
    );
    assert!(local.library.pending_references().await?.is_empty());
    let artist_row = artists::Entity::find().one(&local.db).await?.unwrap();
    assert_eq!(
        query_mix_media_files(
            &local.db,
            &RecommendationDbConnection::new(),
            vec![("lib::artist".into(), artist_row.id.to_string())],
            0,
            20
        )
        .await?
        .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn source_uuid_resolves_reference_without_a_second_identity_registry() -> Result<()> {
    let ns = Uuid::new_v4();
    let local = SqlLibrary::new(ns, Uuid::new_v4()).await?;
    let source = support::memory(ns, Uuid::new_v4())?;
    let selected = scope(&[Collection::Likes, Collection::Tracks]);
    let after = local.initialize_from(&source, &selected).await?;
    let track = Uuid::new_v4();
    support::local_file(&local.db, 1, track, "local").await?;
    source
        .write_local(
            key(Collection::Likes, track),
            Some(Payload::Liked(Liked {
                track_id: track,
                liked: true,
                updated_at: "received".into(),
            })),
        )
        .await?;
    source
        .write_local(key(Collection::Tracks, track), Some(support::track()))
        .await?;
    let first = page(&source, &selected, after).await?;
    local
        .peer
        .apply_page(&first, support::limits().apply)
        .await?;
    assert!(get_liked(&local.db, 1).await?);
    assert!(local.library.pending_references().await?.is_empty());
    let second = page(&source, &selected, first.next).await?;
    local
        .peer
        .apply_page(&second, support::limits().apply)
        .await?;
    assert!(get_liked(&local.db, 1).await?);
    assert_eq!(media_files::Entity::find().all(&local.db).await?.len(), 1);
    Ok(())
}
