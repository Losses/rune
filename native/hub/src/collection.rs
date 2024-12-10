use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use futures::future::join_all;
use rinf::DartSignal;

use database::actions::collection::{CollectionQuery, CollectionQueryListMode, UnifiedCollection};
use database::connection::{MainDbConnection, RecommendationDbConnection};
use database::entities::{albums, artists, mix_queries, mixes, playlists};

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
            let mut collection_groups: Vec<_> = entry
                .into_iter()
                .map(|x| CollectionGroupSummary {
                    group_title: x.0,
                    count: x.1,
                })
                .collect();

            // Partition the collection_groups to separate the special entry
            let (mut special, mut others): (Vec<_>, Vec<_>) = collection_groups
                .into_iter()
                .partition(|x| x.group_title == "\u{200B}Rune");

            // Combine special and others, with special at the front
            special.append(&mut others);
            collection_groups = special;

            CollectionGroupSummaryResponse {
                collection_type: T::collection_type().into(),
                groups: collection_groups,
            }
            .send_signal_to_dart();
        }

        CollectionAction::FetchGroups => {
            let entry = T::get_groups(
                main_db,
                params
                    .group_titles
                    .ok_or_else(|| anyhow::anyhow!("Group title is None"))?,
            )
            .await?;
            let collection_groups: Vec<CollectionGroup> =
                join_all(entry.into_iter().map(|x| async {
                    let collections_result: Result<Vec<Collection>> =
                        join_all(x.1.into_iter().map(|x| {
                            let main_db = Arc::clone(main_db);
                            let recommend_db = Arc::clone(recommend_db);

                            async move {
                                Collection::from_model_bakeable(
                                    &main_db,
                                    recommend_db,
                                    x.0.clone(),
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
            let items = T::get_by_ids(
                main_db,
                &params
                    .ids
                    .ok_or_else(|| anyhow::anyhow!("ID paraneter is None"))?,
            )
            .await?;
            let futures = items.into_iter().map(|item| async move {
                Collection::from_model_bakeable(
                    main_db,
                    Arc::clone(recommend_db),
                    item.clone(),
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
            let items = T::list(
                main_db,
                params
                    .n
                    .ok_or_else(|| anyhow::anyhow!("Parameter N is None"))?
                    .into(),
                CollectionQueryListMode::Forward,
            )
            .await?;
            let futures = items
                .into_iter()
                .map(|x| async move { Collection::from_model(main_db, &x).await });
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

impl From<mix_queries::Model> for MixQuery {
    fn from(model: mix_queries::Model) -> Self {
        MixQuery {
            operator: model.operator,
            parameter: model.parameter,
        }
    }
}

impl Collection {
    pub async fn from_model<T: CollectionQuery>(
        main_db: &MainDbConnection,
        model: &T,
    ) -> Result<Self> {
        let collection = Collection {
            id: model.id(),
            name: model.name().to_owned(),
            queries: T::query_builder(main_db, model.id())
                .await?
                .into_iter()
                .map(|x| MixQuery {
                    operator: x.0,
                    parameter: x.1,
                })
                .collect(),
            collection_type: T::collection_type().into(),
            cover_art_map: HashMap::new(),
            readonly: model.readonly(),
        };

        Ok(collection)
    }

    pub fn from_unified_collection(x: UnifiedCollection) -> Self {
        Collection {
            id: x.id,
            name: x.name,
            queries: x
                .queries
                .into_iter()
                .map(|x| MixQuery {
                    operator: x.0,
                    parameter: x.1,
                })
                .collect(),
            collection_type: x.collection_type.into(),
            cover_art_map: HashMap::new(),
            readonly: x.readonly,
        }
    }

    pub async fn from_model_bakeable<T: CollectionQuery>(
        main_db: &MainDbConnection,
        recommend_db: Arc<RecommendationDbConnection>,
        model: T,
        bake_cover_arts: bool,
    ) -> Result<Self> {
        let mut collection = Collection::from_model(main_db, &model).await?;

        if bake_cover_arts {
            collection = inject_cover_art_map(main_db, recommend_db, collection, None).await?;
        }

        Ok(collection)
    }

    pub async fn from_unified_collection_bakeable(
        main_db: &MainDbConnection,
        recommend_db: Arc<RecommendationDbConnection>,
        x: UnifiedCollection,
        bake_cover_arts: bool,
    ) -> Result<Self> {
        let mut collection = Collection::from_unified_collection(x);

        if bake_cover_arts {
            collection = inject_cover_art_map(main_db, recommend_db, collection, None).await?;
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
            n: Some(dart_signal.message.n.try_into()?),
            ..Default::default()
        },
    )
    .await
}
