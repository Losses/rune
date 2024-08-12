use database::actions::metadata::get_metadata_summary_by_files;
use dunce::canonicalize;
use log::{error, info};
use rinf::DartSignal;
use std::path::Path;
use std::sync::Arc;

use database::actions::file::compound_query_media_files;
use database::actions::metadata::MetadataSummary;
use sea_orm::DatabaseConnection;

use database::actions::file::get_media_files;

use crate::common::*;
use crate::messages;
use messages::media_file::*;

async fn parse_media_files(
    media_summaries: Vec<MetadataSummary>,
    lib_path: Arc<String>,
) -> Result<Vec<MediaFile>> {
    let mut media_files = Vec::with_capacity(media_summaries.len());

    for file in media_summaries {
        let media_path = canonicalize(
            Path::new(lib_path.as_ref())
                .join(&file.directory)
                .join(&file.file_name),
        )?;

        let media_file = MediaFile {
            id: file.id,
            path: media_path.to_str().unwrap().to_string(),
            artist: file.artist,
            album: file.album,
            title: file.title,
            duration: file.duration,
        };

        media_files.push(media_file);
    }

    Ok(media_files)
}

pub async fn fetch_media_files_request(
    db: Arc<DatabaseConnection>,
    lib_path: Arc<String>,
    dart_signal: DartSignal<FetchMediaFilesRequest>,
) -> Result<()> {
    let fetch_media_files = dart_signal.message;
    let cursor = fetch_media_files.cursor;
    let page_size = fetch_media_files.page_size;

    info!("Fetching media list, page: {}, size: {}", cursor, page_size);

    let media_entries = get_media_files(
        &db,
        cursor.try_into().unwrap(),
        page_size.try_into().unwrap(),
    )
    .await?;

    let media_summaries = get_metadata_summary_by_files(&db, media_entries);

    match media_summaries.await {
        Ok(media_summaries) => {
            let media_files = parse_media_files(media_summaries, lib_path).await?;
            MediaFileList { media_files }.send_signal_to_dart(); // GENERATED
        }
        Err(e) => {
            error!("Error happened while getting media summaries: {:#?}", e)
        }
    }

    Ok(())
}

pub async fn compound_query_media_files_request(
    db: Arc<DatabaseConnection>,
    lib_path: Arc<String>,
    dart_signal: DartSignal<CompoundQueryMediaFilesRequest>,
) -> Result<()> {
    let query_media_files = dart_signal.message;
    let cursor = query_media_files.cursor;
    let page_size = query_media_files.page_size;
    let artist_ids = query_media_files.artist_ids;
    let album_ids = query_media_files.album_ids;

    info!(
        "Compound query media list with artist_ids: {:?}, album_ids: {:?}, page: {}, size: {}",
        artist_ids, album_ids, cursor, page_size
    );

    let artist_ids_option = if artist_ids.is_empty() {
        None
    } else {
        Some(artist_ids)
    };

    let album_ids_option = if album_ids.is_empty() {
        None
    } else {
        Some(album_ids)
    };

    let media_entries = compound_query_media_files(
        &db,
        artist_ids_option,
        album_ids_option,
        cursor.try_into().unwrap(),
        page_size.try_into().unwrap(),
    )
    .await?;

    let media_summaries = get_metadata_summary_by_files(&db, media_entries);

    match media_summaries.await {
        Ok(media_summaries) => {
            let media_files = parse_media_files(media_summaries, lib_path).await?;
            CompoundQueryMediaFilesResponse { media_files }.send_signal_to_dart();
            // GENERATED
        }
        Err(e) => {
            error!("Error happened while getting media summaries: {:#?}", e)
        }
    }

    Ok(())
}
