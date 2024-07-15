use std::path::PathBuf;
use tracing_subscriber::filter::EnvFilter;

use database::actions::analysis::analysis_audio_library;
use database::actions::metadata::scan_audio_library;
use database::actions::recommendation::sync_recommendation;
use database::connection::{connect_main_db, connect_recommendation_db};

#[tokio::main]
async fn main() {
    // std::env::set_var("RUST_LOG", "debug");
    let filter = EnvFilter::new("symphonia_format_ogg=off,symphonia_core=off,sea_orm_migration::migrator=off, info"); // ,debug

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .init();

    let path = ".";
    let main_db = connect_main_db(path).await.unwrap();

    // Get the first command line argument.
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().expect("Audio data path not provided");

    let root_path = PathBuf::from(&path);

    // Scan the audio library
    scan_audio_library(&main_db, &root_path, true).await;

    // Analyze the audio files in the database
    analysis_audio_library(&main_db, &root_path, 10)
        .await
        .expect("Audio analysis failed");

    let analysis_db = connect_recommendation_db(&path).unwrap();
    let _ = sync_recommendation(&main_db, &analysis_db).await;

    println!("OK");
}
