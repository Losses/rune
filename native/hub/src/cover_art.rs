use log::{debug, info, warn};
use rinf::DartSignal;
use std::sync::Arc;

use database::actions::cover_art::sync_cover_art_by_file_id;
use database::connection::MainDbConnection;

use crate::messages::cover_art::CoverArtResponse;
use crate::messages::cover_art::GetCoverArtByFileIdRequest;

pub async fn get_cover_art_by_file_id_request(
    main_db: Arc<MainDbConnection>,
    lib_path: Arc<String>,
    dart_signal: DartSignal<GetCoverArtByFileIdRequest>,
) {
    let request = dart_signal.message;
    let file_id = request.file_id;

    debug!("Requesting cover art: {}", file_id);

    match sync_cover_art_by_file_id(&main_db, &lib_path, file_id).await {
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
