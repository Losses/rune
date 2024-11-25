use std::sync::Arc;

use anyhow::Result;
use futures::future::join_all;
use rinf::DartSignal;

use database::actions::cover_art::get_cover_art_id_by_track_id;
use database::actions::cover_art::get_primary_color_by_cover_art_id;
use database::actions::mixes::query_mix_media_files;
use database::connection::MainDbConnection;
use database::connection::RecommendationDbConnection;
use database::entities::media_files;
use tokio::task;

use crate::messages::*;

pub async fn query_cover_arts(
    main_db: &MainDbConnection,
    recommend_db: Arc<RecommendationDbConnection>,
    queries: Vec<MixQuery>,
    n: Option<i32>,
) -> Result<Vec<media_files::Model>> {
    query_mix_media_files(
        main_db,
        &recommend_db,
        queries
            .into_iter()
            .map(|q| (q.operator, q.parameter))
            .chain([("filter::with_cover_art".to_string(), "true".to_string())])
            .collect(),
        0,
        match n {
            Some(n) => {
                if n == 0 {
                    18
                } else {
                    n as usize
                }
            }
            None => 18,
        },
    )
    .await
}

pub async fn get_cover_art_ids_by_mix_queries_request(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    dart_signal: DartSignal<GetCoverArtIdsByMixQueriesRequest>,
) -> Result<()> {
    let request = dart_signal.message;
    let queries = request.requests;

    let files_futures = queries.clone().into_iter().map(|x| {
        let main_db = Arc::clone(&main_db);
        let recommend_db = Arc::clone(&recommend_db);
        async move {
            let query = query_cover_arts(&main_db, recommend_db, x.queries, Some(request.n)).await;

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

pub async fn get_primary_color_by_track_id_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<GetPrimaryColorByTrackIdRequest>,
) -> Result<()> {
    let track_id = dart_signal.message.id;
    let main_db = main_db.clone();

    task::spawn_blocking(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            if let Some(id) = get_cover_art_id_by_track_id(&main_db, track_id)
                .await
                .ok()
                .flatten()
            {
                let primary_color = get_primary_color_by_cover_art_id(&main_db, id).await.ok();

                GetPrimaryColorByTrackIdResponse {
                    id: track_id,
                    primary_color,
                }
                .send_signal_to_dart();
            } else {
                GetPrimaryColorByTrackIdResponse {
                    id: track_id,
                    primary_color: None,
                }
                .send_signal_to_dart();
            }
        })
    });

    Ok(())
}
