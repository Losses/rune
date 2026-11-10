use crate::messages as crate_messages;
use crate::request_error::ExpectedRequestError;
use database::actions::file::{AudioAvailability, audio_availability, remote_content_ids};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};

use ::database::actions::{
    cover_art::{bake_cover_art_by_file_ids, bake_cover_art_by_media_files},
    file::{get_files_by_ids, get_media_files, get_media_files_count, list_files},
    metadata::{get_metadata_summary_by_files, get_parsed_file_by_id},
};

use crate::{
    Session, Signal,
    library_pool::LibraryHandle,
    messages::*,
    utils::{
        GlobalParams, ParamsExtractor, RunningMode, parse_media_files, process_cover_art_map,
        require_library,
    },
};

impl ParamsExtractor for FetchMediaFilesRequest {
    type Params = (Option<LibraryHandle>, Arc<String>, RunningMode);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library.clone(),
            Arc::clone(&all_params.node_id),
            all_params.running_mode,
        )
    }
}

impl Signal for FetchMediaFilesRequest {
    type Params = (Option<LibraryHandle>, Arc<String>, RunningMode);
    type Response = FetchMediaFilesResponse;

    async fn handle(
        &self,
        (library, node_id, running_mode): Self::Params,
        session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let library = require_library(library)?;
        let fsio = &library.fsio;
        let main_db = &library.main_db;
        let media_root = &library.media_root;
        let cursor = request.cursor;
        let page_size = request.page_size;

        let media_entries = get_media_files(
            main_db,
            cursor.try_into().map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid cursor: {error}"))
            })?,
            page_size.try_into().map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid page size: {error}"))
            })?,
        )
        .await?;

        let cover_art_map = if request.bake_cover_arts {
            bake_cover_art_by_media_files(fsio, main_db, media_entries.clone()).await?
        } else {
            HashMap::new()
        };
        let remote_host = session.map(|session| session.host);
        let cover_art_map = process_cover_art_map(
            cover_art_map,
            library.library_id,
            &running_mode,
            &remote_host,
        );

        let media_summaries = get_metadata_summary_by_files(main_db.as_ref(), media_entries)
            .await
            .with_context(|| {
                format!("Failed to fetch media list, page: {cursor}, size: {page_size}")
            })?;

        let media_files = parse_media_files(
            fsio,
            main_db,
            &node_id,
            media_summaries,
            Arc::new(media_root.clone()),
        )
        .await?;
        Ok(Some(FetchMediaFilesResponse {
            call_id: None,
            media_files,
            cover_art_map,
        }))
    }
}

impl ParamsExtractor for FetchMediaFileByIdsRequest {
    type Params = (Option<LibraryHandle>, Arc<String>, RunningMode);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (
            all_params.library.clone(),
            Arc::clone(&all_params.node_id),
            all_params.running_mode,
        )
    }
}

impl Signal for FetchMediaFileByIdsRequest {
    type Params = (Option<LibraryHandle>, Arc<String>, RunningMode);
    type Response = FetchMediaFileByIdsResponse;

    async fn handle(
        &self,
        (library, node_id, running_mode): Self::Params,
        session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let library = require_library(library)?;
        let fsio = &library.fsio;
        let main_db = &library.main_db;
        let media_root = &library.media_root;

        let media_entries = get_files_by_ids(main_db, &request.ids)
            .await
            .with_context(|| format!("Failed to get media summaries for id: {:?}", request.ids))?;

        let media_summaries = get_metadata_summary_by_files(main_db.as_ref(), media_entries)
            .await
            .with_context(|| "Unable to get media summaries")?;

        let items = parse_media_files(
            fsio,
            main_db,
            &node_id,
            media_summaries,
            Arc::new(media_root.clone()),
        )
        .await
        .with_context(|| "Failed to parse media summaries")?;

        let cover_art_map = if request.bake_cover_arts {
            bake_cover_art_by_file_ids(fsio, main_db, request.ids.clone()).await?
        } else {
            HashMap::new()
        };
        let remote_host = session.map(|session| session.host);
        let cover_art_map = process_cover_art_map(
            cover_art_map,
            library.library_id,
            &running_mode,
            &remote_host,
        );

        Ok(Some(FetchMediaFileByIdsResponse {
            call_id: None,
            media_files: items,
            cover_art_map,
        }))
    }
}

impl ParamsExtractor for FetchParsedMediaFileRequest {
    type Params = (Option<LibraryHandle>, Arc<String>);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.library.clone(), Arc::clone(&all_params.node_id))
    }
}

impl Signal for FetchParsedMediaFileRequest {
    type Params = (Option<LibraryHandle>, Arc<String>);
    type Response = FetchParsedMediaFileResponse;

    async fn handle(
        &self,
        (library, node_id): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let fsio = &library.fsio;
        let db = &library.main_db;
        let media_root = &library.media_root;
        let file_id = dart_signal.id;

        let (media_file, artists, album) = get_parsed_file_by_id(db, file_id)
            .await
            .with_context(|| "Failed to get media summaries")?;

        let parsed_files = parse_media_files(
            fsio,
            db,
            &node_id,
            vec![media_file],
            Arc::new(media_root.clone()),
        )
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
            call_id: None,
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
    type Params = (Option<LibraryHandle>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.library.clone(),)
    }
}

impl Signal for SearchMediaFileSummaryRequest {
    type Params = (Option<LibraryHandle>,);
    type Response = SearchMediaFileSummaryResponse;
    async fn handle(
        &self,
        (library,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let request = dart_signal;
        let library = require_library(library)?;
        let main_db = &library.main_db;
        let fsio = &library.fsio;
        let media_root = &library.media_root;

        let items = list_files(
            main_db,
            request.n.try_into().map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid N: {error}"))
            })?,
        )
        .await
        .with_context(|| "Failed to search media file summary")?;

        let media_summaries = get_metadata_summary_by_files(main_db.as_ref(), items)
            .await
            .with_context(|| "Failed to get media summaries")?;

        let states = audio_availability(
            &**main_db,
            fsio,
            Path::new(media_root),
            &media_summaries.iter().map(|row| row.id).collect::<Vec<_>>(),
        )
        .await?;
        let remote_content = remote_content_ids(
            main_db,
            &media_summaries.iter().map(|row| row.id).collect::<Vec<_>>(),
        )
        .await?;
        Ok(Some(SearchMediaFileSummaryResponse {
            call_id: None,
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
                    audio_availability: crate_messages::AudioAvailability::from_local(
                        states
                            .get(&x.id)
                            .copied()
                            .unwrap_or(AudioAvailability::Unavailable),
                        remote_content.contains(&x.id),
                    ),
                })
                .collect(),
        }))
    }
}

impl ParamsExtractor for GetMediaFilesCountRequest {
    type Params = (Option<LibraryHandle>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.library.clone(),)
    }
}

impl Signal for GetMediaFilesCountRequest {
    type Params = (Option<LibraryHandle>,);
    type Response = GetMediaFilesCountResponse;
    async fn handle(
        &self,
        (library,): Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let count = get_media_files_count(&library.main_db)
            .await
            .with_context(|| "Failed to get media files count")?;

        Ok(Some(GetMediaFilesCountResponse {
            call_id: None,
            count: count.try_into()?,
        }))
    }
}
