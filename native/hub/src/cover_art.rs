use log::{info, warn};
use std::sync::Arc;

use database::actions::cover_art::get_cover_art;
use database::connection::MainDbConnection;

use crate::common::*;
use crate::messages;

pub async fn handle_cover_art(main_db: Arc<MainDbConnection>, lib_path: Arc<String>) -> Result<()> {
    use messages::cover_art::*;

    let mut receiver = CoverArtRequest::get_dart_signal_receiver()?; // GENERATED

    info!("Initializing event listeners.");

    while let Some(dart_signal) = receiver.recv().await {
        let request = dart_signal.message;
        let file_id = request.file_id;

        info!("Requesting cover art: {}", file_id);

        match get_cover_art(&main_db, &lib_path, file_id).await {
            Ok(cover_art) => {
                match cover_art {
                    Some(cover_art) => {
                        if !cover_art.is_empty() {
                            CoverArtResponse {
                                file_id,
                                cover_art: Some(cover_art),
                            }
                            .send_signal_to_dart();
                            // GENERATED
                        } else {
                            CoverArtResponse {
                                file_id,
                                cover_art: None,
                            }
                            .send_signal_to_dart();
                            // GENERATED
                        }
                    }
                    _none => {
                        CoverArtResponse {
                            file_id,
                            cover_art: None,
                        }
                        .send_signal_to_dart();
                        // GENERATED
                        info!("No cover art found: {}", file_id);
                    }
                }
            }
            Err(e) => {
                CoverArtResponse {
                    file_id,
                    cover_art: None,
                }
                .send_signal_to_dart();
                // GENERATED
                warn!("Cover art request failed: {}: {:?}", file_id, e);
            }
        }
    }

    Ok(())
}
