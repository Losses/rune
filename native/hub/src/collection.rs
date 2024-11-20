use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use futures::future::join_all;
use rinf::DartSignal;

use database::actions::collection::CollectionQuery;
use database::connection::{MainDbConnection, RecommendationDbConnection};
use database::entities::{albums, artists, mixes, playlists};

use crate::messages::*;

use crate::utils::inject_cover_art_map;

#[derive(Debug)]
pub enum CollectionAction {
    FetchGroupSummary,
    FetchGroups,
    FetchById,
    Search,
}

#[derive(Default)]
pub struct CollectionActionParams {
    group_titles: Option<Vec<String>>,
    ids: Option<Vec<i32>>,
    n: Option<u32>,
    bake_cover_arts: bool,
}

async fn handle_collection_action<T: CollectionQuery + std::clone::Clone>(
    main_db: &Arc<MainDbConnection>,
    recommend_db: &Arc<RecommendationDbConnection>,
    action: CollectionAction,
    params: CollectionActionParams,
) -> Result<()> {
    match action {
        CollectionAction::FetchGroupSummary => {
            let entry = T::count_by_first_letter(main_db).await?;
            let collection_groups = entry
                .into_iter()
                .map(|x| CollectionGroupSummary {
                    group_title: x.0,
                    count: x.1,
                })
                .collect();

            CollectionGroupSummaryResponse {
                collection_type: T::collection_type().into(),
                groups: collection_groups,
            }
            .send_signal_to_dart();
        }
        CollectionAction::FetchGroups => {
            let entry = T::get_groups(main_db, params.group_titles.unwrap()).await?;
            let collection_groups: Vec<CollectionGroup> =
                join_all(entry.into_iter().map(|x| async {
                    let collections_result: Result<Vec<Collection>> =
                        join_all(x.1.into_iter().map(|x| {
                            let main_db = Arc::clone(main_db);
                            let recommend_db = Arc::clone(recommend_db);

                            async move {
                                let query = T::query_builder(&main_db, x.0.id()).await?;
                                Collection::from_model_bakeable(
                                    main_db,
                                    recommend_db,
                                    x.0.clone(),
                                    T::collection_type().into(),
                                    query,
                                    params.bake_cover_arts,
                                )
                                .await
                            }
                        }))
                        .await
                        .into_iter()
                        .collect();

                    collections_result.map(|collections| CollectionGroup {
                        group_title: x.0,
                        collections,
                    })
                }))
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?;

            CollectionGroups {
                groups: collection_groups,
            }
            .send_signal_to_dart();
        }
        CollectionAction::FetchById => {
            let items = T::get_by_ids(main_db, &params.ids.unwrap()).await?;
            let futures = items.into_iter().map(|item| async move {
                Collection::from_model_bakeable(
                    Arc::clone(main_db),
                    Arc::clone(recommend_db),
                    item.clone(),
                    T::collection_type().into(),
                    T::query_builder(main_db, item.id()).await?,
                    params.bake_cover_arts,
                )
                .await
            });

            let collections: Vec<_> = join_all(futures)
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?; // Use `collect` to handle errors

            FetchCollectionByIdsResponse {
                collection_type: T::collection_type().into(),
                result: collections,
            }
            .send_signal_to_dart();
        }
        CollectionAction::Search => {
            let items = T::list(main_db, params.n.unwrap().into()).await?;
            let futures = items.into_iter().map(|x| async move {
                match T::query_builder(main_db, x.id()).await {
                    Ok(query_result) => Ok(Collection::from_model(
                        &x,
                        T::collection_type().into(),
                        query_result,
                    )),
                    Err(e) => Err(e),
                }
            });
            let results = join_all(futures).await;
            let results: Result<Vec<_>, _> = results.into_iter().collect();
            let results = results?;

            SearchCollectionSummaryResponse {
                collection_type: T::collection_type().into(),
                result: results,
            }
            .send_signal_to_dart();
        }
    }

    Ok(())
}

impl From<database::entities::mix_queries::Model> for MixQuery {
    fn from(model: database::entities::mix_queries::Model) -> Self {
        MixQuery {
            operator: model.operator,
            parameter: model.parameter,
        }
    }
}

impl Collection {
    pub fn from_model<T: CollectionQuery>(
        model: &T,
        collection_type: i32,
        query: Vec<(String, String)>,
    ) -> Self {
        Collection {
            id: model.id(),
            name: model.name().to_owned(),
            queries: query
                .into_iter()
                .map(|x| MixQuery {
                    operator: x.0,
                    parameter: x.1,
                })
                .collect(),
            collection_type,
            cover_art_map: HashMap::new(),
            readonly: false,
        }
    }

    pub async fn from_model_bakeable<T: CollectionQuery>(
        main_db: Arc<MainDbConnection>,
        recommend_db: Arc<RecommendationDbConnection>,
        model: T,
        collection_type: i32,
        query: Vec<(String, String)>,
        bake_cover_arts: bool,
    ) -> Result<Self> {
        let mut collection = Collection::from_model(&model, collection_type, query);

        if bake_cover_arts {
            collection = inject_cover_art_map(main_db, recommend_db, collection).await?;
        }

        Ok(collection)
    }
}

pub async fn handle_collection_request(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    collection_type: i32,
    action: CollectionAction,
    params: CollectionActionParams,
) -> Result<()> {
    match collection_type {
        0 => {
            handle_collection_action::<albums::Model>(&main_db, &recommend_db, action, params)
                .await?
        }
        1 => {
            handle_collection_action::<artists::Model>(&main_db, &recommend_db, action, params)
                .await?
        }
        2 => {
            handle_collection_action::<playlists::Model>(&main_db, &recommend_db, action, params)
                .await?
        }
        3 => {
            handle_collection_action::<mixes::Model>(&main_db, &recommend_db, action, params)
                .await?
        }
        _ => return Err(anyhow::anyhow!("Invalid collection type")),
    }

    Ok(())
}

pub async fn fetch_collection_group_summary_request(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    dart_signal: DartSignal<FetchCollectionGroupSummaryRequest>,
) -> Result<()> {
    handle_collection_request(
        main_db,
        recommend_db,
        dart_signal.message.collection_type,
        CollectionAction::FetchGroupSummary,
        CollectionActionParams::default(),
    )
    .await
}

pub async fn fetch_collection_groups_request(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    dart_signal: DartSignal<FetchCollectionGroupsRequest>,
) -> Result<()> {
    handle_collection_request(
        main_db,
        recommend_db,
        dart_signal.message.collection_type,
        CollectionAction::FetchGroups,
        CollectionActionParams {
            group_titles: Some(dart_signal.message.group_titles),
            bake_cover_arts: dart_signal.message.bake_cover_arts,
            ..Default::default()
        },
    )
    .await
}

pub async fn fetch_collection_by_ids_request(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    dart_signal: DartSignal<FetchCollectionByIdsRequest>,
) -> Result<()> {
    handle_collection_request(
        main_db,
        recommend_db,
        dart_signal.message.collection_type,
        CollectionAction::FetchById,
        CollectionActionParams {
            ids: Some(dart_signal.message.ids),
            ..Default::default()
        },
    )
    .await
}

pub async fn search_collection_summary_request(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    dart_signal: DartSignal<SearchCollectionSummaryRequest>,
) -> Result<()> {
    handle_collection_request(
        main_db,
        recommend_db,
        dart_signal.message.collection_type,
        CollectionAction::Search,
        CollectionActionParams {
            n: Some(dart_signal.message.n.try_into().unwrap()),
            ..Default::default()
        },
    )
    .await
}
