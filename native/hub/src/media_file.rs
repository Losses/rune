use dunce::canonicalize;
use log::{error, info};
use std::path::Path;
use std::sync::Arc;

use database::actions::metadata::get_metadata_summary_by_file_ids;
use sea_orm::DatabaseConnection;

use database::actions::file::get_media_files;

use crate::common::*;
use crate::messages;

pub async fn fetch_media_files(db: Arc<DatabaseConnection>, lib_path: Arc<String>) -> Result<()> {
    use messages::media_file::*;

    let mut receiver = FetchMediaFiles::get_dart_signal_receiver()?; // GENERATED

    while let Some(dart_signal) = receiver.recv().await {
        let fetch_media_files = dart_signal.message;
        let cursor = fetch_media_files.cursor;
        let page_size = fetch_media_files.page_size;

        info!(
            "Fetching media list, page: {}, size: {}",
            cursor, page_size
        );

        let media_entries = get_media_files(
            &db,
            cursor.try_into().unwrap(),
            page_size.try_into().unwrap(),
        )
        .await?;

        let media_summaries = get_metadata_summary_by_file_ids(
            &db,
            media_entries.into_iter().map(|x| x.id).collect(),
        );

        match media_summaries.await {
            Ok(media_summaries) => {
                let media_files = media_summaries
                    .into_iter()
                    .map(|file| {
                        let media_path = canonicalize(
                            Path::new(lib_path.as_ref().as_str())
                                .join(file.directory.clone())
                                .join(file.file_name.clone()),
                        );

                        MediaFile {
                            id: file.id,
                            path: media_path.unwrap().to_str().unwrap().to_string(),
                            artist: file.artist,
                            album: file.album,
                            title: file.title,
                            duration: file.duration,
                        }
                    })
                    .collect();

                MediaFileList { media_files }.send_signal_to_dart(); // GENERATED
            }
            Err(e) => {
                error!("Error happened while getting media summaries: {:#?}", e)
            }
        }
    }

    Ok(())
}
