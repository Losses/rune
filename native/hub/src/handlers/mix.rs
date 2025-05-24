use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};

use database::actions::cover_art::bake_cover_art_by_media_files;
use database::actions::metadata::get_metadata_summary_by_files;
use database::actions::mixes::{
    add_item_to_mix, create_mix, get_all_mixes, get_mix_by_id, get_mix_queries_by_mix_id,
    query_mix_media_files, remove_mix, replace_mix_queries, update_mix,
};
use database::connection::{MainDbConnection, RecommendationDbConnection};

use crate::utils::{parse_media_files, GlobalParams, ParamsExtractor};
use crate::{messages::*, Session, Signal};

impl ParamsExtractor for FetchAllMixesRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for FetchAllMixesRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = FetchAllMixesResponse;

    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let mixes = get_all_mixes(&main_db)
            .await
            .with_context(|| "Failed to fetch all mixes")?;

        Ok(Some(FetchAllMixesResponse {
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
        }))
    }
}

impl ParamsExtractor for CreateMixRequest {
    type Params = (Arc<MainDbConnection>, Arc<String>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.node_id),
        )
    }
}

impl Signal for CreateMixRequest {
    type Params = (Arc<MainDbConnection>, Arc<String>);
    type Response = CreateMixResponse;

    async fn handle(
        &self,
        (main_db, node_id): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let mix = create_mix(
            &main_db,
            &node_id,
            request.name.clone(),
            request.group.clone(),
            request.scriptlet_mode,
            request.mode,
            false,
        )
        .await
        .with_context(|| "Failed to create mix")?;

        replace_mix_queries(
            &main_db,
            &node_id,
            mix.id,
            request
                .queries
                .clone()
                .into_iter()
                .map(|x| (x.operator, x.parameter))
                .collect(),
            None,
        )
        .await
        .with_context(|| "Failed to update replace mix queries while creating")?;

        Ok(Some(CreateMixResponse {
            mix: Some(Mix {
                id: mix.id,
                name: mix.name,
                group: mix.group,
                locked: mix.locked,
                mode: mix.mode.expect("Mix mode not exists"),
            }),
        }))
    }
}

impl ParamsExtractor for UpdateMixRequest {
    type Params = (Arc<MainDbConnection>, Arc<String>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.node_id),
        )
    }
}

impl Signal for UpdateMixRequest {
    type Params = (Arc<MainDbConnection>, Arc<String>);
    type Response = UpdateMixResponse;

    async fn handle(
        &self,
        (main_db, node_id): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let mix = update_mix(
            &main_db,
            &node_id,
            request.mix_id,
            Some(request.name.clone()),
            Some(request.group.clone()),
            Some(request.scriptlet_mode),
            Some(request.mode),
            Some(false),
        )
        .await
        .with_context(|| "Failed to update mix metadata")?;

        replace_mix_queries(
            &main_db,
            &node_id,
            request.mix_id,
            request
                .queries
                .clone()
                .into_iter()
                .map(|x| (x.operator, x.parameter))
                .collect(),
            None,
        )
        .await
        .with_context(|| "Failed to update replace mix queries while updating")?;

        Ok(Some(UpdateMixResponse {
            mix: Some(Mix {
                id: mix.id,
                name: mix.name,
                group: mix.group,
                locked: mix.locked,
                mode: mix.mode.expect("Mix mode not exists"),
            }),
        }))
    }
}

impl ParamsExtractor for RemoveMixRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for RemoveMixRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = RemoveMixResponse;

    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        remove_mix(&main_db, request.mix_id)
            .await
            .with_context(|| format!("Failed to remove mix with id: {}", request.mix_id))?;

        Ok(Some(RemoveMixResponse {
            mix_id: request.mix_id,
            success: true,
        }))
    }
}

impl ParamsExtractor for AddItemToMixRequest {
    type Params = (Arc<MainDbConnection>, Arc<String>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.node_id),
        )
    }
}

impl Signal for AddItemToMixRequest {
    type Params = (Arc<MainDbConnection>, Arc<String>);
    type Response = AddItemToMixResponse;

    async fn handle(
        &self,
        (main_db, node_id): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let mix_id = request.mix_id;
        let operator = &request.operator;
        let parameter = &request.parameter;

        add_item_to_mix(
            &main_db,
            &node_id,
            mix_id,
            operator.clone(),
            parameter.clone(),
        )
        .await
        .with_context(|| {
            format!(
                "Failed to add item to mix: mix_id={}, operator={}, parameter={}",
                mix_id, operator, parameter
            )
        })?;

        Ok(Some(AddItemToMixResponse { success: true }))
    }
}

impl ParamsExtractor for GetMixByIdRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for GetMixByIdRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = GetMixByIdResponse;

    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let mix = get_mix_by_id(&main_db, request.mix_id)
            .await
            .with_context(|| format!("Failed to get mix by id: {}", request.mix_id))?;

        Ok(Some(GetMixByIdResponse {
            mix: Some(Mix {
                id: mix.id,
                name: mix.name,
                group: mix.group,
                locked: mix.locked,
                mode: mix.mode.expect("Mix mode not exists"),
            }),
        }))
    }
}

impl ParamsExtractor for MixQueryRequest {
    type Params = (
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
        Arc<String>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.recommend_db),
            Arc::clone(&all_params.lib_path),
        )
    }
}

impl Signal for MixQueryRequest {
    type Params = (
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
        Arc<String>,
    );
    type Response = MixQueryResponse;

    async fn handle(
        &self,
        (main_db, recommend_db, lib_path): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let queries = request
            .queries
            .clone()
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

        Ok(Some(MixQueryResponse {
            files,
            cover_art_map,
        }))
    }
}

impl ParamsExtractor for FetchMixQueriesRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for FetchMixQueriesRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = FetchMixQueriesResponse;

    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let queries = get_mix_queries_by_mix_id(&main_db, request.mix_id)
            .await
            .with_context(|| "Unable to get mix queries files")?;

        Ok(Some(FetchMixQueriesResponse {
            result: queries
                .into_iter()
                .map(|x| MixQuery {
                    operator: x.operator,
                    parameter: x.parameter,
                })
                .collect(),
        }))
    }
}
