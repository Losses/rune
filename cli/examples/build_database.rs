use std::{path::PathBuf, sync::Arc};

use log::{error, info};
use tracing_subscriber::filter::EnvFilter;

use analysis::utils::computing_device::ComputingDevice;
use database::{
    actions::{
        analysis::{
            analysis_audio_library, empty_progress_callback as empty_analysis_progress_callback,
        },
        metadata::{empty_progress_callback as empty_scan_progress_callback, scan_audio_library},
        recommendation::sync_recommendation,
    },
    connection::{connect_main_db, connect_recommendation_db},
};
use fsio::FsIo;

#[tokio::main]
async fn main() {
    let filter = EnvFilter::new(
        "symphonia_format_ogg=off,symphonia_core=off,sea_orm_migration::migrator=off, info",
    );

    let fsio = Arc::new(FsIo::new());

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .init();

    let path = ".";
    let main_db = connect_main_db(&fsio, path, None, "").await.unwrap();

    // Get the first command line argument.
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().expect("Audio data path not provided");

    let root_path = PathBuf::from(&path);
    let node_id = "RUNE_EXAMPLE";

    // Scan the audio library
    let _ = scan_audio_library(
        &fsio,
        &main_db,
        node_id,
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
        Arc::clone(&fsio),
        &main_db,
        &root_path,
        "",
        10,
        ComputingDevice::Gpu,
        empty_analysis_progress_callback,
        None,
    )
    .await
    .expect("Audio analysis failed");

    info!("Syncing recommendation");
    let recommend_db = connect_recommendation_db(&fsio, &path, None).await.unwrap();
    match sync_recommendation(&main_db, &recommend_db).await {
        Ok(_) => info!("OK!"),
        Err(e) => error!("Unable to sync recommendation: {e}"),
    };

    println!("OK");
}
