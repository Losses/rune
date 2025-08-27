use std::sync::Arc;

use anyhow::{Context, Result};

use ::database::{
    actions::logging::{clear_logs, delete_log, list_log},
    connection::MainDbConnection,
};

use crate::{
    Session, Signal,
    messages::*,
    utils::{GlobalParams, ParamsExtractor},
};

impl ParamsExtractor for ListLogRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for ListLogRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = ListLogResponse;

    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let result = list_log(
            &main_db,
            request.cursor.try_into()?,
            request.page_size.try_into()?,
        )
        .await
        .with_context(|| {
            format!(
                "Failed to list log: cursor={}, page_size={}",
                request.cursor, request.page_size
            )
        })?;

        Ok(Some(ListLogResponse {
            result: result
                .into_iter()
                .map(|x| LogDetail {
                    id: x.id,
                    level: x.level,
                    detail: x.detail,
                    domain: x.domain,
                    date: x.date.timestamp(),
                })
                .collect(),
        }))
    }
}

impl ParamsExtractor for ClearLogRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for ClearLogRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = ClearLogResponse;
    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        clear_logs(&main_db)
            .await
            .with_context(|| "Failed to clear logs")?;

        Ok(Some(ClearLogResponse { success: true }))
    }
}

impl ParamsExtractor for RemoveLogRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for RemoveLogRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = RemoveLogResponse;
    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        delete_log(&main_db, request.id)
            .await
            .with_context(|| format!("Failed to delete log with id={}", request.id))?;

        Ok(Some(RemoveLogResponse {
            id: request.id,
            success: true,
        }))
    }
}
