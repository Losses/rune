use database::actions::index::index_audio_library as i;
use database::connection::{MainDbConnection, SearchDbConnection};

pub async fn index_audio_library(main_db: &MainDbConnection, search_db: &mut SearchDbConnection) {
    match i(main_db, search_db, 50).await {
        Ok(_) => {
            println!("Audio indexing completed successfully.");
        }
        Err(e) => {
            eprintln!("Audio indexing failed: {}", e)
        }
    }
}
