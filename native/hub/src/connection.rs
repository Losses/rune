use crate::common::*;
use crate::messages;
use log::info;
use tokio::sync::Mutex;

lazy_static::lazy_static! {
    static ref MEDIA_LIBRARY_PATH: Mutex<Option<String>> = Mutex::new(None);
}

pub async fn receive_media_library_path() -> Result<()> {
    use messages::connection::*;

    let mut receiver = MediaLibraryPath::get_dart_signal_receiver()?; // GENERATED
    while let Some(dart_signal) = receiver.recv().await {
        let media_library_path = dart_signal.message.path;

        info!("Received path: {}", media_library_path);
        let mut path_guard = MEDIA_LIBRARY_PATH.lock().await;
        *path_guard = Some(media_library_path);
    }

    Ok(())
}

// Function to get the media library path
pub async fn get_media_library_path() -> Option<String> {
    let path_guard = MEDIA_LIBRARY_PATH.lock().await;
    path_guard.clone()
}
