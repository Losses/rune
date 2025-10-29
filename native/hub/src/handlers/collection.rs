use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use futures::future::join_all;

use ::database::{
    actions::collection::{
        CollectionQuery, CollectionQueryListMode, CollectionQueryType, UnifiedCollection,
    },
    connection::{MainDbConnection, RecommendationDbConnection},
    entities::{albums, artists, genres, mix_queries, mixes, playlists},
};
use ::fsio::FsIo;

use crate::utils::{GlobalParams, ParamsExtractor, RunningMode, inject_cover_art_map};
use crate::{Session, Signal, messages::*};

impl From<CollectionQueryType> for CollectionType {
    fn from(value: CollectionQueryType) -> Self {
        match value {
            CollectionQueryType::Album => CollectionType::Album,
            CollectionQueryType::Artist => CollectionType::Artist,
            CollectionQueryType::Playlist => CollectionType::Playlist,
            CollectionQueryType::Mix => CollectionType::Mix,
            CollectionQueryType::Genre => CollectionType::Genre,
            CollectionQueryType::Track => CollectionType::Track,
            CollectionQueryType::Directory => CollectionType::Directory,
        }
    }
}

#[derive(Default)]
pub struct CollectionActionParams {
    group_titles: Option<Vec<String>>,
    ids: Option<Vec<i32>>,
    n: Option<u32>,
    bake_cover_arts: bool,
}

async fn handle_fetch_group_summary<T: CollectionQuery>(
    main_db: &Arc<MainDbConnection>,
) -> Result<Option<CollectionGroupSummaryResponse>> {
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

    Ok(Some(CollectionGroupSummaryResponse {
        collection_type: T::collection_type().into(),
        groups: collection_groups,
    }))
}

async fn handle_fetch_groups<T: CollectionQuery + std::clone::Clone>(
    fsio: &Arc<FsIo>,
    main_db: &Arc<MainDbConnection>,
    recommend_db: &Arc<RecommendationDbConnection>,
    running_mode: &RunningMode,
    remote_host: &Option<String>,
    params: CollectionActionParams,
) -> Result<Option<FetchCollectionGroupsResponse>> {
    let entry = T::get_groups(
        main_db,
        params
            .group_titles
            .ok_or_else(|| anyhow::anyhow!("Group title is None"))?,
    )
    .await?;
    let collection_groups: Vec<CollectionGroup> = join_all(entry.into_iter().map(|x| async {
        let collections_result: Result<Vec<Collection>> = join_all(x.1.into_iter().map(|x| {
            let main_db = Arc::clone(main_db);
            let recommend_db = Arc::clone(recommend_db);

            async move {
                Collection::from_model_bakeable(
                    fsio,
                    &main_db,
                    recommend_db,
                    running_mode,
                    remote_host,
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

    Ok(Some(FetchCollectionGroupsResponse {
        groups: collection_groups,
    }))
}

async fn handle_fetch_by_id<T: CollectionQuery + std::clone::Clone>(
    fsio: &Arc<FsIo>,
    main_db: &Arc<MainDbConnection>,
    recommend_db: &Arc<RecommendationDbConnection>,
    running_mode: &RunningMode,
    remote_host: &Option<String>,
    params: CollectionActionParams,
) -> Result<Option<FetchCollectionByIdsResponse>> {
    let items = T::get_by_ids(
        main_db,
        &params
            .ids
            .ok_or_else(|| anyhow::anyhow!("ID parameter is None"))?,
    )
    .await?;
    let futures = items.into_iter().map(|item| async move {
        Collection::from_model_bakeable(
            fsio,
            main_db,
            Arc::clone(recommend_db),
            running_mode,
            remote_host,
            item.clone(),
            params.bake_cover_arts,
        )
        .await
    });

    let collections: Vec<_> = join_all(futures)
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Some(FetchCollectionByIdsResponse {
        collection_type: T::collection_type().into(),
        result: collections,
    }))
}

async fn handle_search<T: CollectionQuery>(
    main_db: &Arc<MainDbConnection>,
    params: CollectionActionParams,
) -> Result<Option<SearchCollectionSummaryResponse>> {
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

    Ok(Some(SearchCollectionSummaryResponse {
        collection_type: T::collection_type().into(),
        result: results,
    }))
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
        fsio: &FsIo,
        main_db: &MainDbConnection,
        recommend_db: Arc<RecommendationDbConnection>,
        running_mode: &RunningMode,
        remote_host: &Option<String>,
        model: T,
        bake_cover_arts: bool,
    ) -> Result<Self> {
        let mut collection = Collection::from_model(main_db, &model).await?;

        if bake_cover_arts {
            collection = inject_cover_art_map(
                fsio,
                main_db,
                recommend_db,
                collection,
                None,
                running_mode,
                remote_host,
            )
            .await?;
        }

        Ok(collection)
    }

    pub async fn from_unified_collection_bakeable(
        fsio: &FsIo,
        main_db: &MainDbConnection,
        recommend_db: Arc<RecommendationDbConnection>,
        running_mode: &RunningMode,
        remote_host: &Option<String>,
        x: UnifiedCollection,
        bake_cover_arts: bool,
    ) -> Result<Self> {
        let mut collection = Collection::from_unified_collection(x);

        if bake_cover_arts {
            collection = inject_cover_art_map(
                fsio,
                main_db,
                recommend_db,
                collection,
                None,
                running_mode,
                remote_host,
            )
            .await?;
        }

        Ok(collection)
    }
}

impl ParamsExtractor for FetchCollectionGroupSummaryRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for FetchCollectionGroupSummaryRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = CollectionGroupSummaryResponse;

    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        match dart_signal.collection_type {
            CollectionType::Album => handle_fetch_group_summary::<albums::Model>(&main_db).await,
            CollectionType::Artist => handle_fetch_group_summary::<artists::Model>(&main_db).await,
            CollectionType::Playlist => {
                handle_fetch_group_summary::<playlists::Model>(&main_db).await
            }
            CollectionType::Mix => handle_fetch_group_summary::<mixes::Model>(&main_db).await,
            CollectionType::Genre => handle_fetch_group_summary::<genres::Model>(&main_db).await,
            _ => Err(anyhow::anyhow!("Invalid collection type")),
        }
    }
}

impl ParamsExtractor for FetchCollectionGroupsRequest {
    type Params = (
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
        RunningMode,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.fsio),
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.recommend_db),
            all_params.running_mode,
        )
    }
}

impl Signal for FetchCollectionGroupsRequest {
    type Params = (
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
        RunningMode,
    );
    type Response = FetchCollectionGroupsResponse;

    async fn handle(
        &self,
        (fsio, main_db, recommend_db, running_mode): Self::Params,
        session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let params = CollectionActionParams {
            group_titles: Some(dart_signal.group_titles.clone()),
            bake_cover_arts: dart_signal.bake_cover_arts,
            ..Default::default()
        };

        let remote_host = match session {
            Some(x) => Some(x.host),
            None => None,
        };

        match dart_signal.collection_type {
            CollectionType::Album => {
                handle_fetch_groups::<albums::Model>(
                    &fsio,
                    &main_db,
                    &recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                )
                .await
            }
            CollectionType::Artist => {
                handle_fetch_groups::<artists::Model>(
                    &fsio,
                    &main_db,
                    &recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                )
                .await
            }
            CollectionType::Playlist => {
                handle_fetch_groups::<playlists::Model>(
                    &fsio,
                    &main_db,
                    &recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                )
                .await
            }
            CollectionType::Mix => {
                handle_fetch_groups::<mixes::Model>(
                    &fsio,
                    &main_db,
                    &recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                )
                .await
            }
            CollectionType::Genre => {
                handle_fetch_groups::<genres::Model>(
                    &fsio,
                    &main_db,
                    &recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                )
                .await
            }
            _ => Err(anyhow::anyhow!("Invalid collection type")),
        }
    }
}

impl ParamsExtractor for FetchCollectionByIdsRequest {
    type Params = (
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
        RunningMode,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.fsio),
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.recommend_db),
            all_params.running_mode,
        )
    }
}

impl Signal for FetchCollectionByIdsRequest {
    type Params = (
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
        RunningMode,
    );
    type Response = FetchCollectionByIdsResponse;

    async fn handle(
        &self,
        (fsio, main_db, recommend_db, running_mode): Self::Params,
        session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let params = CollectionActionParams {
            ids: Some(dart_signal.ids.clone()),
            ..Default::default()
        };

        let remote_host = match session {
            Some(x) => Some(x.host),
            None => None,
        };

        match dart_signal.collection_type {
            CollectionType::Album => {
                handle_fetch_by_id::<albums::Model>(
                    &fsio,
                    &main_db,
                    &recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                )
                .await
            }
            CollectionType::Artist => {
                handle_fetch_by_id::<artists::Model>(
                    &fsio,
                    &main_db,
                    &recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                )
                .await
            }
            CollectionType::Playlist => {
                handle_fetch_by_id::<playlists::Model>(
                    &fsio,
                    &main_db,
                    &recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                )
                .await
            }
            CollectionType::Mix => {
                handle_fetch_by_id::<mixes::Model>(
                    &fsio,
                    &main_db,
                    &recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                )
                .await
            }
            CollectionType::Genre => {
                handle_fetch_by_id::<genres::Model>(
                    &fsio,
                    &main_db,
                    &recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                )
                .await
            }
            _ => Err(anyhow::anyhow!("Invalid collection type")),
        }
    }
}

impl ParamsExtractor for SearchCollectionSummaryRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for SearchCollectionSummaryRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = SearchCollectionSummaryResponse;

    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let params = CollectionActionParams {
            n: Some(dart_signal.n.try_into()?),
            ..Default::default()
        };

        match dart_signal.collection_type {
            Some(CollectionType::Album) => handle_search::<albums::Model>(&main_db, params).await,
            Some(CollectionType::Artist) => handle_search::<artists::Model>(&main_db, params).await,
            Some(CollectionType::Playlist) => {
                handle_search::<playlists::Model>(&main_db, params).await
            }
            Some(CollectionType::Mix) => handle_search::<mixes::Model>(&main_db, params).await,
            Some(CollectionType::Genre) => handle_search::<genres::Model>(&main_db, params).await,
            _ => {
                Err(anyhow::anyhow!(
                    "Invalid collection type: {:?}",
                    dart_signal.collection_type
                ))
            }
        }
    }
}
