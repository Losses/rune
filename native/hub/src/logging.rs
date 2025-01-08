use std::sync::Arc;

use anyhow::{Context, Result};
use rinf::DartSignal;

use database::{
    actions::logging::{clear_logs, delete_log, list_log},
    connection::MainDbConnection,
};

use crate::messages::*;

pub async fn list_log_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<ListLogRequest>,
) -> Result<Option<ListLogResponse>> {
    let request = dart_signal.message;

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

pub async fn clear_log_request(
    main_db: Arc<MainDbConnection>,
    _dart_signal: DartSignal<ClearLogRequest>,
) -> Result<Option<ClearLogResponse>> {
    clear_logs(&main_db)
        .await
        .with_context(|| "Failed to clear logs")?;

    Ok(Some(ClearLogResponse { success: true }))
}

pub async fn remove_log_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<RemoveLogRequest>,
) -> Result<Option<RemoveLogResponse>> {
    let request = dart_signal.message;

    delete_log(&main_db, request.id)
        .await
        .with_context(|| format!("Failed to delete log with id={}", request.id))?;
    
    Ok(Some(RemoveLogResponse {
        id: request.id,
        success: true,
    }))
}
