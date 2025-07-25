use std::path::Path;

use analysis::utils::computing_device::ComputingDevice;
use database::actions::analysis::{analysis_audio_library, empty_progress_callback};
use database::actions::recommendation::sync_recommendation;
use database::connection::{MainDbConnection, RecommendationDbConnection};

pub async fn analyze_audio_library(
    computing_device: ComputingDevice,
    main_db: &MainDbConnection,
    analysis_db: &RecommendationDbConnection,
    path: &Path,
    node_id: &str,
) {
    if let Err(e) = analysis_audio_library(
        main_db,
        path,
        node_id,
        15,
        computing_device,
        empty_progress_callback,
        None,
    )
    .await
    {
        eprintln!("Audio analysis failed: {}", e);
        return;
    }

    print!("Analysis finished");

    if let Err(e) = sync_recommendation(main_db, analysis_db).await {
        eprintln!("Sync recommendation failed: {}", e);
        return;
    }

    print!("Sync finished");

    println!("Audio analysis completed successfully");
}
