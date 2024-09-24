use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use dunce::canonicalize;
use rinf::DartSignal;

use database::actions::file::get_files_by_ids;
use database::actions::file::list_files;
use database::actions::metadata::get_metadata_summary_by_files;
use database::actions::metadata::get_parsed_file_by_id;
use database::actions::metadata::MetadataSummary;
use sea_orm::DatabaseConnection;

use database::actions::file::get_media_files;
use database::connection::MainDbConnection;

use crate::messages;
use crate::messages::album::Album;
use crate::messages::artist::Artist;
use messages::media_file::*;

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

    let media_entries = get_media_files(
        &db,
        cursor.try_into().unwrap(),
        page_size.try_into().unwrap(),
    )
    .await?;

    let media_summaries = get_metadata_summary_by_files(&db, media_entries)
        .await
        .with_context(|| {
            format!(
                "Failed to fetch media list, page: {}, size: {}",
                cursor, page_size
            )
        })?;

    let media_files = parse_media_files(media_summaries, lib_path).await?;
    MediaFileList { media_files }.send_signal_to_dart(); // GENERATED

    Ok(())
}

pub async fn fetch_media_file_by_ids_request(
    main_db: Arc<MainDbConnection>,
    lib_path: Arc<String>,
    dart_signal: DartSignal<FetchMediaFileByIdsRequest>,
) -> Result<()> {
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

    FetchMediaFileByIdsResponse { result: items }.send_signal_to_dart();

    Ok(())
}

pub async fn fetch_parsed_media_file_request(
    db: Arc<DatabaseConnection>,
    lib_path: Arc<String>,
    dart_signal: DartSignal<FetchParsedMediaFileRequest>,
) -> Result<()> {
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
    .send_signal_to_dart();

    Ok(())
}

pub async fn search_media_file_summary_request(
    main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<SearchMediaFileSummaryRequest>,
) -> Result<()> {
    let request = dart_signal.message;

    let items = list_files(&main_db, request.n.try_into().unwrap())
        .await
        .with_context(|| "Failed to search media file summary")?;

    let media_summaries = get_metadata_summary_by_files(&main_db, items)
        .await
        .with_context(|| "Failed to get media summaries")?;

    SearchMediaFileSummaryResponse {
        result: media_summaries
            .into_iter()
            .map(|x| MediaFileSummary {
                id: x.id,
                name: x.title,
            })
            .collect(),
    }
    .send_signal_to_dart();

    Ok(())
}
