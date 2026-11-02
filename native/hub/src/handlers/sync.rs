use std::sync::Arc;

use anyhow::Result;
use database::connection::MainDbConnection;

use crate::{
    Session, Signal,
    messages::{SetDownloadRuleRequest, SetDownloadRuleResponse},
    utils::{GlobalParams, ParamsExtractor},
};

impl ParamsExtractor for SetDownloadRuleRequest {
    type Params = Arc<MainDbConnection>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.main_db.clone()
    }
}

impl Signal for SetDownloadRuleRequest {
    type Params = Arc<MainDbConnection>;
    type Response = SetDownloadRuleResponse;

    async fn handle(
        &self,
        db: Self::Params,
        _session: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let result = database::actions::downloads::set_download_rule(
            &db,
            request
                .queries
                .iter()
                .map(|q| (q.operator.clone(), q.parameter.clone()))
                .collect(),
            request.mix_id,
            request.enabled,
        )
        .await;
        let (rule_id, error) = match result {
            Ok(id) => (id, None),
            Err(error) => (String::new(), Some(error.to_string())),
        };
        Ok(Some(SetDownloadRuleResponse {
            request_id: request.request_id.clone(),
            rule_id,
            error,
        }))
    }
}
