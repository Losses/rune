use std::path::PathBuf;

use database::connection::connect_main_db;
use database::actions::metadata::scan_audio_library;

#[tokio::main]
async fn main() {
    let path = ".";
    let db = connect_main_db(path).await;

    // Get the first command line argument.
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().expect("Audio data path not provided");

    let root_path = PathBuf::from(&path);

    scan_audio_library(&db, &root_path, true).await;

    println!("OK");
}
