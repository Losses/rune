use database::test_fixtures::causal_library as support;

use anyhow::Result;
use database::{
    entities::media_files, playing_item::PlayingFileMetadataProvider,
    playing_item::library_item::LibraryItemProcessor,
};
use fsio::FsIo;
use playback::player::PlayingItem;
use sea_orm::{EntityTrait, Set};
use uuid::Uuid;

async fn insert_file(
    db: &sea_orm::DatabaseConnection,
    file_uuid: Uuid,
    directory: &str,
    file_name: &str,
    local_available: bool,
) -> Result<i32> {
    Ok(media_files::Entity::insert(media_files::ActiveModel {
        file_name: Set(file_name.into()),
        directory: Set(directory.into()),
        extension: Set("flac".into()),
        file_hash: Set(String::new()),
        last_modified: Set(String::new()),
        sample_rate: Set(1),
        duration: Set(1.into()),
        source_uuid: Set(file_uuid.to_string()),
        local_available: Set(local_available),
        ..Default::default()
    })
    .exec(db)
    .await?
    .last_insert_id)
}

/// A synced track whose audio is still remote-only must not resolve to any
/// host path. With an empty directory and filename the bare join collapses
/// onto the library root, so a sidecar sitting beside that root — named after
/// the root itself — would be picked up as the track's lyric. The processor
/// must yield no path for such a track, and the root trap sidecar exists here
/// only to prove the regression shape.
#[tokio::test]
async fn non_materialized_track_yields_no_host_path() -> Result<()> {
    let library_id = Uuid::new_v4();
    let library = support::SqlLibrary::new(library_id, Uuid::new_v4()).await?;
    let root = library.root.path().to_path_buf();

    // The trap: a timed lyric file named after the library root, beside it.
    let trap = root.with_extension("lrc");
    std::fs::write(&trap, "[00:01.00] unrelated root sidecar")?;

    let remote_uuid = Uuid::new_v4();
    insert_file(&library.db, remote_uuid, "", "", false).await?;

    let item = PlayingItem::LibraryTrack {
        library_id,
        track: remote_uuid.to_string(),
    };
    let fsio = FsIo::new();
    let paths = LibraryItemProcessor
        .get_file_path(&fsio, &root, &library.db, std::slice::from_ref(&item))
        .await?;

    assert!(
        !paths.contains_key(&item),
        "remote-only track must not resolve to a host path, got {:?}",
        paths.get(&item)
    );
    Ok(())
}

/// A track marked locally available but still carrying an empty filename —
/// the projection shape before download materialization completes — must not
/// resolve either, since the join would collapse onto the root.
#[tokio::test]
async fn empty_filename_yields_no_host_path() -> Result<()> {
    let library_id = Uuid::new_v4();
    let library = support::SqlLibrary::new(library_id, Uuid::new_v4()).await?;
    let root = library.root.path().to_path_buf();

    let uuid = Uuid::new_v4();
    insert_file(&library.db, uuid, "", "", true).await?;

    let item = PlayingItem::LibraryTrack {
        library_id,
        track: uuid.to_string(),
    };
    let fsio = FsIo::new();
    let paths = LibraryItemProcessor
        .get_file_path(&fsio, &root, &library.db, std::slice::from_ref(&item))
        .await?;

    assert!(
        !paths.contains_key(&item),
        "track without a filename must not resolve to a host path, got {:?}",
        paths.get(&item)
    );
    Ok(())
}

/// Positive control: a materialized track still resolves to its own path, so
/// its own sidecar remains reachable for lyric lookup.
#[tokio::test]
async fn materialized_track_resolves_to_its_own_path() -> Result<()> {
    let library_id = Uuid::new_v4();
    let library = support::SqlLibrary::new(library_id, Uuid::new_v4()).await?;
    let root = library.root.path().to_path_buf();

    std::fs::create_dir_all(root.join("music"))?;
    let audio = root.join("music").join("song.flac");
    std::fs::write(&audio, b"fLaC")?;

    let uuid = Uuid::new_v4();
    insert_file(&library.db, uuid, "music", "song.flac", true).await?;

    let item = PlayingItem::LibraryTrack {
        library_id,
        track: uuid.to_string(),
    };
    let fsio = FsIo::new();
    let paths = LibraryItemProcessor
        .get_file_path(&fsio, &root, &library.db, std::slice::from_ref(&item))
        .await?;

    assert_eq!(
        paths.get(&item),
        Some(&fsio.canonicalize_path(&audio)?),
        "materialized track must resolve to its own file"
    );
    Ok(())
}
