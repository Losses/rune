use crate::request_error::ExpectedRequestError;
use database::actions::mixes::{add_item_to_mix, create_mix, remove_mix, update_mix};
use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result};

use ::database::{
    actions::{
        cover_art::bake_cover_art_by_media_files,
        metadata::get_metadata_summary_by_files,
        mixes::{get_all_mixes, get_mix_by_id, get_mix_queries_by_mix_id, query_mix_media_files},
    },
    connection::{MainDbConnection, RecommendationDbConnection},
};
use ::fsio::FsIo;

use crate::utils::{GlobalParams, ParamsExtractor, parse_media_files};
use crate::{Session, Signal, messages::*};

type MixParams = (
    Arc<MainDbConnection>,
    Option<database::sync::SeaOrmSyncStore>,
    Arc<String>,
);
fn mix_params(params: &GlobalParams) -> MixParams {
    (
        Arc::clone(&params.main_db),
        params.library_sync.as_ref().map(|sync| sync.record_store()),
        Arc::clone(&params.node_id),
    )
}

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
    type Params = MixParams;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        mix_params(all_params)
    }
}

impl Signal for CreateMixRequest {
    type Params = MixParams;
    type Response = CreateMixResponse;

    async fn handle(
        &self,
        (db, store, _node_id): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let store = store.ok_or(ExpectedRequestError::Unavailable {
            what: "mix synchronization",
        })?;
        let mix = create_mix(
            &db,
            &store,
            request.name.clone(),
            request.group.clone(),
            request.scriptlet_mode,
            request.mode,
            false,
            request
                .queries
                .clone()
                .into_iter()
                .map(|x| (x.operator, x.parameter))
                .collect(),
        )
        .await
        .with_context(|| "Failed to create mix")?;

        Ok(Some(CreateMixResponse {
            mix: Mix {
                id: mix.id,
                name: mix.name,
                group: mix.group,
                locked: mix.locked,
                mode: mix.mode.expect("Mix mode not exists"),
            },
        }))
    }
}

impl ParamsExtractor for UpdateMixRequest {
    type Params = MixParams;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        mix_params(all_params)
    }
}

impl Signal for UpdateMixRequest {
    type Params = MixParams;
    type Response = UpdateMixResponse;

    async fn handle(
        &self,
        (db, store, _node_id): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let store = store.ok_or(ExpectedRequestError::Unavailable {
            what: "mix synchronization",
        })?;
        let mix = update_mix(
            &db,
            &store,
            request.mix_id,
            Some(request.name.clone()),
            Some(request.group.clone()),
            Some(request.scriptlet_mode),
            Some(request.mode),
            Some(false),
            Some(
                request
                    .queries
                    .clone()
                    .into_iter()
                    .map(|x| (x.operator, x.parameter))
                    .collect(),
            ),
        )
        .await
        .with_context(|| "Failed to update mix metadata")?;

        Ok(Some(UpdateMixResponse {
            mix: Mix {
                id: mix.id,
                name: mix.name,
                group: mix.group,
                locked: mix.locked,
                mode: mix.mode.expect("Mix mode not exists"),
            },
        }))
    }
}

impl ParamsExtractor for RemoveMixRequest {
    type Params = MixParams;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        mix_params(all_params)
    }
}

impl Signal for RemoveMixRequest {
    type Params = MixParams;
    type Response = RemoveMixResponse;

    async fn handle(
        &self,
        (db, store, _node_id): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let store = store.ok_or(ExpectedRequestError::Unavailable {
            what: "mix synchronization",
        })?;
        remove_mix(&db, &store, request.mix_id)
            .await
            .with_context(|| format!("Failed to remove mix with id: {}", request.mix_id))?;

        Ok(Some(RemoveMixResponse {
            mix_id: request.mix_id,
            success: true,
        }))
    }
}

impl ParamsExtractor for AddItemToMixRequest {
    type Params = MixParams;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        mix_params(all_params)
    }
}

impl Signal for AddItemToMixRequest {
    type Params = MixParams;
    type Response = AddItemToMixResponse;

    async fn handle(
        &self,
        (db, store, _node_id): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let mix_id = request.mix_id;
        let operator = &request.operator;
        let parameter = &request.parameter;

        let store = store.ok_or(ExpectedRequestError::Unavailable {
            what: "mix synchronization",
        })?;
        add_item_to_mix(
            &db, &store, mix_id,
            operator.clone(),
            parameter.clone(),
        )
        .await
        .with_context(|| {
            format!(
                "Failed to add item to mix: mix_id={mix_id}, operator={operator}, parameter={parameter}"
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
            mix: Mix {
                id: mix.id,
                name: mix.name,
                group: mix.group,
                locked: mix.locked,
                mode: mix.mode.expect("Mix mode not exists"),
            },
        }))
    }
}

impl ParamsExtractor for MixQueryRequest {
    type Params = (
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
        Arc<String>,
        Arc<String>,
    );

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.fsio),
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.recommend_db),
            Arc::clone(&all_params.lib_path),
            Arc::clone(&all_params.node_id),
        )
    }
}

impl Signal for MixQueryRequest {
    type Params = (
        Arc<FsIo>,
        Arc<MainDbConnection>,
        Arc<RecommendationDbConnection>,
        Arc<String>,
        Arc<String>,
    );
    type Response = MixQueryResponse;

    async fn handle(
        &self,
        (fsio, main_db, recommend_db, lib_path, node_id): Self::Params,
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

        let media_summaries =
            get_metadata_summary_by_files(main_db.as_ref(), media_entries.clone())
                .await
                .with_context(|| "Failed to get media summaries")?;

        let files = parse_media_files(&fsio, &main_db, &node_id, media_summaries, lib_path).await?;
        let cover_art_map = if request.bake_cover_arts {
            bake_cover_art_by_media_files(&fsio, &main_db, media_entries).await?
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
                .map(|x| crate::messages::StoredMixQuery {
                    clause_id: x.clause_id.to_string(),
                    operator: x.operator,
                    parameter: x.parameter,
                    target_uuid: x.target_uuid.map(|id| id.to_string()),
                    available: x.available,
                })
                .collect(),
        }))
    }
}
