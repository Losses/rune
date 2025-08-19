use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use sea_orm::DatabaseConnection;

use ::database::{
    actions::{
        cover_art::{bake_cover_art_by_file_ids, bake_cover_art_by_media_files},
        file::{get_files_by_ids, get_media_files, list_files},
        metadata::{get_metadata_summary_by_files, get_parsed_file_by_id},
    },
    connection::MainDbConnection,
};
use ::fsio::FsIo;

use crate::{
    Session, Signal,
    messages::*,
    utils::{GlobalParams, ParamsExtractor, parse_media_files},
};

impl ParamsExtractor for FetchMediaFilesRequest {
    type Params = (Arc<FsIo>, Arc<MainDbConnection>, Arc<String>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.fsio),
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.lib_path),
        )
    }
}

impl Signal for FetchMediaFilesRequest {
    type Params = (Arc<FsIo>, Arc<DatabaseConnection>, Arc<String>);
    type Response = FetchMediaFilesResponse;

    async fn handle(
        &self,
        (fsio, main_db, lib_path): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let cursor = request.cursor;
        let page_size = request.page_size;

        let media_entries =
            get_media_files(&main_db, cursor.try_into()?, page_size.try_into()?).await?;

        let cover_art_map = if request.bake_cover_arts {
            bake_cover_art_by_media_files(&fsio, &main_db, media_entries.clone()).await?
        } else {
            HashMap::new()
        };

        let media_summaries = get_metadata_summary_by_files(&main_db, media_entries)
            .await
            .with_context(|| {
                format!("Failed to fetch media list, page: {cursor}, size: {page_size}")
            })?;

        let media_files = parse_media_files(&fsio, media_summaries, lib_path).await?;
        Ok(Some(FetchMediaFilesResponse {
            media_files,
            cover_art_map,
        }))
    }
}

impl ParamsExtractor for FetchMediaFileByIdsRequest {
    type Params = (Arc<FsIo>, Arc<MainDbConnection>, Arc<String>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.fsio),
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.lib_path),
        )
    }
}

impl Signal for FetchMediaFileByIdsRequest {
    type Params = (Arc<FsIo>, Arc<MainDbConnection>, Arc<String>);
    type Response = FetchMediaFileByIdsResponse;

    async fn handle(
        &self,
        (fsio, main_db, lib_path): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

        let media_entries = get_files_by_ids(&main_db, &request.ids)
            .await
            .with_context(|| format!("Failed to get media summaries for id: {:?}", request.ids))?;

        let media_summaries = get_metadata_summary_by_files(&main_db, media_entries)
            .await
            .with_context(|| "Unable to get media summaries")?;

        let items = parse_media_files(&fsio, media_summaries, lib_path)
            .await
            .with_context(|| "Failed to parse media summaries")?;

        let cover_art_map = if request.bake_cover_arts {
            bake_cover_art_by_file_ids(&fsio, &main_db, request.ids.clone()).await?
        } else {
            HashMap::new()
        };

        Ok(Some(FetchMediaFileByIdsResponse {
            media_files: items,
            cover_art_map,
        }))
    }
}

impl ParamsExtractor for FetchParsedMediaFileRequest {
    type Params = (Arc<FsIo>, Arc<MainDbConnection>, Arc<String>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            Arc::clone(&all_params.fsio),
            Arc::clone(&all_params.main_db),
            Arc::clone(&all_params.lib_path),
        )
    }
}

impl Signal for FetchParsedMediaFileRequest {
    type Params = (Arc<FsIo>, Arc<DatabaseConnection>, Arc<String>);
    type Response = FetchParsedMediaFileResponse;

    async fn handle(
        &self,
        (fsio, db, lib_path): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let file_id = dart_signal.id;

        let (media_file, artists, album) = get_parsed_file_by_id(&db, file_id)
            .await
            .with_context(|| "Failed to get media summaries")?;

        let parsed_files = parse_media_files(&fsio, vec![media_file], lib_path.clone())
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
            file: media_file.clone(),
            artists: artists
                .into_iter()
                .map(|x| Artist {
                    id: x.id,
                    name: x.name,
                })
                .collect(),
            album: Album {
                id: album.id,
                name: album.name,
            },
        }))
    }
}

impl ParamsExtractor for SearchMediaFileSummaryRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for SearchMediaFileSummaryRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = SearchMediaFileSummaryResponse;
    async fn handle(
        &self,
        (main_db,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;

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
}
