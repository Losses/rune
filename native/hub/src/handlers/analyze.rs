use anyhow::{Context, Result};

use ::database::actions::analysis::{get_analyze_count, if_analyze_exists};

use crate::{Session, Signal, messages::*};
use crate::{
    library_pool::LibraryHandle,
    utils::{GlobalParams, ParamsExtractor, require_library},
};

impl ParamsExtractor for IfAnalyzeExistsRequest {
    type Params = Option<LibraryHandle>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.library.clone()
    }
}

impl Signal for IfAnalyzeExistsRequest {
    type Params = Option<LibraryHandle>;
    type Response = IfAnalyzeExistsResponse;
    async fn handle(
        &self,
        library: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let file_id = dart_signal.file_id;

        let exists = if_analyze_exists(&library.main_db, file_id)
            .await
            .with_context(|| format!("Failed to test if analysis exists: {file_id}"))?;
        Ok(Some(IfAnalyzeExistsResponse { file_id, exists }))
    }
}

impl ParamsExtractor for GetAnalyzeCountRequest {
    type Params = Option<LibraryHandle>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.library.clone()
    }
}

impl Signal for GetAnalyzeCountRequest {
    type Params = Option<LibraryHandle>;
    type Response = GetAnalyzeCountResponse;
    async fn handle(
        &self,
        library: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let count = get_analyze_count(&library.main_db).await?;
        Ok(Some(GetAnalyzeCountResponse { count }))
    }
}
