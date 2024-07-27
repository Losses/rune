use database::connection::MainDbConnection;
use log::info;
use std::sync::Arc;
use tokio::task;

use database::actions::cover_art::get_cover_art;

use crate::common::*;
use crate::messages;

pub async fn handle_cover_art(main_db: Arc<MainDbConnection>, lib_path: Arc<String>) -> Result<()> {
    use messages::cover_art::*;

    let mut receiver = CoverArtRequest::get_dart_signal_receiver()?; // GENERATED

    info!("Initializing event listeners.");

    while let Some(dart_signal) = receiver.recv().await {
        let request = dart_signal.message;
        let file_id = request.file_id;

        if let Ok(Some(cover_art)) = get_cover_art(&main_db, &lib_path, file_id).await {
            CoverArtResponse { file_id, cover_art }.send_signal_to_dart(); // GENERATED
        }
    }

    Ok(())
}
