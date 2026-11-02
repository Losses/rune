#[path = "support/causal_library.rs"]
mod support;
use anyhow::Result;
use database::{
    actions::mixes::{create_mix, executable_mix_queries},
    sync::model::*,
};
use support::{SqlLibrary, key};
use uuid::Uuid;

#[tokio::test]
async fn deleting_referenced_playlist_does_not_block_unrelated_sync() -> Result<()> {
    let node = Uuid::new_v4();
    let namespace = Uuid::new_v4();
    let local = SqlLibrary::new(namespace, node).await?;
    let store = local.library.record_store();
    let playlist = database::actions::playlists::create_playlist(
        &local.db,
        &store,
        "Referenced".into(),
        "Test".into(),
    )
    .await?;
    let mix = create_mix(
        &local.db,
        &store,
        "Uses playlist".into(),
        "Test".into(),
        false,
        0,
        false,
        vec![("lib::playlist".into(), playlist.id.to_string())],
    )
    .await?;
    database::actions::playlists::remove_playlist(&local.db, &store, playlist.id).await?;
    let head = local.peer.head().await?;
    assert_eq!(local.peer.head().await?, head);
    assert!(executable_mix_queries(&local.db, mix.id).await.is_err());
    assert_eq!(local.library.pending_mixes().await?.len(), 1);
    let source = support::memory(namespace, Uuid::new_v4())?;
    let object = key(Collection::Playlists, Uuid::new_v4());
    source
        .write_local(object.clone(), support::playlist("Unrelated"))
        .await?;
    support::exchange_scoped(
        &source,
        &local.peer,
        Scope {
            collections: [Collection::Playlists].into_iter().collect(),
        },
    )
    .await?;
    assert!(local.peer.get(&object).await?.is_some());
    assert_eq!(local.library.pending_mixes().await?.len(), 1);
    assert!(executable_mix_queries(&local.db, mix.id).await.is_err());
    Ok(())
}
