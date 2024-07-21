mod common;
mod connection;
mod media_file;
mod messages;
mod playback;
use database::connection::connect_main_db;

pub use tokio;

use crate::connection::*;
use crate::media_file::*;
use crate::playback::*;

rinf::write_interface!();

async fn main() {
    // Start receiving the media library path
    tokio::spawn(receive_media_library_path());

    // Ensure that the path is set before calling fetch_media_files
    loop {
        if let Some(path) = get_media_library_path().await {
            println!("Media Library Received, initialize other receivers");
            // Move the path into the async block
            tokio::spawn(async move {
                let db = connect_main_db(&path).await.unwrap();
                let _ = fetch_media_files(&db).await;
                let _ = handle_playback().await;
            });
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}
