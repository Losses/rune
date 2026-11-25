use crate::request_error::ExpectedRequestError;

use anyhow::{Context, Result};

use ::database::actions::logging::{LogLevel, clear_logs, delete_log, list_log};

use crate::{
    Session, Signal,
    messages::*,
    utils::{GlobalParams, LocalLibrary, ParamsExtractor},
};

impl ParamsExtractor for ListLogRequest {
    type Params = LocalLibrary;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        LocalLibrary::from_global(all_params)
    }
}

impl Signal for ListLogRequest {
    type Params = LocalLibrary;
    type Response = ListLogResponse;

    async fn handle(
        &self,
        library: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        // A service library keeps no log on this device.
        let Some(library) = library.or_service()? else {
            return Ok(Some(ListLogResponse {
                call_id: None,
                result: Vec::new(),
            }));
        };
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
                .map(|x| {
                    Ok(LogDetail {
                        id: x.id,
                        level: x.level.parse::<LogLevel>()?.into(),
                        detail: x.detail,
                        domain: x.domain,
                        date: x.date.timestamp(),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        }))
    }
}

impl ParamsExtractor for ClearLogRequest {
    type Params = LocalLibrary;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        LocalLibrary::from_global(all_params)
    }
}

impl Signal for ClearLogRequest {
    type Params = LocalLibrary;
    type Response = ClearLogResponse;
    async fn handle(
        &self,
        library: Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let Some(library) = library.or_service()? else {
            return Ok(Some(ClearLogResponse { call_id: None }));
        };
        clear_logs(&library.main_db)
            .await
            .with_context(|| "Failed to clear logs")?;

        Ok(Some(ClearLogResponse { call_id: None }))
    }
}

impl ParamsExtractor for RemoveLogRequest {
    type Params = LocalLibrary;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        LocalLibrary::from_global(all_params)
    }
}

impl Signal for RemoveLogRequest {
    type Params = LocalLibrary;
    type Response = RemoveLogResponse;
    async fn handle(
        &self,
        library: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let Some(library) = library.or_service()? else {
            return Ok(Some(RemoveLogResponse {
                call_id: None,
                id: request.id,
            }));
        };

        delete_log(&library.main_db, request.id)
            .await
            .with_context(|| format!("Failed to delete log with id={}", request.id))?;

        Ok(Some(RemoveLogResponse {
            call_id: None,
            id: request.id,
        }))
    }
}
