use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use dunce::canonicalize;
use log::error;
use rinf::DartSignal;
use sea_orm::DatabaseConnection;

use database::actions::cover_art::bake_cover_art_by_file_ids;
use database::actions::cover_art::bake_cover_art_by_media_files;
use database::actions::file::get_files_by_ids;
use database::actions::file::get_media_files;
use database::actions::file::list_files;
use database::actions::metadata::get_metadata_summary_by_files;
use database::actions::metadata::get_parsed_file_by_id;
use database::actions::metadata::MetadataSummary;
use database::connection::MainDbConnection;

use crate::messages::*;

pub async fn parse_media_files(
    media_summaries: Vec<MetadataSummary>,
    lib_path: Arc<String>,
) -> Result<Vec<MediaFile>> {
    let mut media_files = Vec::with_capacity(media_summaries.len());

    for file in media_summaries {
        let media_path = canonicalize(
            Path::new(lib_path.as_ref())
                .join(&file.directory)
                .join(&file.file_name),
        );

        match media_path {
            Ok(media_path) => {
                let media_file = MediaFile {
                    id: file.id,
                    path: media_path
                        .to_str()
                        .ok_or_else(|| anyhow::anyhow!("Media path is None"))?
                        .to_string(),
                    artist: if file.artist.is_empty() {
                        "Unknown Artist".to_owned()
                    } else {
                        file.artist
                    },
                    album: if file.album.is_empty() {
                        "Unknown Album".to_owned()
                    } else {
                        file.album
                    },
                    title: if file.title.is_empty() {
                        file.file_name
                    } else {
                        file.title
                    },
                    duration: file.duration,
                    cover_art_id: file.cover_art_id.unwrap_or(-1),
                    track_number: file.track_number,
                };

                media_files.push(media_file);
            }
            Err(e) => {
                error!("{:?}", e);
            }
        }
    }

    Ok(media_files)
}

pub async fn fetch_media_files_request(
    main_db: Arc<DatabaseConnection>,
    lib_path: Arc<String>,
    dart_signal: DartSignal<FetchMediaFilesRequest>,
) -> Result<Option<FetchMediaFilesResponse>> {
    let request = dart_signal.message;
    let cursor = request.cursor;
    let page_size = request.page_size;

    let media_entries =
        get_media_files(&main_db, cursor.try_into()?, page_size.try_into()?).await?;

    let cover_art_map = if request.bake_cover_arts {
        bake_cover_art_by_media_files(&main_db, media_entries.clone()).await?
    } else {
        HashMap::new()
    };

    let media_summaries = get_metadata_summary_by_files(&main_db, media_entries)
        .await
        .with_context(|| {
            format!(
                "Failed to fetch media list, page: {}, size: {}",
                cursor, page_size
            )
        })?;

    let media_files = parse_media_files(media_summaries, lib_path).await?;
    Ok(Some(FetchMediaFilesResponse {
        media_files,
        cover_art_map,
    }))
}

pub async fn fetch_media_file_by_ids_request(
    main_db: Arc<MainDbConnection>,
    lib_path: Arc<String>,
    dart_signal: DartSignal<FetchMediaFileByIdsRequest>,
) -> Result<Option<FetchMediaFileByIdsResponse>> {
    let request = dart_signal.message;

    let media_entries = get_files_by_ids(&main_db, &request.ids)
        .await
        .with_context(|| format!("Failed to get media summaries for id: {:?}", request.ids))?;

    let media_summaries = get_metadata_summary_by_files(&main_db, media_entries)
        .await
        .with_context(|| "Unable to get media summaries")?;

    let items = parse_media_files(media_summaries, lib_path)
        .await
        .with_context(|| "Failed to parse media summaries")?;

    let cover_art_map = if request.bake_cover_arts {
        bake_cover_art_by_file_ids(&main_db, request.ids).await?
    } else {
        HashMap::new()
    };

    Ok(Some(FetchMediaFileByIdsResponse {
        media_files: items,
        cover_art_map,
    }))
}

pub async fn fetch_parsed_media_file_request(
    db: Arc<DatabaseConnection>,
    lib_path: Arc<String>,
    dart_signal: DartSignal<FetchParsedMediaFileRequest>,
) -> Result<Option<FetchParsedMediaFileResponse>> {
    let file_id = dart_signal.message.id;

    let (media_file, artists, album) = get_parsed_file_by_id(&db, file_id)
        .await
        .with_context(|| "Failed to get media summaries")?;

    let parsed_files = parse_media_files(vec![media_file], lib_path.clone())
        .await
        .with_context(|| "Failed to parse media files")?;

    let media_file = parsed_files
        .first()
        .ok_or_else(|| anyhow!("Parsed Files not found for file_id: {}", file_id))
        .with_context(|| "Failed to get media file")?;

    let album = album
        .ok_or(anyhow!("Parsed album not found for file_id: {}", file_id))
        .with_context(|| "Failed to query album")?;

    Ok(Some(FetchParsedMediaFileResponse {
        file: Some(media_file.clone()),
        artists: artists
            .into_iter()
            .map(|x| Artist {
                id: x.id,
                name: x.name,
            })
            .collect(),
        album: Some(Album {
            id: album.id,
            name: album.name,
        }),
    }))
}

pub async fn search_media_file_summary_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<SearchMediaFileSummaryRequest>,
) -> Result<Option<SearchMediaFileSummaryResponse>> {
    let request = dart_signal.message;

    let items = list_files(&main_db, request.n.try_into()?)
        .await
        .with_context(|| "Failed to search media file summary")?;

    let media_summaries = get_metadata_summary_by_files(&main_db, items)
        .await
        .with_context(|| "Failed to get media summaries")?;

    Ok(Some(SearchMediaFileSummaryResponse {
        result: media_summaries
            .into_iter()
            .map(|x| MediaFileSummary {
                id: x.id,
                name: if x.title.is_empty() {
                    x.file_name
                } else {
                    x.title
                },
                cover_art_id: x.cover_art_id.unwrap_or(-1),
            })
            .collect(),
    }))
}
