use database::{
    actions::index::index_audio_library as index_files, connection::MainDbConnection,
    sync::SeaOrmSyncStore,
};

pub async fn index_audio_library(
    main_db: &MainDbConnection,
    node_id: &str,
    store: &SeaOrmSyncStore,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    index_files(main_db, node_id, 50, None, store).await?;
    println!("Audio indexing completed successfully.");
    Ok(())
}
