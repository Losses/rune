use std::sync::Arc;

use anyhow::{Context, Result};
use futures::future::join_all;
use rinf::DartSignal;

use database::actions::library::get_latest_albums_and_artists;
use database::connection::MainDbConnection;
use database::connection::RecommendationDbConnection;

use crate::messages::library_home::FetchLibrarySummaryRequest;
use crate::messages::library_home::LibrarySummaryResponse;

use crate::Collection;

pub async fn fetch_library_summary_request(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    dart_signal: DartSignal<FetchLibrarySummaryRequest>,
) -> Result<()> {
    let library = get_latest_albums_and_artists(&main_db)
        .await
        .with_context(|| "Failed to fetch library summary")?;

    let bake = dart_signal.message.bake_cover_arts;

    let albums = join_all(library.0.into_iter().map(|x| {
        let main_db = Arc::clone(&main_db);
        let recommend_db = Arc::clone(&recommend_db);

        Collection::from_model_bakeable(main_db, recommend_db, x, 0, "lib::album", bake)
    }))
    .await
    .into_iter()
    .collect::<Result<Vec<_>>>()?;

    let artists = join_all(library.1.clone().into_iter().map(|x| {
        let main_db = Arc::clone(&main_db);
        let recommend_db = Arc::clone(&recommend_db);

        Collection::from_model_bakeable(main_db, recommend_db, x, 1, "lib::artist", bake)
    }))
    .await
    .into_iter()
    .collect::<Result<Vec<_>>>()?;

    LibrarySummaryResponse { albums, artists }.send_signal_to_dart();

    Ok(())
}
