use std::{marker::PhantomData, str::FromStr, sync::Arc};

use anyhow::{Error, Result};
use async_trait::async_trait;
use futures::future::try_join_all;
use log::warn;

use ::database::{
    actions::{
        collection::{
            CollectionQuery, CollectionQueryListMode, CollectionQueryType, UnifiedCollection,
        },
        file::{get_media_files, get_random_files, get_reverse_listed_media_files},
        metadata::{MetadataSummary, get_metadata_summary_by_files},
        mixes::query_mix_media_files,
    },
    connection::{MainDbConnection, RecommendationDbConnection},
    entities::{albums, artists, genres, media_files, mixes, playlists},
};
use ::fsio::FsIo;

use crate::{
    Session, Signal,
    messages::*,
    utils::{GlobalParams, ParamsExtractor},
};

#[async_trait]
pub trait ComplexQuery: Send + Sync {
    async fn execute(
        &self,
        main_db: &MainDbConnection,
        recommend_db: &RecommendationDbConnection,
    ) -> Result<Vec<UnifiedCollection>>;
}

#[async_trait]
impl<T> ComplexQuery for CollectionComplexQuery<T>
where
    T: CollectionQuery + Send + Sync,
{
    async fn execute(
        &self,
        main_db: &MainDbConnection,
        _: &RecommendationDbConnection,
    ) -> Result<Vec<UnifiedCollection>> {
        self.query_model_collections(main_db).await
    }
}

struct CollectionComplexQuery<T> {
    limit: u32,
    mode: CollectionQueryListMode,
    _phantom: PhantomData<T>,
}

impl<T> CollectionComplexQuery<T>
where
    T: CollectionQuery,
{
    fn new(limit: u32, mode: CollectionQueryListMode) -> Self {
        Self {
            limit,
            mode,
            _phantom: PhantomData,
        }
    }

    async fn query_model_collections(
        &self,
        main_db: &MainDbConnection,
    ) -> Result<Vec<UnifiedCollection>> {
        let models = T::list(main_db, self.limit.into(), self.mode).await?;
        let requests = models
            .into_iter()
            .map(|model| UnifiedCollection::from_model(main_db, model, false));

        try_join_all(requests).await
    }
}

struct NaiveTrackComplexQuery {
    mode: CollectionQueryListMode,
}

struct MixTrackComplexQuery {
    query: Vec<(String, String)>,
    enabled: bool,
}

async fn build_track_collections(
    main_db: &MainDbConnection,
    tracks: Vec<media_files::Model>,
) -> Result<Vec<UnifiedCollection>> {
    let metadata = get_metadata_summary_by_files(main_db, tracks).await?;
    let all_ids: Vec<String> = metadata.iter().map(|x| x.id.to_string()).collect();

    Ok(metadata
        .into_iter()
        .enumerate()
        .map(|(idx, x)| create_track_collection(x, &all_ids, idx))
        .collect())
}

fn create_track_collection(
    metadata: MetadataSummary,
    all_ids: &[String],
    idx: usize,
) -> UnifiedCollection {
    let mut queries = Vec::new();
    for item in all_ids.iter().skip(idx) {
        queries.push(("lib::track".to_string(), item.clone()));
    }
    for item in all_ids.iter().take(idx) {
        queries.push(("lib::track".to_string(), item.clone()));
    }

    UnifiedCollection {
        id: metadata.id,
        name: metadata.title,
        queries,
        collection_type: CollectionQueryType::Track,
        readonly: false,
    }
}

#[async_trait]
impl ComplexQuery for NaiveTrackComplexQuery {
    async fn execute(
        &self,
        main_db: &MainDbConnection,
        _: &RecommendationDbConnection,
    ) -> Result<Vec<UnifiedCollection>> {
        let tracks = match self.mode {
            // This is actually fake since we didn't find anywhere that use this case
            CollectionQueryListMode::Name => get_media_files(main_db, 0, 25).await,
            CollectionQueryListMode::Forward => get_media_files(main_db, 0, 25).await,
            CollectionQueryListMode::Reverse => {
                get_reverse_listed_media_files(main_db, 0, 25).await
            }
            CollectionQueryListMode::Random => get_random_files(main_db, 25).await,
        }?;

        build_track_collections(main_db, tracks).await
    }
}

#[async_trait]
impl ComplexQuery for MixTrackComplexQuery {
    async fn execute(
        &self,
        main_db: &MainDbConnection,
        recommend_db: &RecommendationDbConnection,
    ) -> Result<Vec<UnifiedCollection>> {
        if !self.enabled {
            return Ok(vec![]);
        }

        let tracks =
            query_mix_media_files(main_db, recommend_db, self.query.clone(), 0, 25).await?;

        build_track_collections(main_db, tracks).await
    }
}

fn create_query(domain: &str, parameter: &str) -> Result<Box<dyn ComplexQuery>> {
    match domain {
        "artists" => Ok(Box::new(CollectionComplexQuery::<artists::Model>::new(
            25,
            CollectionQueryListMode::from_str(parameter)?,
        ))),
        "albums" => Ok(Box::new(CollectionComplexQuery::<albums::Model>::new(
            25,
            CollectionQueryListMode::from_str(parameter)?,
        ))),
        "genres" => Ok(Box::new(CollectionComplexQuery::<genres::Model>::new(
            25,
            CollectionQueryListMode::from_str(parameter)?,
        ))),
        "playlists" => Ok(Box::new(CollectionComplexQuery::<playlists::Model>::new(
            25,
            CollectionQueryListMode::from_str(parameter)?,
        ))),
        "mixes" => Ok(Box::new(CollectionComplexQuery::<mixes::Model>::new(
            25,
            CollectionQueryListMode::from_str(parameter)?,
        ))),
        "tracks" => Ok(Box::new(NaiveTrackComplexQuery {
            mode: CollectionQueryListMode::from_str(parameter)?,
        })),
        "liked" => Ok(Box::new(MixTrackComplexQuery {
            query: vec![
                ("lib::directory.deep".to_owned(), "/".to_owned()),
                ("filter::liked".to_owned(), "true".to_owned()),
            ],
            enabled: parameter == "enable",
        })),
        "most" => Ok(Box::new(MixTrackComplexQuery {
            query: vec![
                ("lib::directory.deep".to_owned(), "/".to_owned()),
                ("sort::playedthrough".to_owned(), "false".to_owned()),
            ],
            enabled: parameter == "enable",
        })),
        unknown => {
            warn!("Unknown complex query operator: {unknown}");

            Ok(Box::new(MixTrackComplexQuery {
                query: vec![],
                enabled: false,
            }))
        }
    }
}

impl ParamsExtractor for ComplexQueryRequest {
    type Params = (
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
        crate::utils::RunningMode,
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

impl Signal for ComplexQueryRequest {
    type Params = (
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
        crate::utils::RunningMode,
    );
    type Response = ComplexQueryResponse;

    async fn handle(
        &self,
        (fsio, main_db, recommend_db, running_mode): Self::Params,
        session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let queries = &dart_signal.queries;
        let remote_host = match session {
            Some(x) => Some(x.host.clone()),
            None => None,
        };

        let futures = queries.iter().map(|query| {
            let main_db = main_db.clone();
            let recommend_db = recommend_db.clone();
            let fsio = Arc::clone(&fsio);

            {
                let value = remote_host.clone();
                async move {
                    let fsio = Arc::clone(&fsio);
                    let query_executor = create_query(&query.domain, &query.parameter)?;
                    let unified_collections =
                        query_executor.execute(&main_db, &recommend_db).await?;
                    let remote_host = value.clone();

                    let entries_futures =
                        unified_collections.into_iter().map(|unified_collection| {
                            let fsio = Arc::clone(&fsio);
                            let main_db = main_db.clone();
                            let recommend_db = recommend_db.clone();
                            let remote_host = remote_host.clone();
                            async move {
                                // Get the collection with processed cover art paths
                                let mut collection =
                                    Collection::from_unified_collection(unified_collection);
                                if collection.cover_art_map.is_empty() {
                                    collection = crate::utils::inject_cover_art_map(
                                        &fsio,
                                        &main_db,
                                        recommend_db,
                                        collection,
                                        None,
                                        &running_mode,
                                        &remote_host,
                                    )
                                    .await?;
                                }

                                Ok::<ComplexQueryEntry, Error>(ComplexQueryEntry {
                                    id: collection.id,
                                    name: collection.name,
                                    queries: collection.queries,
                                    collection_type: collection.collection_type,
                                    cover_art_map: collection.cover_art_map,
                                    readonly: collection.readonly,
                                })
                            }
                        });

                    let entries = try_join_all(entries_futures).await?;

                    Ok::<ComplexQueryGroup, Error>(ComplexQueryGroup {
                        id: query.id.clone(),
                        title: query.title.clone(),
                        entries,
                    })
                }
            }
        });

        let result = try_join_all(futures).await?;

        Ok(Some(ComplexQueryResponse { result }))
    }
}
