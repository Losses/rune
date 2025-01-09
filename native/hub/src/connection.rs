use std::sync::Arc;

use anyhow::{Context, Result};
use log::{error, info};

use database::connection::{
    check_library_state, connect_main_db, connect_recommendation_db, create_redirect, LibraryState,
    MainDbConnection, RecommendationDbConnection,
};
use rinf::DartSignal;
use scrobbling::manager::ScrobblingManager;
use tokio::sync::Mutex;

use crate::messages::*;

pub struct DatabaseConnections {
    pub main_db: Arc<MainDbConnection>,
    pub recommend_db: Arc<RecommendationDbConnection>,
}

async fn initialize_databases(path: &str, db_path: Option<&str>) -> Result<DatabaseConnections> {
    info!("Initializing databases");

    let main_db = connect_main_db(path, db_path)
        .await
        .with_context(|| "Failed to connect to main DB")?;

    let recommend_db = connect_recommendation_db(path, db_path)
        .with_context(|| "Failed to connect to recommendation DB")?;

    Ok(DatabaseConnections {
        main_db: Arc::new(main_db),
        recommend_db: Arc::new(recommend_db),
    })
}

pub async fn test_library_initialized_request(
    _main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<TestLibraryInitializedRequest>,
) -> Result<Option<TestLibraryInitializedResponse>> {
    let media_library_path = dart_signal.message.path;
    let test_result = check_library_state(&media_library_path);

    let result = match test_result {
        Ok(state) => match &state {
            LibraryState::Uninitialized => TestLibraryInitializedResponse {
                path: media_library_path.clone(),
                success: true,
                error: None,
                not_ready: true,
            },

            LibraryState::Initialized(_) => TestLibraryInitializedResponse {
                path: media_library_path.clone(),
                success: true,
                error: None,
                not_ready: false,
            },
        },
        Err(e) => TestLibraryInitializedResponse {
            path: media_library_path.clone(),
            success: false,
            error: Some(format!("{:#?}", e)),
            not_ready: false,
        },
    };

    Ok(Some(result))
}

pub async fn receive_media_library_path<F, Fut>(
    main_loop: F,
    scrobbler: Arc<Mutex<ScrobblingManager>>,
) -> Result<()>
where
    F: Fn(String, DatabaseConnections, Arc<Mutex<ScrobblingManager>>) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = ()> + Send,
{
    let receiver = SetMediaLibraryPathRequest::get_dart_signal_receiver();

    loop {
        while let Some(dart_signal) = receiver.recv().await {
            let media_library_path = dart_signal.message.path;
            let database_path = dart_signal.message.db_path;
            let database_mode = dart_signal.message.mode;
            info!("Received path: {}", media_library_path);

            let library_test = match check_library_state(&media_library_path) {
                Ok(x) => x,
                Err(e) => {
                    SetMediaLibraryPathResponse {
                        path: media_library_path.clone(),
                        success: false,
                        error: Some(format!("{:#?}", e)),
                        not_ready: false,
                    }
                    .send_signal_to_dart();
                    continue;
                }
            };

            if database_mode.is_none() {
                match &library_test {
                    LibraryState::Uninitialized => {
                        SetMediaLibraryPathResponse {
                            path: media_library_path.clone(),
                            success: false,
                            error: None,
                            not_ready: true,
                        }
                        .send_signal_to_dart();
                        continue;
                    }
                    LibraryState::Initialized(_) => {}
                }
            }

            if let Some(mode) = database_mode {
                if mode == 1 {
                    if let Err(e) = create_redirect(&media_library_path) {
                        SetMediaLibraryPathResponse {
                            path: media_library_path.clone(),
                            success: false,
                            error: Some(format!("{:#?}", e)),
                            not_ready: false,
                        }
                        .send_signal_to_dart();
                        continue;
                    }
                }
            }

            // Initialize databases
            match initialize_databases(&media_library_path, Some(&database_path)).await {
                Ok(db_connections) => {
                    // Send success response to Dart
                    SetMediaLibraryPathResponse {
                        path: media_library_path.clone(),
                        success: true,
                        error: None,
                        not_ready: false,
                    }
                    .send_signal_to_dart();

                    // Clone the Arc for this iteration
                    let scrobbler_clone = Arc::clone(&scrobbler);

                    // Continue with main loop
                    main_loop(media_library_path, db_connections, scrobbler_clone).await;
                }
                Err(e) => {
                    error!("Database initialization failed: {:#?}", e);
                    // Send error response to Dart
                    SetMediaLibraryPathResponse {
                        path: media_library_path,
                        success: false,
                        error: Some(format!("{:#?}", e)),
                        not_ready: false,
                    }
                    .send_signal_to_dart();
                }
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}
