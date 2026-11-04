use crate::node::NodeContext;
use database::entities::{media_cover_art, media_cover_art::Column};
use std::path::Path;
use std::sync::Arc;
use tokio::runtime::Runtime;

use anyhow::Result;
use futures::future::join_all;

use tokio::task;

use ::database::playing_item::dispatcher::PlayingItemActionDispatcher;

use crate::{
    Session, Signal,
    library_pool::LibraryHandle,
    messages::*,
    utils::{GlobalParams, ParamsExtractor, query_cover_arts, require_library},
};

impl ParamsExtractor for GetCoverArtIdsByMixQueriesRequest {
    type Params = (Option<LibraryHandle>, Arc<NodeContext>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.library.clone(), all_params.node.clone())
    }
}

impl Signal for GetCoverArtIdsByMixQueriesRequest {
    type Params = (Option<LibraryHandle>, Arc<NodeContext>);
    type Response = GetCoverArtIdsByMixQueriesResponse;
    async fn handle(
        &self,
        (library, node): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let library = require_library(library)?;
        let main_db = Arc::clone(&library.main_db);
        let recommend_db = Arc::clone(&library.recommend_db);
        let queries = &request.requests;
        let queue = Arc::new(
            node.session_queue_for_library(library.descriptor.library_id)
                .await?,
        );

        let files_futures = queries.clone().into_iter().map(|x| {
            let main_db = Arc::clone(&main_db);
            let recommend_db = Arc::clone(&recommend_db);
            let queue = queue.clone();
            async move {
                let query = query_cover_arts(
                    &main_db,
                    recommend_db,
                    x.queries.clone(),
                    Some(request.n),
                    &queue,
                )
                .await;

                match query {
                    Ok(files) => {
                        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
                        let uuids: std::collections::BTreeSet<_> = files
                            .iter()
                            .filter_map(|file| file.cover_art_uuid.as_deref())
                            .collect();
                        let mut cover_art_ids: Vec<i32> = media_cover_art::Entity::find()
                            .select_only()
                            .column(Column::Id)
                            .filter(Column::HlcUuid.is_in(uuids))
                            .into_tuple()
                            .all(&*main_db)
                            .await
                            .unwrap_or_default();

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
    type Params = Option<LibraryHandle>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.library.clone()
    }
}

impl Signal for GetPrimaryColorByTrackIdRequest {
    type Params = Option<LibraryHandle>;
    type Response = GetPrimaryColorByTrackIdResponse;

    async fn handle(
        &self,
        library: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let item = dart_signal.item.clone();
        let library = require_library(library)?;

        if let Some(item) = item {
            let response = task::spawn_blocking(move || {
                let runtime = Runtime::new().unwrap();
                runtime.block_on(async move {
                    let dispatcher = PlayingItemActionDispatcher::new();
                    let primary_color = dispatcher
                        .get_cover_art_primary_color(
                            &library.fsio,
                            Path::new(&library.media_root),
                            &library.main_db,
                            &item.clone().into(),
                        )
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
