use anyhow::Result;
use log::info;

use crate::messages;

pub async fn receive_media_library_path<F, Fut>(main_loop: F) -> Result<()>
where
    F: Fn(String) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = ()> + Send,
{
    use messages::connection::*;

    let mut receiver = MediaLibraryPath::get_dart_signal_receiver()?; // GENERATED

    loop {
        while let Some(dart_signal) = receiver.recv().await {
            let media_library_path = dart_signal.message.path;

            info!("Received path: {}", media_library_path);

            main_loop(media_library_path).await;
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}
