use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::future::join_all;
use rinf::DartSignal;

use database::actions::utils::create_count_by_first_letter;
use database::connection::{MainDbConnection, RecommendationDbConnection};
use database::entities::{albums, artists, mixes, playlists};

use crate::messages::collection::{
    Collection, CollectionGroup, CollectionGroupSummary, CollectionGroupSummaryResponse,
    CollectionGroups, FetchCollectionByIdsRequest, FetchCollectionByIdsResponse,
    FetchCollectionGroupSummaryRequest, FetchCollectionGroupsRequest,
    SearchCollectionSummaryRequest, SearchCollectionSummaryResponse,
};

use crate::utils::inject_cover_art_map;
use crate::MixQuery;

#[async_trait]
pub trait CollectionType: Send + Sync + 'static {
    fn collection_type() -> i32;
    fn query_operator() -> &'static str;
    async fn count_by_first_letter(main_db: &Arc<MainDbConnection>) -> Result<Vec<(String, i32)>>;
    async fn get_groups(
        main_db: &Arc<MainDbConnection>,
        group_titles: Vec<String>,
    ) -> Result<Vec<(String, Vec<(Self, HashSet<i32>)>)>>
    where
        Self: std::marker::Sized;
    async fn get_by_ids(main_db: &Arc<MainDbConnection>, ids: &[i32]) -> Result<Vec<Self>>
    where
        Self: std::marker::Sized;
    async fn list(main_db: &Arc<MainDbConnection>, limit: u64) -> Result<Vec<Self>>
    where
        Self: std::marker::Sized;

    fn id(&self) -> i32;
    fn name(&self) -> &str;
}

macro_rules! impl_collection_type {
    ($model:ty, $entity:ty, $collection_type:expr, $type_name:expr, $query_operator:expr, $get_groups:path, $get_by_ids:path, $list:path) => {
        #[async_trait]
        impl CollectionType for $model {
            fn collection_type() -> i32 {
                $collection_type
            }
            fn query_operator() -> &'static str {
                $query_operator
            }
            async fn count_by_first_letter(
                main_db: &Arc<MainDbConnection>,
            ) -> Result<Vec<(String, i32)>> {
                create_count_by_first_letter::<$entity>()(main_db)
                    .await
                    .with_context(|| "Failed to count collection by first letter")
            }
            async fn get_groups(
                main_db: &Arc<MainDbConnection>,
                group_titles: Vec<String>,
            ) -> Result<Vec<(String, Vec<(Self, HashSet<i32>)>)>> {
                $get_groups(main_db, group_titles)
                    .await
                    .with_context(|| "Failed to get collection groups")
            }
            async fn get_by_ids(main_db: &Arc<MainDbConnection>, ids: &[i32]) -> Result<Vec<Self>> {
                $get_by_ids(main_db, ids)
                    .await
                    .with_context(|| "Failed to get collection item by ids")
            }
            async fn list(main_db: &Arc<MainDbConnection>, limit: u64) -> Result<Vec<Self>> {
                $list(main_db, limit)
                    .await
                    .with_context(|| "Failed to get collection list")
            }

            fn id(&self) -> i32 {
                self.id
            }

            fn name(&self) -> &str {
                &self.name
            }
        }
    };
}

impl_collection_type!(
    albums::Model,
    database::entities::prelude::Albums,
    0,
    "album",
    "lib::album",
    database::actions::albums::get_albums_groups,
    database::actions::albums::get_albums_by_ids,
    database::actions::albums::list_albums
);
impl_collection_type!(
    artists::Model,
    database::entities::prelude::Artists,
    1,
    "artist",
    "lib::artist",
    database::actions::artists::get_artists_groups,
    database::actions::artists::get_artists_by_ids,
    database::actions::artists::list_artists
);
impl_collection_type!(
    playlists::Model,
    database::entities::prelude::Playlists,
    2,
    "playlist",
    "lib::playlist",
    database::actions::playlists::get_playlists_groups,
    database::actions::playlists::get_playlists_by_ids,
    database::actions::playlists::list_playlists
);
impl_collection_type!(
    mixes::Model,
    database::entities::prelude::Mixes,
    3,
    "mix",
    "lib::mix",
    database::actions::mixes::get_mixes_groups,
    database::actions::mixes::get_mixes_by_ids,
    database::actions::mixes::list_mixes
);

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

async fn handle_collection_action<T: CollectionType + std::clone::Clone>(
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
                collection_type: T::collection_type(),
                groups: collection_groups,
            }
            .send_signal_to_dart();
        }
        CollectionAction::FetchGroups => {
            let entry = T::get_groups(main_db, params.group_titles.unwrap()).await?;
            CollectionGroups {
                groups: entry
                    .into_iter()
                    .map(|x| CollectionGroup {
                        group_title: x.0,
                        collections: x
                            .1
                            .into_iter()
                            .map(|x| {
                                Collection::from_model(
                                    &x.0,
                                    T::collection_type(),
                                    T::query_operator(),
                                )
                            })
                            .collect(),
                    })
                    .collect(),
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
                    T::collection_type(),
                    T::query_operator(),
                    params.bake_cover_arts,
                )
            }))
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

            FetchCollectionByIdsResponse {
                collection_type: T::collection_type(),
                result: collections,
            }
            .send_signal_to_dart();
        }
        CollectionAction::Search => {
            let items = T::list(main_db, params.n.unwrap().into()).await?;
            SearchCollectionSummaryResponse {
                collection_type: T::collection_type(),
                result: items
                    .into_iter()
                    .map(|x| Collection::from_model(&x, T::collection_type(), T::query_operator()))
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
        CollectionAction::FetchGroups => {
            let entry =
                database::actions::mixes::get_mixes_groups(main_db, params.group_titles.unwrap())
                    .await?;
            let results = fetch_mix_queries(main_db, &entry).await?;
            CollectionGroups {
                groups: create_collection_groups(entry, results),
            }
            .send_signal_to_dart();
        }
        CollectionAction::FetchById => {
            let items =
                database::actions::mixes::get_mixes_by_ids(main_db, &params.ids.unwrap()).await?;
            let results = fetch_mix_queries_for_items(main_db, &items).await?;
            let collections = join_all(results.into_iter().map(|(mix, queries)| {
                let queries = queries.clone();
                let mix = mix.clone();

                Collection::from_mix_bakeable::<mixes::Model>(
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
            let items =
                database::actions::mixes::list_mixes(main_db, params.n.unwrap().into()).await?;
            let results = fetch_mix_queries_for_items(main_db, &items).await?;
            SearchCollectionSummaryResponse {
                collection_type: 3,
                result: create_collections(results),
            }
            .send_signal_to_dart();
        }
        _ => {
            handle_collection_action::<mixes::Model>(main_db, recommend_db, action, params).await?
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

type CollectionGroupsEntry = Vec<(String, Vec<(mixes::Model, HashSet<i32>)>)>;

fn create_collection_groups(
    entry: CollectionGroupsEntry,
    results: Vec<(String, mixes::Model, Vec<MixQuery>)>,
) -> Vec<CollectionGroup> {
    entry
        .into_iter()
        .map(|(group_title, _)| {
            let collections = results
                .iter()
                .filter_map(|(gt, mix, queries)| {
                    if gt == &group_title {
                        Some(Collection::from_mix(mix, queries))
                    } else {
                        None
                    }
                })
                .collect();

            CollectionGroup {
                group_title,
                collections,
            }
        })
        .collect()
}

fn create_collections(results: Vec<(mixes::Model, Vec<MixQuery>)>) -> Vec<Collection> {
    results
        .into_iter()
        .map(|(mix, queries)| Collection::from_mix(&mix, &queries))
        .collect()
}

impl Collection {
    pub fn from_model<T: CollectionType>(
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
        }
    }

    pub async fn from_model_bakeable<T: CollectionType>(
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
        }
    }

    pub async fn from_mix_bakeable<T: CollectionType>(
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
