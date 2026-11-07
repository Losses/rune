use crate::node::NodeContext;
use crate::request_error::ExpectedRequestError;
use crate::utils::CollectionCoverOptions;
use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use futures::future::join_all;

use crate::library_pool::LibraryHandle;
use ::database::{
    actions::collection::{
        CollectionQuery, CollectionQueryListMode, CollectionQueryType, UnifiedCollection,
    },
    connection::{MainDbConnection, RecommendationDbConnection},
    entities::{albums, artists, genres, mix_queries, mixes, playlists},
};
use ::fsio::FsIo;

use crate::utils::{
    GlobalParams, ParamsExtractor, RunningMode, inject_cover_art_map, require_library,
};
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
    queue: Vec<i32>,
) -> Result<Option<FetchCollectionGroupsResponse>> {
    let entry = T::get_groups(
        main_db,
        params
            .group_titles
            .ok_or_else(|| ExpectedRequestError::invalid_input("Group title is None"))?,
    )
    .await?;
    let collection_groups: Vec<CollectionGroup> = join_all(entry.into_iter().map(|x| async {
        let collections_result: Result<Vec<Collection>> = join_all(x.1.into_iter().map(|x| {
            let main_db = Arc::clone(main_db);
            let recommend_db = Arc::clone(recommend_db);
            let queue = queue.clone();

            async move {
                Collection::from_model_bakeable(
                    fsio,
                    &main_db,
                    recommend_db,
                    x.0.clone(),
                    CollectionCoverOptions {
                        bake: params.bake_cover_arts,
                        running_mode,
                        remote_host,
                        playback_queue: &queue,
                    },
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
    queue: Vec<i32>,
) -> Result<Option<FetchCollectionByIdsResponse>> {
    let items = T::get_by_ids(
        main_db,
        &params
            .ids
            .ok_or_else(|| ExpectedRequestError::invalid_input("ID parameter is None"))?,
    )
    .await?;
    let futures = items.into_iter().map(|item| {
        let queue = queue.clone();
        async move {
            Collection::from_model_bakeable(
                fsio,
                main_db,
                Arc::clone(recommend_db),
                item.clone(),
                CollectionCoverOptions {
                    bake: params.bake_cover_arts,
                    running_mode,
                    remote_host,
                    playback_queue: &queue,
                },
            )
            .await
        }
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
            .ok_or_else(|| ExpectedRequestError::invalid_input("Parameter N is None"))?
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
        model: T,
        options: CollectionCoverOptions<'_>,
    ) -> Result<Self> {
        let mut collection = Collection::from_model(main_db, &model).await?;

        if options.bake {
            collection =
                inject_cover_art_map(fsio, main_db, recommend_db, collection, None, options)
                    .await?;
        }

        Ok(collection)
    }

    pub async fn from_unified_collection_bakeable(
        fsio: &FsIo,
        main_db: &MainDbConnection,
        recommend_db: Arc<RecommendationDbConnection>,
        x: UnifiedCollection,
        options: CollectionCoverOptions<'_>,
    ) -> Result<Self> {
        let mut collection = Collection::from_unified_collection(x);

        if options.bake {
            collection =
                inject_cover_art_map(fsio, main_db, recommend_db, collection, None, options)
                    .await?;
        }

        Ok(collection)
    }
}

impl ParamsExtractor for FetchCollectionGroupSummaryRequest {
    type Params = (Option<LibraryHandle>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.library.clone(),)
    }
}

impl Signal for FetchCollectionGroupSummaryRequest {
    type Params = (Option<LibraryHandle>,);
    type Response = CollectionGroupSummaryResponse;

    async fn handle(
        &self,
        (library,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        match dart_signal.collection_type {
            CollectionType::Album => {
                handle_fetch_group_summary::<albums::Model>(&library.main_db).await
            }
            CollectionType::Artist => {
                handle_fetch_group_summary::<artists::Model>(&library.main_db).await
            }
            CollectionType::Playlist => {
                handle_fetch_group_summary::<playlists::Model>(&library.main_db).await
            }
            CollectionType::Mix => {
                handle_fetch_group_summary::<mixes::Model>(&library.main_db).await
            }
            CollectionType::Genre => {
                handle_fetch_group_summary::<genres::Model>(&library.main_db).await
            }
            _ => Err(ExpectedRequestError::invalid_input("Invalid collection type").into()),
        }
    }
}

impl ParamsExtractor for FetchCollectionGroupsRequest {
    type Params = (Option<LibraryHandle>, RunningMode, Arc<NodeContext>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library.clone(),
            all_params.running_mode,
            all_params.node.clone(),
        )
    }
}

impl Signal for FetchCollectionGroupsRequest {
    type Params = (Option<LibraryHandle>, RunningMode, Arc<NodeContext>);
    type Response = FetchCollectionGroupsResponse;

    async fn handle(
        &self,
        (library, running_mode, node): Self::Params,
        session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let fsio = &library.fsio;
        let main_db = &library.main_db;
        let recommend_db = &library.recommend_db;
        let queue = node
            .session_queue_for_library(library.descriptor.library_id)
            .await?;
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
                    fsio,
                    main_db,
                    recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                    queue.clone(),
                )
                .await
            }
            CollectionType::Artist => {
                handle_fetch_groups::<artists::Model>(
                    fsio,
                    main_db,
                    recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                    queue.clone(),
                )
                .await
            }
            CollectionType::Playlist => {
                handle_fetch_groups::<playlists::Model>(
                    fsio,
                    main_db,
                    recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                    queue.clone(),
                )
                .await
            }
            CollectionType::Mix => {
                handle_fetch_groups::<mixes::Model>(
                    fsio,
                    main_db,
                    recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                    queue.clone(),
                )
                .await
            }
            CollectionType::Genre => {
                handle_fetch_groups::<genres::Model>(
                    fsio,
                    main_db,
                    recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                    queue.clone(),
                )
                .await
            }
            _ => Err(ExpectedRequestError::invalid_input("Invalid collection type").into()),
        }
    }
}

impl ParamsExtractor for FetchCollectionByIdsRequest {
    type Params = (Option<LibraryHandle>, RunningMode, Arc<NodeContext>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library.clone(),
            all_params.running_mode,
            all_params.node.clone(),
        )
    }
}

impl Signal for FetchCollectionByIdsRequest {
    type Params = (Option<LibraryHandle>, RunningMode, Arc<NodeContext>);
    type Response = FetchCollectionByIdsResponse;

    async fn handle(
        &self,
        (library, running_mode, node): Self::Params,
        session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let fsio = &library.fsio;
        let main_db = &library.main_db;
        let recommend_db = &library.recommend_db;
        let queue = node
            .session_queue_for_library(library.descriptor.library_id)
            .await?;
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
                    fsio,
                    main_db,
                    recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                    queue.clone(),
                )
                .await
            }
            CollectionType::Artist => {
                handle_fetch_by_id::<artists::Model>(
                    fsio,
                    main_db,
                    recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                    queue.clone(),
                )
                .await
            }
            CollectionType::Playlist => {
                handle_fetch_by_id::<playlists::Model>(
                    fsio,
                    main_db,
                    recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                    queue.clone(),
                )
                .await
            }
            CollectionType::Mix => {
                handle_fetch_by_id::<mixes::Model>(
                    fsio,
                    main_db,
                    recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                    queue.clone(),
                )
                .await
            }
            CollectionType::Genre => {
                handle_fetch_by_id::<genres::Model>(
                    fsio,
                    main_db,
                    recommend_db,
                    &running_mode,
                    &remote_host,
                    params,
                    queue.clone(),
                )
                .await
            }
            _ => Err(ExpectedRequestError::invalid_input("Invalid collection type").into()),
        }
    }
}

impl ParamsExtractor for SearchCollectionSummaryRequest {
    type Params = (Option<LibraryHandle>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.library.clone(),)
    }
}

impl Signal for SearchCollectionSummaryRequest {
    type Params = (Option<LibraryHandle>,);
    type Response = SearchCollectionSummaryResponse;

    async fn handle(
        &self,
        (library,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let main_db = &library.main_db;
        let params = CollectionActionParams {
            n: Some(dart_signal.n.try_into().map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid N: {error}"))
            })?),
            ..Default::default()
        };

        match dart_signal.collection_type {
            Some(CollectionType::Album) => handle_search::<albums::Model>(main_db, params).await,
            Some(CollectionType::Artist) => handle_search::<artists::Model>(main_db, params).await,
            Some(CollectionType::Playlist) => {
                handle_search::<playlists::Model>(main_db, params).await
            }
            Some(CollectionType::Mix) => handle_search::<mixes::Model>(main_db, params).await,
            Some(CollectionType::Genre) => handle_search::<genres::Model>(main_db, params).await,
            _ => Err(ExpectedRequestError::invalid_input(format!(
                "Invalid collection type: {:?}",
                dart_signal.collection_type
            ))
            .into()),
        }
    }
}
