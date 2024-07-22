use std::path::Path;
use std::sync::Arc;

use sea_orm::DatabaseConnection;

use database::actions::file::get_media_files;

use crate::common::*;
use crate::messages;

pub async fn fetch_media_files(db: Arc<DatabaseConnection>) -> Result<()> {
    use messages::media_file::*;

    let mut receiver = FetchMediaFiles::get_dart_signal_receiver()?; // GENERATED

    while let Some(dart_signal) = receiver.recv().await {
        let fetch_media_files = dart_signal.message;
        let page_key = fetch_media_files.page_key;
        let page_size = fetch_media_files.page_size;

        let media_entries = get_media_files(
            &db,
            page_key.try_into().unwrap(),
            page_size.try_into().unwrap(),
        )
        .await?;

        let media_files = media_entries
            .into_iter()
            .map(|media_file| {
                let media_path = Path::new(&media_file.directory).join(media_file.file_name);

                MediaFile {
                    id: media_file.id,
                    path: media_path.to_str().unwrap().to_string(),
                }
            })
            .collect();

        MediaFileList { media_files }.send_signal_to_dart(); // GENERATED
    }

    Ok(())
}
