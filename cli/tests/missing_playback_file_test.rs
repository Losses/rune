use std::{fs, sync::Mutex};

use anyhow::Result;
use database::{
    actions::{file::get_file_by_id, metadata::scan_audio_file},
    connection::connect_main_db,
    sync::LibrarySyncStore,
};
use fsio::FsIo;
use log::LevelFilter;
use log::{Log, Metadata, Record};
use rune::playback::{play_by_id, play_random};
use std::path::Path;
use uuid::Uuid;

struct PlaybackLog(Mutex<Vec<String>>);

impl Log for PlaybackLog {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "rune::playback"
    }

    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata()) {
            self.0.lock().unwrap().push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

static LOG: PlaybackLog = PlaybackLog(Mutex::new(Vec::new()));

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_catalog_entries_are_reported_and_valid_random_tracks_play() -> Result<()> {
    log::set_logger(&LOG)?;
    log::set_max_level(LevelFilter::Debug);
    let root = tempfile::tempdir()?;
    let root_path = root.path().canonicalize()?;
    let node = Uuid::new_v4();
    let library_id = Uuid::new_v4();
    let fsio = FsIo::new();
    let db = connect_main_db(&fsio, root_path.to_str().unwrap(), None, &node.to_string()).await?;
    let library = LibrarySyncStore::open(db.clone(), node, Some(library_id)).await?;
    let store = library.record_store();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/startup_0.ogg");
    let mut ids = Vec::new();
    for name in ["missing.ogg", "valid.ogg"] {
        let path = root_path.join(name);
        fs::copy(&fixture, &path)?;
        ids.push(scan_audio_file(&fsio, &db, &node.to_string(), &root_path, &path, &store).await?);
    }
    fs::remove_file(root_path.join("missing.ogg"))?;
    assert!(get_file_by_id(&db, ids[0]).await?.is_some());

    play_by_id(&db, &root_path, ids[0], &library_id.to_string()).await;
    let messages = LOG.0.lock().unwrap().drain(..).collect::<Vec<_>>();
    assert!(
        messages
            .iter()
            .any(|line| line.contains("Failed to canonicalize") && line.contains("missing.ogg")),
        "missing track was not reported: {messages:?}"
    );

    // Both rows are selected because the random limit exceeds the catalog size.
    play_random(&db, &root_path, &library_id.to_string()).await;
    let messages = LOG.0.lock().unwrap().drain(..).collect::<Vec<_>>();
    assert!(
        messages
            .iter()
            .any(|line| line.contains("Failed to canonicalize") && line.contains("missing.ogg")),
        "random playback did not report the missing track: {messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|line| line.contains("Player status updated:")
                && line.contains("valid.ogg")
                && line.contains("state: Playing")),
        "valid track did not enter playback: {messages:?}"
    );
    Ok(())
}
