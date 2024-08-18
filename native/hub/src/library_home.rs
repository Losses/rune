use database::actions::library::get_latest_albums_and_artists;
use log::{debug, error};
use rinf::DartSignal;
use std::sync::Arc;

use database::connection::MainDbConnection;

use crate::messages::album::Album;
use crate::messages::artist::Artist;
use crate::messages::library_home::FetchLibrarySummaryRequest;
use crate::messages::library_home::LibrarySummaryResponse;

pub async fn fetch_library_summary_request(
    main_db: Arc<MainDbConnection>,
    _dart_signal: DartSignal<FetchLibrarySummaryRequest>,
) {
    debug!("Requesting library summary");

    match get_latest_albums_and_artists(&main_db).await {
        Ok(library) => {
            let albums = library
                .0
                .into_iter()
                .map(|x| Album {
                    id: x.0.id,
                    name: x.0.name,
                    cover_ids: x.1.into_iter().collect(),
                })
                .collect();

            let artists = library
                .1
                .into_iter()
                .map(|x| Artist {
                    id: x.0.id,
                    name: x.0.name,
                    cover_ids: x.1.into_iter().collect(),
                })
                .collect();

            LibrarySummaryResponse { albums, artists }.send_signal_to_dart();
            // GENERATED
        }
        Err(e) => {
            error!("Failed to fetch library summary: {}", e);
        }
    };
}
