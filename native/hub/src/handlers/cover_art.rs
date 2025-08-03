use std::sync::Arc;

use anyhow::Result;
use futures::future::join_all;

use database::connection::MainDbConnection;
use database::connection::RecommendationDbConnection;
use database::playing_item::dispatcher::PlayingItemActionDispatcher;
use tokio::task;

use crate::Session;
use crate::utils::GlobalParams;
use crate::utils::ParamsExtractor;
use crate::utils::query_cover_arts;
use crate::{Signal, messages::*};

impl ParamsExtractor for GetCoverArtIdsByMixQueriesRequest {
    type Params = (Arc<MainDbConnection>, Arc<RecommendationDbConnection>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.recommend_db),
        )
    }
}

impl Signal for GetCoverArtIdsByMixQueriesRequest {
    type Params = (Arc<MainDbConnection>, Arc<RecommendationDbConnection>);
    type Response = GetCoverArtIdsByMixQueriesResponse;
    async fn handle(
        &self,
        (main_db, recommend_db): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let queries = &request.requests;

        let files_futures = queries.clone().into_iter().map(|x| {
            let main_db = Arc::clone(&main_db);
            let recommend_db = Arc::clone(&recommend_db);
            async move {
                let query =
                    query_cover_arts(&main_db, recommend_db, x.queries.clone(), Some(request.n))
                        .await;

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

        Ok(Some(GetCoverArtIdsByMixQueriesResponse {
            result: join_all(files_futures).await,
        }))
    }
}

impl ParamsExtractor for GetPrimaryColorByTrackIdRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for GetPrimaryColorByTrackIdRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = GetPrimaryColorByTrackIdResponse;

    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let item = dart_signal.item.clone();
        let main_db = main_db.clone();

        if let Some(item) = item {
            let response = task::spawn_blocking(move || {
                let runtime = tokio::runtime::Runtime::new().unwrap();
                runtime.block_on(async move {
                    let dispatcher = PlayingItemActionDispatcher::new();
                    let primary_color = dispatcher
                        .get_cover_art_primary_color(&main_db, &item.clone().into())
                        .await;

                    match primary_color {
                        Some(x) => GetPrimaryColorByTrackIdResponse {
                            item,
                            primary_color: Some(x),
                        },
                        None => GetPrimaryColorByTrackIdResponse {
                            item,
                            primary_color: None,
                        },
                    }
                })
            })
            .await?;

            Ok(Some(response))
        } else {
            Ok(None)
        }
    }
}
