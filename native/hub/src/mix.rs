use std::sync::Arc;

use anyhow::Result;
use log::{debug, error, info};
use rinf::DartSignal;

use database::actions::metadata::get_metadata_summary_by_files;
use database::actions::mixes::{
    add_item_to_mix, create_mix, get_all_mixes, get_mix_by_id, get_mix_queries_by_mix_id,
    get_mixes_by_ids, get_mixes_groups, get_unique_mix_groups, query_mix_media_files,
    replace_mix_queries, update_mix,
};
use database::actions::utils::create_count_by_first_letter;
use database::connection::{MainDbConnection, RecommendationDbConnection};
use database::entities::mixes;

use crate::{
    parse_media_files, AddItemToMixRequest, AddItemToMixResponse, CreateMixRequest,
    CreateMixResponse, FetchAllMixesRequest, FetchAllMixesResponse, FetchMixQueriesRequest,
    FetchMixQueriesResponse, FetchMixesByIdsRequest, FetchMixesByIdsResponse,
    FetchMixesGroupSummaryRequest, FetchMixesGroupsRequest, GetMixByIdRequest, GetMixByIdResponse,
    GetUniqueMixGroupsRequest, GetUniqueMixGroupsResponse, Mix, MixGroupSummaryResponse, MixQuery,
    MixQueryRequest, MixQueryResponse, MixWithoutCoverIds, MixesGroup, MixesGroupSummary,
    MixesGroups, UpdateMixRequest, UpdateMixResponse,
};

pub async fn fetch_mixes_group_summary_request(
    main_db: Arc<MainDbConnection>,
    _dart_signal: DartSignal<FetchMixesGroupSummaryRequest>,
) {
    debug!("Requesting summary group");

    let count_mixes = create_count_by_first_letter::<mixes::Entity>();

    match count_mixes(&main_db).await {
        Ok(entry) => {
            let mixes_groups = entry
                .into_iter()
                .map(|x| MixesGroupSummary {
                    group_title: x.0,
                    count: x.1,
                })
                .collect();
            MixGroupSummaryResponse { mixes_groups }.send_signal_to_dart();
            // GENERATED
        }
        Err(e) => {
            error!("Failed to fetch mixes groups summary: {}", e);
        }
    };
}

pub async fn fetch_mixes_groups_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<FetchMixesGroupsRequest>,
) {
    let request = dart_signal.message;

    debug!("Requesting mixs groups");

    match get_mixes_groups(&main_db, request.group_titles).await {
        Ok(entry) => {
            MixesGroups {
                groups: entry
                    .into_iter()
                    .map(|x| MixesGroup {
                        group_title: x.0,
                        mixes: x
                            .1
                            .into_iter()
                            .map(|x| Mix {
                                id: x.id,
                                name: x.name,
                                group: x.group,
                                cover_ids: [].to_vec(),
                            })
                            .collect(),
                    })
                    .collect(),
            }
            .send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to fetch mixs groups: {}", e);
        }
    };
}

pub async fn fetch_mixes_by_ids_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<FetchMixesByIdsRequest>,
) {
    let request = dart_signal.message;

    debug!("Requesting mixs: {:#?}", request.ids);

    match get_mixes_by_ids(&main_db, &request.ids).await {
        Ok(items) => {
            FetchMixesByIdsResponse {
                result: items
                    .into_iter()
                    .map(|x| Mix {
                        id: x.id,
                        name: x.name,
                        group: x.group,
                        cover_ids: [].to_vec(),
                    })
                    .collect(),
            }
            .send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to fetch albums groups: {}", e);
        }
    };
}

pub async fn fetch_all_mixes_request(
    main_db: Arc<MainDbConnection>,
    _dart_signal: DartSignal<FetchAllMixesRequest>,
) {
    debug!("Fetching all mixs");

    match get_all_mixes(&main_db).await {
        Ok(mixes) => {
            FetchAllMixesResponse {
                mixes: mixes
                    .into_iter()
                    .map(|mix| MixWithoutCoverIds {
                        id: mix.id,
                        name: mix.name,
                        group: mix.group,
                    })
                    .collect(),
            }
            .send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to fetch all mixes: {}", e);
        }
    }
}

pub async fn create_mix_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<CreateMixRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    debug!(
        "Creating mix: name={}, group={}",
        request.name, request.group
    );

    match create_mix(
        &main_db,
        request.name,
        request.group,
        request.scriptlet_mode,
        request.mode,
        false,
    )
    .await
    {
        Ok(mix) => {
            CreateMixResponse {
                mix: Some(MixWithoutCoverIds {
                    id: mix.id,
                    name: mix.name,
                    group: mix.group,
                }),
            }
            .send_signal_to_dart();

            let replace_result = replace_mix_queries(
                &main_db,
                mix.id,
                request
                    .queries
                    .into_iter()
                    .map(|x| (x.operator, x.parameter))
                    .collect(),
                None,
            )
            .await;

            if let Err(e) = replace_result {
                error!("Failed to update replace mix queries: {}", e);
            }
        }
        Err(e) => {
            error!("Failed to create mix: {}", e);
        }
    };

    Ok(())
}

pub async fn update_mix_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<UpdateMixRequest>,
) {
    let request = dart_signal.message;

    debug!(
        "Updating mix: id={}, name={:?}",
        request.mix_id, request.name
    );

    match update_mix(
        &main_db,
        request.mix_id,
        Some(request.name),
        Some(request.group),
        Some(request.scriptlet_mode),
        Some(request.mode),
        Some(false),
    )
    .await
    {
        Ok(mix) => {
            if !request.queries.is_empty() {
                match replace_mix_queries(
                    &main_db,
                    request.mix_id,
                    request
                        .queries
                        .into_iter()
                        .map(|x| (x.operator, x.parameter))
                        .collect(),
                    None,
                )
                .await
                {
                    Ok(_) => {
                        info!("Mix queries created");
                        UpdateMixResponse {
                            mix: Some(MixWithoutCoverIds {
                                id: mix.id,
                                name: mix.name,
                                group: mix.group,
                            }),
                        }
                        .send_signal_to_dart();
                    }
                    Err(e) => error!("Failed to update replace mix queries: {}", e),
                };
            }
        }
        Err(e) => error!("Failed to update mix metadata: {}", e),
    };
}

pub async fn add_item_to_mix_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<AddItemToMixRequest>,
) {
    let request = dart_signal.message;

    debug!(
        "Adding item to mix: mix_id={}, operator={}, parameter={:#?}",
        request.mix_id, request.operator, request.parameter
    );

    match add_item_to_mix(
        &main_db,
        request.mix_id,
        request.operator,
        request.parameter,
    )
    .await
    {
        Ok(_) => {
            AddItemToMixResponse { success: true }.send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to add item to mix: {}", e);
            AddItemToMixResponse { success: false }.send_signal_to_dart();
        }
    };
}

pub async fn get_unique_mix_groups_request(
    main_db: Arc<MainDbConnection>,
    _dart_signal: DartSignal<GetUniqueMixGroupsRequest>,
) {
    debug!("Requesting unique mix groups");

    match get_unique_mix_groups(&main_db).await {
        Ok(groups) => {
            GetUniqueMixGroupsResponse { groups }.send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to get unique mix groups: {}", e);
        }
    }
}

pub async fn get_mix_by_id_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<GetMixByIdRequest>,
) {
    let request = dart_signal.message;

    debug!("Requesting mix by id: {}", request.mix_id);

    match get_mix_by_id(&main_db, request.mix_id).await {
        Ok(mix) => {
            GetMixByIdResponse {
                mix: Some(MixWithoutCoverIds {
                    id: mix.id,
                    name: mix.name,
                    group: mix.group,
                }),
            }
            .send_signal_to_dart();
        }
        Err(e) => {
            error!("Failed to get mix by id: {}", e);
        }
    }
}

pub async fn mix_query_request(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    lib_path: Arc<String>,
    dart_signal: DartSignal<MixQueryRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    let queries = request
        .queries
        .into_iter()
        .map(|x| (x.operator, x.parameter))
        .collect();

    match query_mix_media_files(
        &main_db,
        &recommend_db,
        queries,
        request.cursor as usize,
        request.page_size as usize,
    )
    .await
    {
        Ok(media_entries) => {
            let media_summaries = get_metadata_summary_by_files(&main_db, media_entries);

            match media_summaries.await {
                Ok(media_summaries) => {
                    let result = parse_media_files(media_summaries, lib_path).await?;
                    MixQueryResponse { result }.send_signal_to_dart();
                    // GENERATED
                }
                Err(e) => {
                    error!("Error happened while getting media summaries: {:#?}", e)
                }
            }
        }
        Err(e) => error!("Unable to query mix media files: {}", e),
    }

    Ok(())
}

pub async fn fetch_mix_queries_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<FetchMixQueriesRequest>,
) {
    let request = dart_signal.message;

    match get_mix_queries_by_mix_id(&main_db, request.mix_id).await {
        Ok(queries) => {
            FetchMixQueriesResponse {
                result: queries
                    .into_iter()
                    .map(|x| MixQuery {
                        operator: x.operator,
                        parameter: x.parameter,
                    })
                    .collect(),
            }.send_signal_to_dart();
        }
        Err(e) => error!("Unable to get mix queries files: {}", e),
    }
}
