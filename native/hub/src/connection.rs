use std::sync::Arc;

use anyhow::{Context, Result};
use log::{error, info};

use database::connection::{
    connect_main_db, connect_recommendation_db, MainDbConnection, RecommendationDbConnection,
};

use crate::messages::*;

pub struct DatabaseConnections {
    pub main_db: Arc<MainDbConnection>,
    pub recommend_db: Arc<RecommendationDbConnection>,
}

async fn initialize_databases(path: &str) -> Result<DatabaseConnections> {
    info!("Initializing databases");

    let main_db = connect_main_db(path)
        .await
        .with_context(|| "Failed to connect to main DB")?;

    let recommend_db = connect_recommendation_db(path)
        .with_context(|| "Failed to connect to recommendation DB")?;

    Ok(DatabaseConnections {
        main_db: Arc::new(main_db),
        recommend_db: Arc::new(recommend_db),
    })
}

pub async fn receive_media_library_path<F, Fut>(main_loop: F) -> Result<()>
where
    F: Fn(String, DatabaseConnections) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = ()> + Send,
{
    let receiver = SetMediaLibraryPathRequest::get_dart_signal_receiver();

    loop {
        while let Some(dart_signal) = receiver.recv().await {
            let media_library_path = dart_signal.message.path;
            info!("Received path: {}", media_library_path);

            // Initialize databases
            match initialize_databases(&media_library_path).await {
                Ok(db_connections) => {
                    // Send success response to Dart
                    SetMediaLibraryPathResponse {
                        path: media_library_path.clone(),
                        success: true,
                        error: None,
                    }
                    .send_signal_to_dart();

                    // Continue with main loop
                    main_loop(media_library_path, db_connections).await;
                }
                Err(e) => {
                    error!("Database initialization failed: {}", e);
                    // Send error response to Dart
                    SetMediaLibraryPathResponse {
                        path: media_library_path,
                        success: false,
                        error: Some(format!("{:#?}", e)),
                    }
                    .send_signal_to_dart();
                }
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}
