use std::path::Path;

use database::actions::analysis::{analysis_audio_library, empty_progress_callback};
use database::actions::recommendation::sync_recommendation;
use database::connection::{MainDbConnection, RecommendationDbConnection};

pub async fn analyze_audio_library(
    main_db: &MainDbConnection,
    analysis_db: &RecommendationDbConnection,
    path: &Path,
) {
    if let Err(e) = analysis_audio_library(main_db, path, 10, empty_progress_callback, None).await {
        eprintln!("Audio analysis failed: {}", e);
        return;
    }

    if let Err(e) = sync_recommendation(main_db, analysis_db).await {
        eprintln!("Sync recommendation failed: {}", e);
        return;
    }

    println!("Audio analysis completed successfully.");
}
