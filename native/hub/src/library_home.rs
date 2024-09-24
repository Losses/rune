use std::sync::Arc;

use anyhow::{Context, Result};
use rinf::DartSignal;

use database::actions::library::get_latest_albums_and_artists;
use database::connection::MainDbConnection;

use crate::messages::album::Album;
use crate::messages::artist::Artist;
use crate::messages::library_home::FetchLibrarySummaryRequest;
use crate::messages::library_home::LibrarySummaryResponse;

pub async fn fetch_library_summary_request(
    main_db: Arc<MainDbConnection>,
    _dart_signal: DartSignal<FetchLibrarySummaryRequest>,
) -> Result<()> {
    let library = get_latest_albums_and_artists(&main_db)
        .await
        .with_context(|| "Failed to fetch library summary")?;
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

    Ok(())
}
