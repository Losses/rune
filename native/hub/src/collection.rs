use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::future::join_all;
use rinf::DartSignal;

use database::actions::albums::get_albums_by_ids;
use database::actions::albums::get_albums_groups;
use database::actions::albums::list_albums;
use database::actions::artists::get_artists_by_ids;
use database::actions::artists::get_artists_groups;
use database::actions::artists::list_artists;
use database::actions::mixes::get_mix_queries_by_mix_id;
use database::actions::mixes::get_mixes_by_ids;
use database::actions::mixes::get_mixes_groups;
use database::actions::mixes::list_mixes;
use database::actions::playlists::get_playlists_by_ids;
use database::actions::playlists::get_playlists_groups;
use database::actions::playlists::list_playlists;
use database::actions::utils::create_count_by_first_letter;
use database::connection::MainDbConnection;
use database::entities::albums;
use database::entities::artists;
use database::entities::mixes;
use database::entities::playlists;

use crate::messages::collection::Collection;
use crate::messages::collection::CollectionGroup;
use crate::messages::collection::CollectionGroupSummary;
use crate::messages::collection::CollectionGroupSummaryResponse;
use crate::messages::collection::CollectionGroups;
use crate::messages::collection::FetchCollectionByIdsRequest;
use crate::messages::collection::FetchCollectionByIdsResponse;
use crate::messages::collection::FetchCollectionGroupSummaryRequest;
use crate::messages::collection::FetchCollectionGroupsRequest;
use crate::messages::collection::SearchCollectionSummaryRequest;
use crate::messages::collection::SearchCollectionSummaryResponse;

use crate::MixQuery;

trait CollectionType: Send + Sync + 'static {
    fn collection_type() -> i32;
    fn type_name() -> &'static str;
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

impl CollectionType for albums::Model {
    fn collection_type() -> i32 {
        0
    }
    fn type_name() -> &'static str {
        "album"
    }
    fn query_operator() -> &'static str {
        "lib::album"
    }
    async fn count_by_first_letter(main_db: &Arc<MainDbConnection>) -> Result<Vec<(String, i32)>> {
        create_count_by_first_letter::<albums::Entity>()(main_db)
            .await
            .with_context(|| "Failed to count collection by first letter")
    }
    async fn get_groups(
        main_db: &Arc<MainDbConnection>,
        group_titles: Vec<String>,
    ) -> Result<Vec<(String, Vec<(Self, HashSet<i32>)>)>> {
        get_albums_groups(main_db, group_titles)
            .await
            .with_context(|| "Failed to get collection groups")
    }
    async fn get_by_ids(main_db: &Arc<MainDbConnection>, ids: &[i32]) -> Result<Vec<Self>> {
        get_albums_by_ids(main_db, ids)
            .await
            .with_context(|| "Failed to get collection item by ids")
    }
    async fn list(main_db: &Arc<MainDbConnection>, limit: u64) -> Result<Vec<Self>> {
        list_albums(main_db, limit)
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

impl CollectionType for artists::Model {
    fn collection_type() -> i32 {
        1
    }
    fn type_name() -> &'static str {
        "artist"
    }
    fn query_operator() -> &'static str {
        "lib::artist"
    }
    async fn count_by_first_letter(main_db: &Arc<MainDbConnection>) -> Result<Vec<(String, i32)>> {
        create_count_by_first_letter::<artists::Entity>()(main_db)
            .await
            .with_context(|| "Failed to count collection by first letter")
    }
    async fn get_groups(
        main_db: &Arc<MainDbConnection>,
        group_titles: Vec<String>,
    ) -> Result<Vec<(String, Vec<(Self, HashSet<i32>)>)>> {
        get_artists_groups(main_db, group_titles)
            .await
            .with_context(|| "Failed to get collection groups")
    }
    async fn get_by_ids(main_db: &Arc<MainDbConnection>, ids: &[i32]) -> Result<Vec<Self>> {
        get_artists_by_ids(main_db, ids)
            .await
            .with_context(|| "Failed to get collection item by ids")
    }
    async fn list(main_db: &Arc<MainDbConnection>, limit: u64) -> Result<Vec<Self>> {
        list_artists(main_db, limit)
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

impl CollectionType for playlists::Model {
    fn collection_type() -> i32 {
        2
    }
    fn type_name() -> &'static str {
        "playlist"
    }
    fn query_operator() -> &'static str {
        "lib::playlist"
    }
    async fn count_by_first_letter(main_db: &Arc<MainDbConnection>) -> Result<Vec<(String, i32)>> {
        create_count_by_first_letter::<playlists::Entity>()(main_db)
            .await
            .with_context(|| "Failed to count collection by first letter")
    }
    async fn get_groups(
        main_db: &Arc<MainDbConnection>,
        group_titles: Vec<String>,
    ) -> Result<Vec<(String, Vec<(Self, HashSet<i32>)>)>> {
        get_playlists_groups(main_db, group_titles)
            .await
            .with_context(|| "Failed to get collection groups")
    }
    async fn get_by_ids(main_db: &Arc<MainDbConnection>, ids: &[i32]) -> Result<Vec<Self>> {
        get_playlists_by_ids(main_db, ids)
            .await
            .with_context(|| "Failed to get collection item by ids")
    }
    async fn list(main_db: &Arc<MainDbConnection>, limit: u64) -> Result<Vec<Self>> {
        list_playlists(main_db, limit)
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

impl CollectionType for mixes::Model {
    fn collection_type() -> i32 {
        3
    }
    fn type_name() -> &'static str {
        "mix"
    }
    fn query_operator() -> &'static str {
        "lib::mix"
    }
    async fn count_by_first_letter(main_db: &Arc<MainDbConnection>) -> Result<Vec<(String, i32)>> {
        create_count_by_first_letter::<mixes::Entity>()(main_db)
            .await
            .with_context(|| "Failed to count collection by first letter")
    }
    async fn get_groups(
        main_db: &Arc<MainDbConnection>,
        group_titles: Vec<String>,
    ) -> Result<Vec<(String, Vec<(Self, HashSet<i32>)>)>> {
        get_mixes_groups(main_db, group_titles)
            .await
            .with_context(|| "Failed to get collection groups")
    }
    async fn get_by_ids(main_db: &Arc<MainDbConnection>, ids: &[i32]) -> Result<Vec<Self>> {
        get_mixes_by_ids(main_db, ids)
            .await
            .with_context(|| "Failed to get collection item by ids")
    }
    async fn list(main_db: &Arc<MainDbConnection>, limit: u64) -> Result<Vec<Self>> {
        list_mixes(main_db, limit)
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
}

async fn handle_collection_type<T: CollectionType>(
    main_db: &Arc<MainDbConnection>,
    action: CollectionAction,
    params: CollectionActionParams,
) -> Result<()> {
    match action {
        CollectionAction::FetchGroupSummary => {
            let entry = T::count_by_first_letter(main_db)
                .await
                .with_context(|| format!("Failed to fetch {} groups summary", T::type_name()))?;

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
            let entry = T::get_groups(main_db, params.group_titles.unwrap())
                .await
                .with_context(|| format!("Failed to fetch {} groups", T::type_name()))?;

            CollectionGroups {
                groups: entry
                    .into_iter()
                    .map(|x| CollectionGroup {
                        group_title: x.0,
                        collections: x
                            .1
                            .into_iter()
                            .map(|x| Collection {
                                id: x.0.id(),
                                name: x.0.name().to_owned(),
                                queries: vec![MixQuery {
                                    operator: T::query_operator().to_string(),
                                    parameter: x.0.id().to_string(),
                                }],
                                collection_type: T::collection_type(),
                            })
                            .collect(),
                    })
                    .collect(),
            }
            .send_signal_to_dart();
        }
        CollectionAction::FetchById => {
            let items = T::get_by_ids(main_db, &params.ids.unwrap())
                .await
                .with_context(|| format!("Failed to fetch {} by id", T::type_name()))?;

            FetchCollectionByIdsResponse {
                collection_type: T::collection_type(),
                result: items
                    .into_iter()
                    .map(|x| Collection {
                        id: x.id(),
                        name: x.name().to_owned(),
                        queries: vec![MixQuery {
                            operator: T::query_operator().to_string(),
                            parameter: x.id().to_string(),
                        }],
                        collection_type: T::collection_type(),
                    })
                    .collect(),
            }
            .send_signal_to_dart();
        }
        CollectionAction::Search => {
            let items = T::list(main_db, params.n.unwrap().into())
                .await
                .with_context(|| format!("Failed to search {} summary", T::type_name()))?;

            SearchCollectionSummaryResponse {
                collection_type: T::collection_type(),
                result: items
                    .into_iter()
                    .map(|x| Collection {
                        id: x.id(),
                        name: x.name().to_owned(),
                        queries: vec![MixQuery {
                            operator: T::query_operator().to_string(),
                            parameter: x.id().to_string(),
                        }],
                        collection_type: T::collection_type(),
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
    action: CollectionAction,
    params: CollectionActionParams,
) -> Result<()> {
    match action {
        CollectionAction::FetchGroups => {
            let entry = get_mixes_groups(main_db, params.group_titles.unwrap())
                .await
                .with_context(|| "Failed to fetch mix groups")?;

            let futures: Vec<_> = entry
                .iter()
                .flat_map(|(group_title, mixes)| {
                    mixes.iter().map({
                        let main_db = Arc::clone(main_db);
                        move |mix| {
                            let main_db = Arc::clone(&main_db);
                            let group_title = group_title.clone();
                            async move {
                                let queries = get_mix_queries_by_mix_id(&main_db, mix.0.id).await?;
                                Ok::<_, anyhow::Error>((group_title, mix.clone(), queries))
                            }
                        }
                    })
                })
                .collect();

            let results = join_all(futures)
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?;

            let groups = entry
                .into_iter()
                .map(|(group_title, _)| {
                    let collections = results
                        .iter()
                        .filter_map(|(gt, mix, queries)| {
                            if gt == &group_title {
                                Some(Collection {
                                    id: mix.0.id,
                                    name: mix.0.name.clone(),
                                    queries: queries
                                        .iter()
                                        .map(|x| MixQuery {
                                            operator: x.operator.clone(),
                                            parameter: x.parameter.clone(),
                                        })
                                        .collect(),
                                    collection_type: 3,
                                })
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
                .collect();

            CollectionGroups { groups }.send_signal_to_dart();
        }
        CollectionAction::FetchById => {
            let items = get_mixes_by_ids(main_db, &params.ids.unwrap())
                .await
                .with_context(|| "Failed to fetch mixes by id")?;

            let futures: Vec<_> = items
                .iter()
                .map({
                    let main_db = Arc::clone(main_db);
                    move |mix| {
                        let main_db = Arc::clone(&main_db);
                        async move {
                            let queries = get_mix_queries_by_mix_id(&main_db, mix.id).await?;
                            Ok::<_, anyhow::Error>((mix.clone(), queries))
                        }
                    }
                })
                .collect();

            let results = join_all(futures)
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?;

            FetchCollectionByIdsResponse {
                collection_type: 3,
                result: results
                    .into_iter()
                    .map(|(mix, queries)| Collection {
                        id: mix.id,
                        name: mix.name,
                        queries: queries
                            .into_iter()
                            .map(|x| MixQuery {
                                operator: x.operator,
                                parameter: x.parameter,
                            })
                            .collect(),
                        collection_type: 3,
                    })
                    .collect(),
            }
            .send_signal_to_dart();
        }
        CollectionAction::Search => {
            let items = list_mixes(main_db, params.n.unwrap().into())
                .await
                .with_context(|| "Failed to fetch all mixes")?;

            let futures: Vec<_> = items
                .iter()
                .map({
                    let main_db = Arc::clone(main_db);
                    move |mix| {
                        let main_db = Arc::clone(&main_db);
                        async move {
                            let queries = get_mix_queries_by_mix_id(&main_db, mix.id).await?;
                            Ok::<_, anyhow::Error>((mix.clone(), queries))
                        }
                    }
                })
                .collect();

            let results = join_all(futures)
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?;

            SearchCollectionSummaryResponse {
                collection_type: 3,
                result: results
                    .into_iter()
                    .map(|(mix, queries)| Collection {
                        id: mix.id,
                        name: mix.name,
                        queries: queries
                            .into_iter()
                            .map(|x| MixQuery {
                                operator: x.operator,
                                parameter: x.parameter,
                            })
                            .collect(),
                        collection_type: 3,
                    })
                    .collect(),
            }
            .send_signal_to_dart();
        }
        _ => handle_collection_type::<mixes::Model>(main_db, action, params).await?,
    }

    Ok(())
}

pub async fn handle_collection_request(
    main_db: Arc<MainDbConnection>,
    collection_type: i32,
    action: CollectionAction,
    params: CollectionActionParams,
) -> Result<()> {
    match collection_type {
        0 => handle_collection_type::<albums::Model>(&main_db, action, params).await?,
        1 => handle_collection_type::<artists::Model>(&main_db, action, params).await?,
        2 => handle_collection_type::<playlists::Model>(&main_db, action, params).await?,
        3 => handle_mixes(&main_db, action, params).await?,
        _ => return Err(anyhow::anyhow!("Invalid collection type")),
    }

    Ok(())
}

pub async fn fetch_collection_group_summary_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<FetchCollectionGroupSummaryRequest>,
) -> Result<()> {
    handle_collection_request(
        main_db,
        dart_signal.message.collection_type,
        CollectionAction::FetchGroupSummary,
        CollectionActionParams::default(),
    )
    .await
}

pub async fn fetch_collection_groups_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<FetchCollectionGroupsRequest>,
) -> Result<()> {
    handle_collection_request(
        main_db,
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
    dart_signal: DartSignal<FetchCollectionByIdsRequest>,
) -> Result<()> {
    handle_collection_request(
        main_db,
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
    dart_signal: DartSignal<SearchCollectionSummaryRequest>,
) -> Result<()> {
    handle_collection_request(
        main_db,
        dart_signal.message.collection_type,
        CollectionAction::Search,
        CollectionActionParams {
            n: Some(dart_signal.message.n.try_into().unwrap()),
            ..Default::default()
        },
    )
    .await
}
