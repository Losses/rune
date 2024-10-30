use database::actions::index::index_audio_library as i;
use database::connection::MainDbConnection;

pub async fn index_audio_library(main_db: &MainDbConnection) {
    match i(main_db, 50).await {
        Ok(_) => {
            println!("Audio indexing completed successfully.");
        }
        Err(e) => {
            eprintln!("Audio indexing failed: {}", e)
        }
    }
}
