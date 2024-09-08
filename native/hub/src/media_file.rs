use database::connection::MainDbConnection;
use dunce::canonicalize;
use log::debug;
use log::{error, info};
use rinf::DartSignal;
use std::path::Path;
use std::sync::Arc;

use database::actions::file::{compound_query_media_files, get_files_by_ids};
use database::actions::metadata::get_metadata_summary_by_files;
use database::actions::metadata::get_parsed_file_by_id;
use database::actions::metadata::MetadataSummary;
use sea_orm::DatabaseConnection;

use database::actions::file::get_media_files;

use crate::common::*;
use crate::messages;
use crate::messages::album::Album;
use crate::messages::artist::Artist;
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

pub async fn fetch_media_file_by_ids_request(
    main_db: Arc<MainDbConnection>,
    lib_path: Arc<String>,
    dart_signal: DartSignal<FetchMediaFileByIdsRequest>,
) {
    let request = dart_signal.message;

    debug!("Requesting media files: {:#?}", request.ids);

    match get_files_by_ids(&main_db, &request.ids).await {
        Ok(media_entries) => {
            let media_summaries = get_metadata_summary_by_files(&main_db, media_entries);

            match media_summaries.await {
                Ok(media_summaries) => {
                    let items = parse_media_files(media_summaries, lib_path).await;

                    match items {
                        Ok(items) => {
                            FetchMediaFileByIdsResponse { result: items }.send_signal_to_dart();
                        }
                        Err(e) => {
                            error!("Error happened while parsing media summaries: {:#?}", e)
                        }
                    }
                }
                Err(e) => {
                    error!("Error happened while getting media summaries: {:#?}", e)
                }
            }
        }
        Err(e) => {
            error!("Failed to fetch albums groups: {}", e);
        }
    };
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
    let playlist_ids = query_media_files.playlist_ids;

    info!(
        "Compound query media list with artist_ids: {:?}, album_ids: {:?}, playlist_ids: {:?}, page: {}, size: {}",
        artist_ids, album_ids, playlist_ids, cursor, page_size
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

    let playlist_ids_option = if playlist_ids.is_empty() {
        None
    } else {
        Some(playlist_ids)
    };

    let media_entries = compound_query_media_files(
        &db,
        artist_ids_option,
        album_ids_option,
        playlist_ids_option,
        None,
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

pub async fn fetch_parsed_media_file_request(
    db: Arc<DatabaseConnection>,
    lib_path: Arc<String>,
    dart_signal: DartSignal<FetchParsedMediaFileRequest>,
) -> Result<()> {
    let file_id = dart_signal.message.id;

    match get_parsed_file_by_id(&db, file_id).await {
        Ok((media_file, artists, album)) => {
            let parsed_media_file = parse_media_files(vec![media_file], lib_path.clone());

            match parsed_media_file.await {
                Ok(parsed_files) => {
                    if let Some(media_file) = parsed_files.first() {
                        if let Some(album) = album {
                            FetchParsedMediaFileResponse {
                                file: Some(media_file.clone()),
                                artists: artists
                                    .into_iter()
                                    .map(|x| Artist {
                                        id: x.id,
                                        name: x.name,
                                        cover_ids: [].to_vec(),
                                    })
                                    .collect(),
                                album: Some(Album {
                                    id: album.id,
                                    name: album.name,
                                    cover_ids: [].to_vec(),
                                }),
                            }
                            .send_signal_to_dart(); // GENERATED
                        } else {
                            error!("Album not found for file_id: {}", file_id);
                        }
                    } else {
                        error!("Parsed media file not found for file_id: {}", file_id);
                    }
                }
                Err(e) => {
                    error!("Error happened while parsing media files: {:#?}", e);
                }
            }
        }
        Err(e) => {
            error!("Error happened while getting media summaries: {:#?}", e);
        }
    };
    Ok(())
}
