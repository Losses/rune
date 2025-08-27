use database::actions::index::index_audio_library as i;
use database::connection::MainDbConnection;

pub async fn index_audio_library(main_db: &MainDbConnection, node_id: &str) {
    match i(main_db, node_id, 50, None).await {
        Ok(_) => {
            println!("Audio indexing completed successfully.");
        }
        Err(e) => {
            eprintln!("Audio indexing failed: {e}")
        }
    }
}
