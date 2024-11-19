use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::future::join_all;
use rinf::DartSignal;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};

use database::actions::collection::CollectionQuery;
use database::actions::utils::create_count_by_first_letter;
use database::connection::{MainDbConnection, RecommendationDbConnection};
use database::entities::prelude;
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
                                Collection::from_model_bakeable(
                                    main_db,
                                    recommend_db,
                                    x.0,
                                    T::collection_type().into(),
                                    T::query_operator(),
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
            let collections = join_all(items.into_iter().map(|item| {
                let item = item.clone();

                Collection::from_model_bakeable(
                    Arc::clone(main_db),
                    Arc::clone(recommend_db),
                    item,
                    T::collection_type().into(),
                    T::query_operator(),
                    params.bake_cover_arts,
                )
            }))
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

            FetchCollectionByIdsResponse {
                collection_type: T::collection_type().into(),
                result: collections,
            }
            .send_signal_to_dart();
        }
        CollectionAction::Search => {
            let items = T::list(main_db, params.n.unwrap().into()).await?;
            SearchCollectionSummaryResponse {
                collection_type: T::collection_type().into(),
                result: items
                    .into_iter()
                    .map(|x| {
                        Collection::from_model(&x, T::collection_type().into(), T::query_operator())
                    })
                    .collect(),
            }
            .send_signal_to_dart();
        }
    }

    Ok(())
}

async fn handle_mixes(
    main_db: &Arc<MainDbConnection>,
    recommend_db: &Arc<RecommendationDbConnection>,
    action: CollectionAction,
    params: CollectionActionParams,
) -> Result<()> {
    match action {
        CollectionAction::FetchGroupSummary => {
            let entry = create_count_by_first_letter::<prelude::Mixes>()(main_db)
                .await
                .with_context(|| "Failed to count collection by first letter")?;

            let collection_groups = entry
                .into_iter()
                .map(|x| CollectionGroupSummary {
                    group_title: x.0,
                    count: x.1,
                })
                .collect();

            CollectionGroupSummaryResponse {
                collection_type: 3,
                groups: collection_groups,
            }
            .send_signal_to_dart();
        }

        CollectionAction::FetchGroups => {
            let entry =
                database::actions::mixes::get_mixes_groups(main_db, params.group_titles.unwrap())
                    .await?;
            let results = fetch_mix_queries(main_db, &entry).await?;

            let groups = entry
                .into_iter()
                .map(|(group_title, _)| {
                    let collections_futures = results
                        .iter()
                        .filter_map(|(gt, mix, queries)| {
                            if gt == &group_title {
                                Some(Collection::from_mix_bakeable(
                                    Arc::clone(main_db),
                                    Arc::clone(recommend_db),
                                    mix.clone(),
                                    queries.to_vec(),
                                    params.bake_cover_arts,
                                ))
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>();

                    async move {
                        let collections = join_all(collections_futures)
                            .await
                            .into_iter()
                            .filter_map(Result::ok)
                            .collect();
                        CollectionGroup {
                            group_title,
                            collections,
                        }
                    }
                })
                .collect::<Vec<_>>();

            let groups = join_all(groups).await;

            CollectionGroups { groups }.send_signal_to_dart();
        }
        CollectionAction::FetchById => {
            let items: Vec<mixes::Model> = mixes::Entity::find()
                .filter(mixes::Column::Id.is_in(params.ids.unwrap()))
                .all(main_db.as_ref())
                .await?;
            let results = fetch_mix_queries_for_items(main_db, &items).await?;
            let collections = join_all(results.into_iter().map(|(mix, queries)| {
                let queries = queries.clone();
                let mix = mix.clone();

                Collection::from_mix_bakeable(
                    Arc::clone(main_db),
                    Arc::clone(recommend_db),
                    mix,
                    queries,
                    params.bake_cover_arts,
                )
            }))
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

            FetchCollectionByIdsResponse {
                collection_type: 3,
                result: collections,
            }
            .send_signal_to_dart();
        }
        CollectionAction::Search => {
            let items: Vec<mixes::Model> = mixes::Entity::find()
                .limit(params.n.unwrap() as u64)
                .all(main_db.as_ref())
                .await?;
            let results = fetch_mix_queries_for_items(main_db, &items).await?;
            SearchCollectionSummaryResponse {
                collection_type: 3,
                result: create_collections(results),
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

type GroupEntry = Vec<(String, Vec<(mixes::Model, HashSet<i32>)>)>;

async fn fetch_mix_queries(
    main_db: &Arc<MainDbConnection>,
    entry: &GroupEntry,
) -> Result<Vec<(String, mixes::Model, Vec<MixQuery>)>> {
    let futures: Vec<_> = entry
        .iter()
        .flat_map(|(group_title, mixes)| {
            mixes.iter().map({
                let main_db = Arc::clone(main_db);
                move |mix| {
                    let main_db = Arc::clone(&main_db);
                    let group_title = group_title.clone();
                    async move {
                        let queries: Vec<MixQuery> =
                            database::actions::mixes::get_mix_queries_by_mix_id(&main_db, mix.0.id)
                                .await?
                                .into_iter()
                                .map(|x| x.into())
                                .collect();

                        Ok::<_, anyhow::Error>((group_title, mix.0.clone(), queries))
                    }
                }
            })
        })
        .collect();

    join_all(futures)
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
}

async fn fetch_mix_queries_for_items(
    main_db: &Arc<MainDbConnection>,
    items: &[mixes::Model],
) -> Result<Vec<(mixes::Model, Vec<MixQuery>)>> {
    let futures: Vec<_> = items
        .iter()
        .map({
            let main_db = Arc::clone(main_db);
            move |mix| {
                let main_db = Arc::clone(&main_db);
                async move {
                    let queries =
                        database::actions::mixes::get_mix_queries_by_mix_id(&main_db, mix.id)
                            .await?
                            .into_iter()
                            .map(|x| x.into())
                            .collect();
                    Ok::<_, anyhow::Error>((mix.clone(), queries))
                }
            }
        })
        .collect();

    join_all(futures)
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
}

fn create_collections(results: Vec<(mixes::Model, Vec<MixQuery>)>) -> Vec<Collection> {
    results
        .into_iter()
        .map(|(mix, queries)| Collection::from_mix(&mix, &queries))
        .collect()
}

impl Collection {
    pub fn from_model<T: CollectionQuery>(
        model: &T,
        collection_type: i32,
        query_operator: &str,
    ) -> Self {
        Collection {
            id: model.id(),
            name: model.name().to_owned(),
            queries: vec![MixQuery {
                operator: query_operator.to_string(),
                parameter: model.id().to_string(),
            }],
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
        query_operator: &str,
        bake_cover_arts: bool,
    ) -> Result<Self> {
        let mut collection = Collection::from_model(&model, collection_type, query_operator);

        if bake_cover_arts {
            collection = inject_cover_art_map(main_db, recommend_db, collection).await?;
        }

        Ok(collection)
    }

    pub fn from_mix(mix: &mixes::Model, queries: &[MixQuery]) -> Self {
        Collection {
            id: mix.id,
            name: mix.name.clone(),
            queries: queries.to_vec(),
            collection_type: 3,
            cover_art_map: HashMap::new(),
            readonly: mix.locked,
        }
    }

    pub async fn from_mix_bakeable(
        main_db: Arc<MainDbConnection>,
        recommend_db: Arc<RecommendationDbConnection>,
        mix: mixes::Model,
        queries: Vec<MixQuery>,
        bake_cover_arts: bool,
    ) -> Result<Self> {
        let mut collection = Collection::from_mix(&mix, &queries);

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
        3 => handle_mixes(&main_db, &recommend_db, action, params).await?,
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
