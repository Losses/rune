use anyhow::Result;
use database::sync::{model::*, references};
use database::test_fixtures::causal_library as support;
use uuid::Uuid;

#[tokio::test]
async fn reference_pages_release_reads_and_observe_parent_arrival() -> Result<()> {
    let library = support::SqlLibrary::new(Uuid::new_v4(), Uuid::new_v4()).await?;
    let album = migration::catalog_identity::parent(
        migration::catalog_identity::ParentKind::Album,
        "Late album",
    );
    for id in 1..=3 {
        let track = Uuid::from_u128(id);
        let mut value = support::track();
        if let Payload::Track(value) = &mut value {
            value.metadata.clear();
        }
        library
            .peer
            .write_local(support::key(Collection::Tracks, track), Some(value))
            .await?;
        library
            .peer
            .write_local(
                support::key(Collection::TrackAlbums, track),
                Some(Payload::TrackAlbum(TrackAlbum {
                    track_id: track,
                    album_id: album,
                    track_number: None,
                })),
            )
            .await?;
    }
    let store = library.sync_store.record_store();
    let first = references::page(&library.db, &store, Collection::TrackAlbums, None, 1).await?;
    assert!(!first.done);
    assert_eq!(first.missing.len(), 1);
    assert_eq!(first.missing[0].owner.id, Uuid::from_u128(1));
    assert_eq!(
        first.missing[0].target,
        support::key(Collection::Albums, album)
    );
    // This write uses the same one-connection pool between diagnostic pages.
    library
        .peer
        .write_local(
            support::key(Collection::Albums, album),
            Some(Payload::Album(Album {
                name: "Late album".into(),
                group: "test".into(),
            })),
        )
        .await?;
    let second =
        references::page(&library.db, &store, Collection::TrackAlbums, first.next, 1).await?;
    assert!(second.missing.is_empty());
    assert!(!second.done);
    assert_eq!(second.next, Some(Uuid::from_u128(2)));
    let last =
        references::page(&library.db, &store, Collection::TrackAlbums, second.next, 1).await?;
    assert!(last.missing.is_empty());
    assert!(last.done);
    assert!(
        references::page(&library.db, &store, Collection::TrackAlbums, None, 0)
            .await
            .is_err()
    );
    assert!(
        references::page(&library.db, &store, Collection::TrackAlbums, None, 33)
            .await
            .is_err()
    );
    Ok(())
}
