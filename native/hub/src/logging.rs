use std::sync::Arc;

use anyhow::{Context, Result};
use rinf::DartSignal;

use database::{actions::logging::{clear_logs, delete_log, list_log}, connection::MainDbConnection};

use crate::messages::*;

pub async fn list_log_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<ListLogRequest>,
) -> Result<()> {
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
    ListLogResponse {
        result: result
            .into_iter()
            .map(|x| LogDetail {
                level: x.level,
                detail: x.detail,
                domain: x.domain,
                date: x.date.timestamp(),
            })
            .collect(),
    }
    .send_signal_to_dart();

    Ok(())
}

pub async fn clear_log_request(
    main_db: Arc<MainDbConnection>,
    _dart_signal: DartSignal<ClearLogRequest>,
) -> Result<()> {
    clear_logs(&main_db)
        .await
        .with_context(|| "Failed to clear logs")?;
    ClearLogResponse { success: true }.send_signal_to_dart();

    Ok(())
}

pub async fn remove_log_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<RemoveLogRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    delete_log(&main_db, request.id)
        .await
        .with_context(|| format!("Failed to delete log with id={}", request.id))?;
    RemoveLogResponse {
        id: request.id,
        success: true,
    }
    .send_signal_to_dart();

    Ok(())
}

