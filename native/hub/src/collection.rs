use std::sync::Arc;

use anyhow::{Context, Result};
use database::actions::mixes::list_mixes;
use database::actions::playlists::list_playlists;
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
use database::actions::playlists::get_playlists_by_ids;
use database::actions::playlists::get_playlists_groups;
use database::actions::utils::create_count_by_first_letter;
use database::actions::utils::CountByFirstLetter;
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
use crate::messages::collection::CollectionSummary;
use crate::messages::collection::FetchCollectionByIdsRequest;
use crate::messages::collection::FetchCollectionByIdsResponse;
use crate::messages::collection::FetchCollectionGroupSummaryRequest;
use crate::messages::collection::FetchCollectionGroupsRequest;
use crate::messages::collection::SearchCollectionSummaryRequest;
use crate::messages::collection::SearchCollectionSummaryResponse;

use crate::MixQuery;

pub async fn fetch_collection_group_summary_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<FetchCollectionGroupSummaryRequest>,
) -> Result<()> {
    let collection_type = dart_signal.message.collection_type;

    // Define a helper function to handle the common logic
    async fn task<T>(
        main_db: &Arc<MainDbConnection>,
        collection_type: i32,
        collection_type_hint: String,
    ) -> Result<()>
    where
        T: CountByFirstLetter + Send + Sync + 'static,
    {
        let count_fn = create_count_by_first_letter::<T>();

        let entry = count_fn(main_db)
            .await
            .with_context(|| format!("Failed to fetch {} groups summary", collection_type_hint))?;

        let collection_groups = entry
            .into_iter()
            .map(|x| CollectionGroupSummary {
                group_title: x.0,
                count: x.1,
            })
            .collect();

        CollectionGroupSummaryResponse {
            collection_type,
            groups: collection_groups,
        }
        .send_signal_to_dart();

        Ok(())
    }

    match collection_type {
        0 => task::<albums::Entity>(&main_db, 0, "album".to_string()).await?,
        1 => task::<artists::Entity>(&main_db, 1, "artist".to_string()).await?,
        2 => task::<playlists::Entity>(&main_db, 2, "playlist".to_string()).await?,
        3 => task::<mixes::Entity>(&main_db, 3, "mix".to_string()).await?,
        _ => return Err(anyhow::anyhow!("Invalid collection type")),
    }

    Ok(())
}

pub async fn fetch_collection_groups_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<FetchCollectionGroupsRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    macro_rules! task {
        ($collection_type:expr, $fetch_fn:ident, $collection_type_val:expr, $query_operator:expr) => {{
            let entry = $fetch_fn(&main_db, request.group_titles)
                .await
                .with_context(|| {
                    format!("Failed to fetch {} groups", stringify!($collection_type))
                })?;

            CollectionGroups {
                groups: entry
                    .into_iter()
                    .map(|x| CollectionGroup {
                        group_title: x.0,
                        collections: x
                            .1
                            .into_iter()
                            .map(|x| Collection {
                                id: x.0.id,
                                name: x.0.name,
                                queries: vec![MixQuery {
                                    operator: $query_operator.to_string(),
                                    parameter: x.0.id.to_string(),
                                }],
                                collection_type: $collection_type_val,
                            })
                            .collect(),
                    })
                    .collect(),
            }
            .send_signal_to_dart();
        }};
    }

    match request.collection_type {
        0 => task!(albums, get_albums_groups, 0, "lib::album"),
        1 => task!(artists, get_artists_groups, 1, "lib::artist"),
        2 => task!(playlists, get_playlists_groups, 2, "lib::playlist"),
        3 => {
            let entry = get_mixes_groups(&main_db, request.group_titles)
                .await
                .with_context(|| "Failed to fetch mix groups")?;

            let futures: Vec<_> = entry
                .iter()
                .flat_map(|(group_title, mixes)| {
                    mixes.iter().map({
                        let main_db = Arc::clone(&main_db);
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
        _ => return Err(anyhow::anyhow!("Invalid collection type")),
    }

    Ok(())
}

pub async fn fetch_collection_by_ids_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<FetchCollectionByIdsRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    macro_rules! task {
        ($collection_type:expr, $fetch_fn:ident, $collection_type_val:expr, $query_operator:expr) => {{
            let items = $fetch_fn(&main_db, &request.ids).await.with_context(|| {
                format!("Failed to fetch {} by id", stringify!($collection_type))
            })?;

            FetchCollectionByIdsResponse {
                collection_type: $collection_type_val,
                result: items
                    .into_iter()
                    .map(|x| Collection {
                        id: x.id,
                        name: x.name,
                        queries: vec![MixQuery {
                            operator: $query_operator.to_string(),
                            parameter: x.id.to_string(),
                        }],
                        collection_type: $collection_type_val,
                    })
                    .collect(),
            }
            .send_signal_to_dart();
        }};
    }

    match request.collection_type {
        0 => task!(albums, get_albums_by_ids, 0, "lib::album"),
        1 => task!(artists, get_artists_by_ids, 1, "lib::artist"),
        2 => task!(playlists, get_playlists_by_ids, 2, "lib::playlist"),
        3 => {
            let items = get_mixes_by_ids(&main_db, &request.ids)
                .await
                .with_context(|| "Failed to fetch mixes by id")?;

            let futures: Vec<_> = items
                .iter()
                .map({
                    let main_db = Arc::clone(&main_db);
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
        _ => return Err(anyhow::anyhow!("Invalid collection type")),
    }

    Ok(())
}

pub async fn search_collection_summary_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<SearchCollectionSummaryRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    macro_rules! task {
        ($fetch_fn:ident, $collection_type_val:expr, $query_operator:expr, $n:expr) => {{
            let items = $fetch_fn(&main_db, $n.try_into().unwrap())
                .await
                .with_context(|| format!("Failed to search {} summary", stringify!($fetch_fn)))?;

            SearchCollectionSummaryResponse {
                collection_type: $collection_type_val,
                result: items
                    .into_iter()
                    .map(|x| CollectionSummary {
                        id: x.id,
                        name: x.name,
                        queries: vec![MixQuery {
                            operator: $query_operator.to_string(),
                            parameter: x.id.to_string(),
                        }],
                    })
                    .collect(),
            }
            .send_signal_to_dart();
        }};
    }

    match request.collection_type {
        0 => task!(list_albums, 0, "lib::album", request.n),
        1 => task!(list_artists, 1, "lib::artist", request.n),
        2 => task!(list_playlists, 2, "lib::playlist", request.n),
        3 => {
            let items = list_mixes(&main_db, request.n.try_into().unwrap())
                .await
                .with_context(|| "Failed to fetch all mixes")?;

            let futures: Vec<_> = items
                .iter()
                .map({
                    let main_db = Arc::clone(&main_db);
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
                    .map(|(mix, queries)| CollectionSummary {
                        id: mix.id,
                        name: mix.name,
                        queries: queries
                            .into_iter()
                            .map(|x| MixQuery {
                                operator: x.operator,
                                parameter: x.parameter,
                            })
                            .collect(),
                    })
                    .collect(),
            }
            .send_signal_to_dart();
        }
        _ => return Err(anyhow::anyhow!("Invalid collection type")),
    }

    Ok(())
}
