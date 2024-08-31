use std::path::PathBuf;

use database::actions::metadata::{empty_progress_callback, scan_audio_library};
use database::connection::{connect_main_db, connect_search_db};

#[tokio::main]
async fn main() {
    let path = ".";
    let main_db = connect_main_db(path).await.unwrap();
    let mut search_db = connect_search_db(path).unwrap();

    // Get the first command line argument.
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().expect("Audio data path not provided");

    let root_path = PathBuf::from(&path);

    scan_audio_library(
        &main_db,
        &mut search_db,
        &root_path,
        true,
        empty_progress_callback,
        None,
    )
    .await;

    println!("OK");
}
