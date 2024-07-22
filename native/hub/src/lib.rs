mod common;
mod connection;
mod media_file;
mod messages;
mod playback;

use log::info;
use std::sync::Arc;
use tracing_subscriber::filter::EnvFilter;

use database::connection::connect_main_db;

pub use tokio;

use crate::connection::*;
use crate::media_file::*;
use crate::playback::*;

rinf::write_interface!();

async fn main() {
    let filter = EnvFilter::new(
        "symphonia_format_ogg=off,symphonia_core=off,sea_orm_migration::migrator=off,info",
    );

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .init();

    // Start receiving the media library path
    tokio::spawn(receive_media_library_path());

    // Ensure that the path is set before calling fetch_media_files
    loop {
        if let Some(path) = get_media_library_path().await {
            info!("Media Library Received, initialize other receivers");
            // Move the path into the async block
            let db = Arc::new(connect_main_db(&path).await.unwrap());
            info!("Initializing fetchers");
            // Pass the cloned Arc directly
            tokio::spawn(fetch_media_files(db.clone()));
            info!("Initializing playback");
            tokio::spawn(handle_playback(db.clone()));
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}
