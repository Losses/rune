use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use rinf::DartSignal;

use database::actions::cover_art::bake_cover_art_by_media_files;
use database::actions::metadata::get_metadata_summary_by_files;
use database::actions::mixes::{
    add_item_to_mix, create_mix, get_all_mixes, get_mix_by_id, get_mix_queries_by_mix_id,
    query_mix_media_files, remove_mix, replace_mix_queries, update_mix,
};
use database::connection::{MainDbConnection, RecommendationDbConnection};

use crate::{
    parse_media_files, AddItemToMixRequest, AddItemToMixResponse, CreateMixRequest,
    CreateMixResponse, FetchAllMixesRequest, FetchAllMixesResponse, FetchMixQueriesRequest,
    FetchMixQueriesResponse, GetMixByIdRequest, GetMixByIdResponse, Mix, MixQuery, MixQueryRequest,
    MixQueryResponse, RemoveMixRequest, RemoveMixResponse, UpdateMixRequest, UpdateMixResponse,
};

pub async fn fetch_all_mixes_request(
    main_db: Arc<MainDbConnection>,
    _dart_signal: DartSignal<FetchAllMixesRequest>,
) -> Result<()> {
    let mixes = get_all_mixes(&main_db)
        .await
        .with_context(|| "Failed to fetch all mixes")?;

    FetchAllMixesResponse {
        mixes: mixes
            .into_iter()
            .map(|mix| Mix {
                id: mix.id,
                name: mix.name,
                group: mix.group,
                locked: mix.locked,
                mode: mix.mode.expect("Mix mode not exists"),
            })
            .collect(),
    }
    .send_signal_to_dart();

    Ok(())
}

pub async fn create_mix_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<CreateMixRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    let mix = create_mix(
        &main_db,
        request.name,
        request.group,
        request.scriptlet_mode,
        request.mode,
        false,
    )
    .await
    .with_context(|| "Failed to create mix")?;
    CreateMixResponse {
        mix: Some(Mix {
            id: mix.id,
            name: mix.name,
            group: mix.group,
            locked: mix.locked,
            mode: mix.mode.expect("Mix mode not exists"),
        }),
    }
    .send_signal_to_dart();

    replace_mix_queries(
        &main_db,
        mix.id,
        request
            .queries
            .into_iter()
            .map(|x| (x.operator, x.parameter))
            .collect(),
        None,
    )
    .await
    .with_context(|| "Failed to update replace mix queries while creating")?;

    Ok(())
}

pub async fn update_mix_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<UpdateMixRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    let mix = update_mix(
        &main_db,
        request.mix_id,
        Some(request.name),
        Some(request.group),
        Some(request.scriptlet_mode),
        Some(request.mode),
        Some(false),
    )
    .await
    .with_context(|| "Failed to update mix metadata")?;

    if !request.queries.is_empty() {
        replace_mix_queries(
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
        .with_context(|| "Failed to update replace mix queries while updating")?;

        UpdateMixResponse {
            mix: Some(Mix {
                id: mix.id,
                name: mix.name,
                group: mix.group,
                locked: mix.locked,
                mode: mix.mode.expect("Mix mode not exists"),
            }),
        }
        .send_signal_to_dart();
    }

    Ok(())
}

pub async fn remove_mix_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<RemoveMixRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    remove_mix(&main_db, request.mix_id)
        .await
        .with_context(|| format!("Failed to remove mix with id: {}", request.mix_id))?;

    RemoveMixResponse {
        mix_id: request.mix_id,
        success: true,
    }
    .send_signal_to_dart();

    Ok(())
}

pub async fn add_item_to_mix_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<AddItemToMixRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    let mix_id = request.mix_id;
    let operator = request.operator;
    let parameter = request.parameter;

    add_item_to_mix(&main_db, mix_id, operator.clone(), parameter.clone())
        .await
        .with_context(|| {
            format!(
                "Failed to add item to mix: mix_id={}, operator={}, parameter={}",
                mix_id, operator, parameter
            )
        })?;

    AddItemToMixResponse { success: true }.send_signal_to_dart();

    Ok(())
}

pub async fn get_mix_by_id_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<GetMixByIdRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    let mix = get_mix_by_id(&main_db, request.mix_id)
        .await
        .with_context(|| format!("Failed to get mix by id: {}", request.mix_id))?;

    GetMixByIdResponse {
        mix: Some(Mix {
            id: mix.id,
            name: mix.name,
            group: mix.group,
            locked: mix.locked,
            mode: mix.mode.expect("Mix mode not exists"),
        }),
    }
    .send_signal_to_dart();

    Ok(())
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

    let media_entries = query_mix_media_files(
        &main_db,
        &recommend_db,
        queries,
        request.cursor as usize,
        request.page_size as usize,
    )
    .await
    .with_context(|| "Unable to query mix media files")?;

    let media_summaries = get_metadata_summary_by_files(&main_db, media_entries.clone())
        .await
        .with_context(|| "Failed to get media summaries")?;

    let files = parse_media_files(media_summaries, lib_path).await?;
    let cover_art_map = if request.bake_cover_arts {
        bake_cover_art_by_media_files(&main_db, media_entries).await?
    } else {
        HashMap::new()
    };

    MixQueryResponse {
        files,
        cover_art_map,
    }
    .send_signal_to_dart();

    Ok(())
}

pub async fn fetch_mix_queries_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<FetchMixQueriesRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    let queries = get_mix_queries_by_mix_id(&main_db, request.mix_id)
        .await
        .with_context(|| "Unable to get mix queries files")?;

    FetchMixQueriesResponse {
        result: queries
            .into_iter()
            .map(|x| MixQuery {
                operator: x.operator,
                parameter: x.parameter,
            })
            .collect(),
    }
    .send_signal_to_dart();

    Ok(())
}
