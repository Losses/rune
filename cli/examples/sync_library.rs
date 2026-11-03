use database::sync::LibrarySyncStore;
use database::{
    actions::metadata::{
        MissingHostFilePolicy, ScanOptions, empty_progress_callback, scan_audio_library,
    },
    connection::connect_main_db,
};
use std::{path::PathBuf, sync::Arc};

use fsio::FsIo;

#[tokio::main]
async fn main() {
    let path = ".";
    let fsio = FsIo::new();
    let node_id = uuid::Uuid::new_v4();
    let main_db = connect_main_db(&fsio, path, None, &node_id.to_string())
        .await
        .unwrap();

    // Get the first command line argument.
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().expect("Audio data path not provided");

    let root_path = PathBuf::from(&path);
    let fsio = Arc::new(FsIo::new());

    let library = LibrarySyncStore::open(main_db.clone(), node_id, None)
        .await
        .unwrap();
    library.import_existing(node_id).await.unwrap();
    let _ = scan_audio_library(
        &fsio,
        &main_db,
        &node_id.to_string(),
        &root_path,
        ScanOptions {
            cleanup: true,
            force: false,
            missing_host_file: MissingHostFilePolicy::RemoveEverywhere,
            cancel_token: None,
        },
        empty_progress_callback,
        &library.record_store(),
    )
    .await;

    println!("OK");
}
