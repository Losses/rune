use log::{error, info};
use std::path::PathBuf;
use tracing_subscriber::filter::EnvFilter;

use analysis::utils::computing_device::ComputingDevice;
use database::actions::analysis::{
    analysis_audio_library, empty_progress_callback as empty_analysis_progress_callback,
};
use database::actions::metadata::{
    empty_progress_callback as empty_scan_progress_callback, scan_audio_library,
};
use database::actions::recommendation::sync_recommendation;
use database::connection::{connect_main_db, connect_recommendation_db};

#[tokio::main]
async fn main() {
    let filter = EnvFilter::new(
        "symphonia_format_ogg=off,symphonia_core=off,sea_orm_migration::migrator=off, info",
    );

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .init();

    let path = ".";
    let main_db = connect_main_db(path, None, "").await.unwrap();

    // Get the first command line argument.
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().expect("Audio data path not provided");

    let root_path = PathBuf::from(&path);

    // Scan the audio library
    let _ = scan_audio_library(
        &main_db,
        &root_path,
        true,
        false,
        empty_scan_progress_callback,
        None,
    )
    .await;

    info!("Analyzing tracks");
    // Analyze the audio files in the database
    analysis_audio_library(
        &main_db,
        &root_path,
        10,
        ComputingDevice::Gpu,
        empty_analysis_progress_callback,
        None,
    )
    .await
    .expect("Audio analysis failed");

    info!("Syncing recommendation");
    let recommend_db = connect_recommendation_db(&path, None).unwrap();
    match sync_recommendation(&main_db, &recommend_db).await {
        Ok(_) => info!("OK!"),
        Err(e) => error!("Unable to sync recommendation: {}", e),
    };

    println!("OK");
}
