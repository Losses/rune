use crate::request_error::ExpectedRequestError;

use anyhow::{Context, Result};

use ::database::actions::logging::{clear_logs, delete_log, list_log};

use crate::{
    Session, Signal,
    library_pool::LibraryHandle,
    messages::*,
    utils::{GlobalParams, ParamsExtractor, require_library},
};

impl ParamsExtractor for ListLogRequest {
    type Params = Option<LibraryHandle>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.library.clone()
    }
}

impl Signal for ListLogRequest {
    type Params = Option<LibraryHandle>;
    type Response = ListLogResponse;

    async fn handle(
        &self,
        library: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;

        let result = list_log(
            &library.main_db,
            request.cursor.try_into().map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid cursor: {error}"))
            })?,
            request.page_size.try_into().map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid page size: {error}"))
            })?,
        )
        .await
        .with_context(|| {
            format!(
                "Failed to list log: cursor={}, page_size={}",
                request.cursor, request.page_size
            )
        })?;

        Ok(Some(ListLogResponse {
            call_id: None,
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
    type Params = Option<LibraryHandle>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.library.clone()
    }
}

impl Signal for ClearLogRequest {
    type Params = Option<LibraryHandle>;
    type Response = ClearLogResponse;
    async fn handle(
        &self,
        library: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        clear_logs(&library.main_db)
            .await
            .with_context(|| "Failed to clear logs")?;

        Ok(Some(ClearLogResponse {
            call_id: None,
            success: true,
        }))
    }
}

impl ParamsExtractor for RemoveLogRequest {
    type Params = Option<LibraryHandle>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.library.clone()
    }
}

impl Signal for RemoveLogRequest {
    type Params = Option<LibraryHandle>;
    type Response = RemoveLogResponse;
    async fn handle(
        &self,
        library: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let request = dart_signal;

        delete_log(&library.main_db, request.id)
            .await
            .with_context(|| format!("Failed to delete log with id={}", request.id))?;

        Ok(Some(RemoveLogResponse {
            call_id: None,
            id: request.id,
            success: true,
        }))
    }
}
