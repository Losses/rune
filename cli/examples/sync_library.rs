use std::{path::PathBuf, sync::Arc};

use database::{
    actions::metadata::{empty_progress_callback, scan_audio_library},
    connection::connect_main_db,
};
use fsio::FsIo;

#[tokio::main]
async fn main() {
    let path = ".";
    let fsio = FsIo::new();
    let main_db = connect_main_db(&fsio, path, None, "").await.unwrap();
    let node_id = "RUNE_EXAMPLE";

    // Get the first command line argument.
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().expect("Audio data path not provided");

    let root_path = PathBuf::from(&path);
    let fsio = Arc::new(FsIo::new());

    let _ = scan_audio_library(
        &fsio,
        &main_db,
        node_id,
        &root_path,
        true,
        false,
        empty_progress_callback,
        None,
    )
    .await;

    println!("OK");
}
