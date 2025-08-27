use std::sync::Arc;

use anyhow::{Context, Result};

use ::database::actions::analysis::{get_analyze_count, if_analyze_exists};
use ::database::connection::MainDbConnection;

use crate::utils::{GlobalParams, ParamsExtractor};
use crate::{Session, Signal, messages::*};

impl ParamsExtractor for IfAnalyzeExistsRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for IfAnalyzeExistsRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = IfAnalyzeExistsResponse;
    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let file_id = dart_signal.file_id;

        let exists = if_analyze_exists(&main_db, file_id)
            .await
            .with_context(|| format!("Failed to test if analysis exists: {file_id}"))?;
        Ok(Some(IfAnalyzeExistsResponse { file_id, exists }))
    }
}

impl ParamsExtractor for GetAnalyzeCountRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for GetAnalyzeCountRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = GetAnalyzeCountResponse;
    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let count = get_analyze_count(&main_db).await?;
        Ok(Some(GetAnalyzeCountResponse { count }))
    }
}
