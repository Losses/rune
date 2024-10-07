use std::sync::Arc;

use anyhow::{Context, Result};
use rinf::DartSignal;

use database::actions::analysis::{get_analyse_count, if_analyse_exists};
use database::connection::MainDbConnection;

use crate::messages::*;

pub async fn if_analyse_exists_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<IfAnalyseExistsRequest>,
) -> Result<()> {
    let file_id = dart_signal.message.file_id;

    let exists = if_analyse_exists(&main_db, file_id)
        .await
        .with_context(|| format!("Failed to test if analysis exists: {}", file_id))?;
    IfAnalyseExistsResponse { file_id, exists }.send_signal_to_dart();

    Ok(())
}

pub async fn get_analyse_count_request(
    main_db: Arc<MainDbConnection>,
    _dart_signal: DartSignal<GetAnalyseCountRequest>,
) -> Result<()> {
    GetAnalyseCountResponse {
        count: get_analyse_count(&main_db).await?,
    }
    .send_signal_to_dart();

    Ok(())
}
