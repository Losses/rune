use std::path::PathBuf;

use database::actions::metadata::{empty_progress_callback, scan_audio_library};
use database::connection::connect_main_db;

#[tokio::main]
async fn main() {
    let path = ".";
    let main_db = connect_main_db(path, None, "").await.unwrap();

    // Get the first command line argument.
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().expect("Audio data path not provided");

    let root_path = PathBuf::from(&path);

    let _ = scan_audio_library(
        &main_db,
        &root_path,
        true,
        false,
        empty_progress_callback,
        None,
    )
    .await;

    println!("OK");
}
