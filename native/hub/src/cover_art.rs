use std::sync::Arc;

use log::{debug, error, info, warn};
use rinf::DartSignal;

use database::actions::cover_art::get_cover_art_by_id;
use database::actions::cover_art::get_random_cover_art_ids;
use database::actions::cover_art::sync_cover_art_by_file_id;
use database::connection::MainDbConnection;

use crate::{
    CoverArtByCoverArtIdResponse, CoverArtByFileIdResponse, GetCoverArtByCoverArtIdRequest,
    GetCoverArtByFileIdRequest, GetRandomCoverArtIdsRequest, GetRandomCoverArtIdsResponse,
};

pub async fn get_cover_art_by_file_id_request(
    main_db: Arc<MainDbConnection>,
    lib_path: Arc<String>,
    dart_signal: DartSignal<GetCoverArtByFileIdRequest>,
) {
    let request = dart_signal.message;
    let file_id = request.file_id;

    debug!("Requesting cover art by file ID: {}", file_id);

    match sync_cover_art_by_file_id(&main_db, &lib_path, file_id).await {
        Ok(cover_art) => {
            match cover_art {
                Some((cover_art_id, cover_art)) => {
                    if !cover_art.is_empty() {
                        CoverArtByFileIdResponse {
                            file_id,
                            cover_art_id,
                            cover_art: Some(cover_art),
                        }
                        .send_signal_to_dart();
                        // GENERATED
                    } else {
                        CoverArtByFileIdResponse {
                            file_id,
                            cover_art_id,
                            cover_art: None,
                        }
                        .send_signal_to_dart();
                        // GENERATED
                    }
                }
                _none => {
                    CoverArtByFileIdResponse {
                        file_id,
                        cover_art_id: -1,
                        cover_art: None,
                    }
                    .send_signal_to_dart();
                    // GENERATED
                    info!("No cover art found: {}", file_id);
                }
            }
        }
        Err(e) => {
            CoverArtByFileIdResponse {
                file_id,
                cover_art_id: -1,
                cover_art: None,
            }
            .send_signal_to_dart();
            // GENERATED
            warn!("Cover art request failed: {}: {:?}", file_id, e);
        }
    }
}

pub async fn get_cover_art_by_cover_art_id_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<GetCoverArtByCoverArtIdRequest>,
) {
    let cover_art_id = dart_signal.message.cover_art_id;

    debug!("Requesting cover art by cover art ID: {}", cover_art_id);

    match get_cover_art_by_id(&main_db, cover_art_id).await {
        Ok(entry) => match entry {
            Some(entry) => CoverArtByCoverArtIdResponse {
                cover_art_id,
                cover_art: Some(entry),
            }
            .send_signal_to_dart(),
            _none => CoverArtByCoverArtIdResponse {
                cover_art_id,
                cover_art: None,
            }
            .send_signal_to_dart(),
        },
        Err(_) => CoverArtByCoverArtIdResponse {
            cover_art_id,
            cover_art: None,
        }
        .send_signal_to_dart(),
    };
}

pub async fn get_random_cover_art_ids_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<GetRandomCoverArtIdsRequest>,
) {
    let count = dart_signal.message.count;

    match get_random_cover_art_ids(&main_db, count as usize).await {
        Ok(items) => GetRandomCoverArtIdsResponse {
            cover_art_ids: items.into_iter().map(|x| x.id).collect(),
        }
        .send_signal_to_dart(),
        Err(_) => {
            GetRandomCoverArtIdsResponse {
                cover_art_ids: Vec::new(),
            }
            .send_signal_to_dart();
            error!("Unable to get random cover art ids");
        }
    }
}
