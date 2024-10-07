use std::sync::Arc;

use anyhow::{Context, Result};
use database::entities::media_files;
use futures::future::join_all;
use rinf::DartSignal;

use database::actions::cover_art::get_random_cover_art_ids;
use database::actions::mixes::query_mix_media_files;
use database::connection::MainDbConnection;
use database::connection::RecommendationDbConnection;

use crate::messages::*;

pub async fn get_random_cover_art_ids_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<GetRandomCoverArtIdsRequest>,
) -> Result<()> {
    let count = dart_signal.message.count;

    let paths: Vec<String> = get_random_cover_art_ids(&main_db, count as usize)
        .await
        .with_context(|| "Unable to get random cover art ids")?
        .values()
        .cloned()
        .collect();

    GetRandomCoverArtIdsResponse { paths }.send_signal_to_dart();

    Ok(())
}

pub async fn query_cover_arts(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    queries: Vec<MixQuery>,
) -> Result<Vec<media_files::Model>> {
    query_mix_media_files(
        &main_db,
        &recommend_db,
        queries
            .into_iter()
            .map(|q| (q.operator, q.parameter))
            .chain([("filter::with_cover_art".to_string(), "true".to_string())])
            .collect(),
        0,
        36,
    )
    .await
}

pub async fn get_cover_art_ids_by_mix_queries_request(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    dart_signal: DartSignal<GetCoverArtIdsByMixQueriesRequest>,
) -> Result<()> {
    let requests = dart_signal.message.requests;

    let files_futures = requests.clone().into_iter().map(|x| {
        let main_db = Arc::clone(&main_db);
        let recommend_db = Arc::clone(&recommend_db);
        async move {
            let query = query_cover_arts(main_db, recommend_db, x.queries).await;

            match query {
                Ok(files) => {
                    let mut cover_art_ids: Vec<i32> =
                        files.iter().filter_map(|file| file.cover_art_id).collect();

                    cover_art_ids.dedup();

                    GetCoverArtIdsByMixQueriesResponseUnit {
                        id: x.id,
                        cover_art_ids,
                    }
                }
                Err(_) => GetCoverArtIdsByMixQueriesResponseUnit {
                    id: x.id,
                    cover_art_ids: Vec::new(),
                },
            }
        }
    });

    GetCoverArtIdsByMixQueriesResponse {
        result: join_all(files_futures).await,
    }
    .send_signal_to_dart();

    Ok(())
}
