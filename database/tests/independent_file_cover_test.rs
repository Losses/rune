use anyhow::Result;
use database::{
    playing_item::dispatcher::PlayingItemActionDispatcher, test_fixtures::fixtures::audio_fixture,
};
use fsio::FsIo;
use playback::player::PlayingItem;
use sea_orm::DatabaseConnection;

/// A file outside any library is described without a library's database: the player hands over
/// a connection that was never opened.
#[tokio::test]
async fn a_file_outside_the_library_is_described_without_a_database() -> Result<()> {
    let dispatcher = PlayingItemActionDispatcher::new();
    let fsio = FsIo::new();
    let no_database = DatabaseConnection::default();
    let items = [PlayingItem::IndependentFile(
        audio_fixture().to_string_lossy().into_owned(),
    )];

    let summaries = dispatcher
        .get_metadata_summary(&fsio, &no_database, &items)
        .await?;
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].item, items[0]);

    dispatcher
        .bake_cover_art(&fsio, "", &no_database, &items)
        .await?;
    Ok(())
}
