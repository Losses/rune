use log::debug;
use rinf::DartSignal;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use database::actions::metadata::scan_audio_library;
use database::connection::{MainDbConnection, SearchDbConnection};

use crate::messages::library_manage::{
    ScanAudioLibraryProgress, ScanAudioLibraryRequest, ScanAudioLibraryResponse,
};
use crate::{CloseLibraryRequest, CloseLibraryResponse};

pub async fn close_library_request(
    lib_path: Arc<String>,
    cancel_token: Arc<CancellationToken>,
    dart_signal: DartSignal<CloseLibraryRequest>,
) {
    let request = dart_signal.message;

    debug!("Closing library");

    if request.path != *lib_path {
        return;
    }

    cancel_token.cancel();

    CloseLibraryResponse {
        path: request.path.clone(),
    }
    .send_signal_to_dart()
}

pub async fn scan_audio_library_request(
    main_db: Arc<MainDbConnection>,
    search_db: Arc<Mutex<SearchDbConnection>>,
    cancel_token: Arc<CancellationToken>,
    dart_signal: DartSignal<ScanAudioLibraryRequest>,
) {
    let request = dart_signal.message;

    debug!("Scanning library summary");

    let mut search_db = search_db.lock().await;

    scan_audio_library(
        &main_db,
        &mut search_db,
        Path::new(&request.path),
        true,
        |progress| {
            ScanAudioLibraryProgress {
                path: request.path.clone(),
                progress: progress.try_into().unwrap(),
            }
            .send_signal_to_dart()
        },
        Some((*cancel_token).clone()),
    )
    .await;

    ScanAudioLibraryResponse {
        path: request.path.clone(),
    }
    .send_signal_to_dart()
}
