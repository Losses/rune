#[path = "support/causal_library.rs"]
mod support;
use anyhow::Result;
use database::{
    actions::index::index_audio_library,
    entities::{albums, media_file_albums},
    sync::model::*,
};
use sea_orm::{EntityTrait, PaginatorTrait};
use support::{SqlLibrary, key};
use sync::{self as p};
use uuid::Uuid;

#[tokio::test]
async fn cleanup_during_partial_sync_does_not_delete_source_album() -> Result<()> {
    let namespace = Uuid::new_v4();
    let source = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let target = SqlLibrary::new(namespace, Uuid::new_v4()).await?;
    let scope = Scope {
        collections: [
            Collection::Albums,
            Collection::Tracks,
            Collection::TrackAlbums,
        ]
        .into_iter()
        .collect(),
    };
    let after = target.initialize_from(&source.peer, &scope).await?;
    let album = migration::catalog_identity::parent(
        migration::catalog_identity::ParentKind::Album,
        "Album",
    );
    let track = Uuid::new_v4();
    source
        .peer
        .write_local(
            key(Collection::Albums, album),
            Some(Payload::Album(Album {
                name: "Album".into(),
                group: "Artist".into(),
            })),
        )
        .await?;
    source
        .peer
        .write_local(key(Collection::Tracks, track), Some(support::track()))
        .await?;
    source
        .peer
        .write_local(
            key(Collection::TrackAlbums, track),
            Some(Payload::TrackAlbum(TrackAlbum {
                track_id: track,
                album_id: album,
                track_number: Some(3),
            })),
        )
        .await?;
    let p::PageRead::Page(page) = source
        .peer
        .read_page(p::PageRequest {
            source: source.peer.identity().await?,
            scope: scope.clone(),
            after,
            through: source.peer.head().await?,
            limits: p::PageLimits {
                max_scanned: 1,
                max_bytes: 65536,
            },
        })
        .await?
    else {
        anyhow::bail!("fixture exceeded one page");
    };
    assert!(page.next.sequence < page.request.through.sequence);
    target
        .peer
        .apply_page(&page, support::limits().apply)
        .await?;
    assert_eq!(albums::Entity::find().count(&target.db).await?, 1);
    assert_eq!(
        media_file_albums::Entity::find().count(&target.db).await?,
        0
    );
    let head = target.peer.head().await?;
    let actor = target.library.record_store().author().to_string();
    database::actions::metadata::scan_audio_library(
        &fsio::FsIo::new(),
        &target.db,
        &actor,
        target.root.path(),
        true,
        false,
        |_| {},
        None,
        &target.library.record_store(),
    )
    .await?;
    index_audio_library(&target.db, &actor, 16, None, &target.library.record_store()).await?;
    assert_eq!(target.peer.head().await?, head);
    assert_eq!(albums::Entity::find().count(&target.db).await?, 1);
    support::exchange_scoped(&target.peer, &source.peer, scope.clone()).await?;
    assert_eq!(albums::Entity::find().count(&source.db).await?, 1);
    assert_eq!(
        media_file_albums::Entity::find().count(&source.db).await?,
        1
    );
    support::exchange_scoped(&source.peer, &target.peer, scope).await?;
    assert_eq!(
        media_file_albums::Entity::find().count(&target.db).await?,
        1
    );
    Ok(())
}
