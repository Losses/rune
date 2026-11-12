use crate::node::NodeContext;
use database::actions::mixes::{add_item_to_mix, create_mix, remove_mix, update_mix};
use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result};

use ::database::actions::{
    cover_art::bake_cover_art_by_media_files,
    metadata::get_metadata_summary_by_files,
    mixes::{get_all_mixes, get_mix_by_id, get_mix_queries_by_mix_id, query_mix_media_files},
};

use crate::library_pool::LibraryHandle;
use crate::utils::{
    GlobalParams, ParamsExtractor, parse_media_files, process_cover_art_map, require_library,
};
use crate::{Session, Signal, messages::*};

type MixParams = Option<LibraryHandle>;
fn mix_params(params: &GlobalParams) -> MixParams {
    params.library.clone()
}

impl ParamsExtractor for FetchAllMixesRequest {
    type Params = Option<LibraryHandle>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.library.clone()
    }
}

impl Signal for FetchAllMixesRequest {
    type Params = Option<LibraryHandle>;
    type Response = FetchAllMixesResponse;

    async fn handle(
        &self,
        library: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let mixes = get_all_mixes(&library.main_db)
            .await
            .with_context(|| "Failed to fetch all mixes")?;

        Ok(Some(FetchAllMixesResponse {
            call_id: None,
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
        library: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;
        let store = library.library_sync.record_store();
        let mix = create_mix(
            &library.main_db,
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
            call_id: None,
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
        library: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;
        let store = library.library_sync.record_store();
        let mix = update_mix(
            &library.main_db,
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
            call_id: None,
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
        library: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;
        let store = library.library_sync.record_store();
        remove_mix(&library.main_db, &store, request.mix_id)
            .await
            .with_context(|| format!("Failed to remove mix with id: {}", request.mix_id))?;

        Ok(Some(RemoveMixResponse {
            call_id: None,
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
        library: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;

        let mix_id = request.mix_id;
        let operator = &request.operator;
        let parameter = &request.parameter;

        let store = library.library_sync.record_store();
        add_item_to_mix(
            &library.main_db, &store, mix_id,
            operator.clone(),
            parameter.clone(),
        )
        .await
        .with_context(|| {
            format!(
                "Failed to add item to mix: mix_id={mix_id}, operator={operator}, parameter={parameter}"
            )
        })?;

        Ok(Some(AddItemToMixResponse {
            call_id: None,
            success: true,
        }))
    }
}

impl ParamsExtractor for GetMixByIdRequest {
    type Params = Option<LibraryHandle>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.library.clone()
    }
}

impl Signal for GetMixByIdRequest {
    type Params = Option<LibraryHandle>;
    type Response = GetMixByIdResponse;

    async fn handle(
        &self,
        library: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;

        let mix = get_mix_by_id(&library.main_db, request.mix_id)
            .await
            .with_context(|| format!("Failed to get mix by id: {}", request.mix_id))?;

        Ok(Some(GetMixByIdResponse {
            call_id: None,
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
    type Params = (Option<LibraryHandle>, Arc<String>, Arc<NodeContext>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library.clone(),
            Arc::clone(&all_params.node_id),
            all_params.node.clone(),
        )
    }
}

impl Signal for MixQueryRequest {
    type Params = (Option<LibraryHandle>, Arc<String>, Arc<NodeContext>);
    type Response = MixQueryResponse;

    async fn handle(
        &self,
        (library, node_id, node): Self::Params,
        session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;
        let lib_path = Arc::new(library.media_root.clone());

        let queries = request
            .queries
            .clone()
            .into_iter()
            .map(|x| (x.operator, x.parameter))
            .collect();

        let queue = node.session_queue_for_library(library.library_id).await?;
        let media_entries = query_mix_media_files(
            &library.main_db,
            &library.recommend_db,
            queries,
            request.cursor as usize,
            request.page_size as usize,
            &queue,
        )
        .await
        .with_context(|| "Unable to query mix media files")?;

        let media_summaries =
            get_metadata_summary_by_files(library.main_db.as_ref(), media_entries.clone())
                .await
                .with_context(|| "Failed to get media summaries")?;

        let files = parse_media_files(
            &library.fsio,
            &library.main_db,
            &node_id,
            &library.library_id.to_string(),
            media_summaries,
            lib_path,
        )
        .await?;
        let cover_art_map = if request.bake_cover_arts {
            bake_cover_art_by_media_files(&library.fsio, &library.main_db, media_entries).await?
        } else {
            HashMap::new()
        };
        let cover_art_map =
            process_cover_art_map(cover_art_map, library.library_id, session.as_ref());

        Ok(Some(MixQueryResponse {
            call_id: None,
            files,
            cover_art_map,
        }))
    }
}

impl ParamsExtractor for FetchMixQueriesRequest {
    type Params = Option<LibraryHandle>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.library.clone()
    }
}

impl Signal for FetchMixQueriesRequest {
    type Params = Option<LibraryHandle>;
    type Response = FetchMixQueriesResponse;

    async fn handle(
        &self,
        library: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;

        let queries = get_mix_queries_by_mix_id(&library.main_db, request.mix_id)
            .await
            .with_context(|| "Unable to get mix queries files")?;

        Ok(Some(FetchMixQueriesResponse {
            call_id: None,
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
