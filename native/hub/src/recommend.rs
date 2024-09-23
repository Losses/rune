use std::sync::Arc;

use anyhow::Result;
use log::error;
use rinf::DartSignal;

use crate::{IfAnalysisExistsRequest, IfAnalysisExistsResponse};
use database::actions::analysis::if_analysis_exists;
use database::connection::MainDbConnection;

pub async fn if_analysis_exists_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<IfAnalysisExistsRequest>,
) -> Result<()> {
    let file_id = dart_signal.message.file_id;

    match if_analysis_exists(&main_db, file_id).await {
        Ok(exists) => IfAnalysisExistsResponse { file_id, exists }.send_signal_to_dart(),
        Err(_) => {
            error!("Failed to test if analysis exists: {}", file_id)
        }
    };

    Ok(())
}
