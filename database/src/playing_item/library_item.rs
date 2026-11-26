use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::Result;
use async_trait::async_trait;
use sea_orm::DatabaseConnection;
use uuid::Uuid;

use fsio::FsIo;
use playback::player::PlayingItem;

use crate::actions::{
    cover_art::{
        bake_cover_art_by_file_ids, get_cover_art_id_by_track_id, get_primary_color_by_cover_art_id,
    },
    file::{file_ids_by_uuid, get_files_by_ids},
    metadata::get_metadata_summary_by_file_ids,
};

use super::{MediaFileHandle, PlayingFileMetadataProvider, PlayingItemMetadataSummary};

pub async fn extract_in_library_ids(
    main_db: &DatabaseConnection,
    items: &[PlayingItem],
) -> Result<HashMap<PlayingItem, i32>> {
    let file_uuids = items
        .iter()
        .filter_map(|item| match item {
            PlayingItem::LibraryTrack { track, .. } => Uuid::parse_str(track).ok(),
            _ => None,
        })
        .collect::<Vec<_>>();
    let ids = file_ids_by_uuid(main_db, &file_uuids).await?;
    Ok(items
        .iter()
        .filter_map(|item| match item {
            PlayingItem::LibraryTrack { track, .. } => Uuid::parse_str(track)
                .ok()
                .and_then(|file_uuid| ids.get(&file_uuid).map(|id| (item.clone(), *id))),
            _ => None,
        })
        .collect())
}

pub struct LibraryItemProcessor;

#[async_trait]
impl PlayingFileMetadataProvider for LibraryItemProcessor {
    async fn get_file_handle(
        &self,
        _fsio: &FsIo,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<Vec<MediaFileHandle>> {
        let in_library_ids = extract_in_library_ids(main_db, items).await?;

        Ok(get_files_by_ids(
            main_db,
            &in_library_ids.values().copied().collect::<Vec<_>>(),
        )
        .await?
        .into_iter()
        .map(|x| {
            let id = x.id;
            let mut handle: MediaFileHandle = x.into();
            if let Some(item) = in_library_ids
                .iter()
                .find_map(|(item, item_id)| (*item_id == id).then_some(item))
            {
                handle.item = item.clone();
            }
            handle
        })
        .collect())
    }

    async fn get_file_path(
        &self,
        fsio: &FsIo,
        lib_path: &Path,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, PathBuf>> {
        let handles = self.get_file_handle(fsio, main_db, items).await?;

        // `canonicalize_path` is synchronous filesystem work: on OpenDAL backends
        // it blocks the calling thread on a runtime, which must not be an async
        // worker, so the loop runs on a blocking worker like `materialized_path`.
        let fsio = fsio.clone();
        let lib_path = lib_path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let mut result: HashMap<PlayingItem, PathBuf> = HashMap::new();

            for handle in handles {
                // A track whose audio is not materialized locally has no usable
                // host path: its directory and filename may even be empty, which
                // would collapse the join onto the library root. Skipping it here
                // keeps consumers, such as lyric lookup, from resolving files the
                // track does not own. Mirrors the guard in
                // `crate::actions::file::access::row_path`.
                if !handle.local_available || handle.file_name.is_empty() {
                    continue;
                }

                let file_path = fsio.canonicalize_path(
                    &lib_path
                        .join(handle.directory.clone())
                        .join(handle.file_name.clone()),
                )?;

                result.insert(handle.item.clone(), file_path);
            }

            Ok(result)
        })
        .await?
    }

    async fn get_metadata_summary(
        &self,
        _fsio: &FsIo,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<Vec<PlayingItemMetadataSummary>> {
        let in_library_ids = extract_in_library_ids(main_db, items).await?;

        let result: Vec<PlayingItemMetadataSummary> =
            get_metadata_summary_by_file_ids(main_db, in_library_ids.values().copied().collect())
                .await?
                .into_iter()
                .map(|x| {
                    let id = x.id;
                    let mut summary: PlayingItemMetadataSummary = x.into();
                    if let Some(item) = in_library_ids
                        .iter()
                        .find_map(|(item, item_id)| (*item_id == id).then_some(item))
                    {
                        summary.item = item.clone();
                    }
                    summary
                })
                .collect();

        Ok(result)
    }

    async fn bake_cover_art(
        &self,
        fsio: &FsIo,
        _lib_path: &Path,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, String>> {
        let in_library_ids = extract_in_library_ids(main_db, items).await?;

        let result: HashMap<i32, String> =
            bake_cover_art_by_file_ids(fsio, main_db, in_library_ids.values().copied().collect())
                .await?;

        let mut converted_result = HashMap::new();

        for (id, cover_art) in result {
            converted_result.insert(
                in_library_ids
                    .iter()
                    .find_map(|(item, item_id)| (*item_id == id).then_some(item.clone()))
                    .unwrap_or(PlayingItem::Unknown),
                cover_art,
            );
        }

        Ok(converted_result)
    }

    async fn get_cover_art_primary_color(
        &self,
        _fsio: &FsIo,
        _lib_path: &Path,
        main_db: &DatabaseConnection,
        item: &PlayingItem,
    ) -> Option<i32> {
        let track_id = match item {
            PlayingItem::LibraryTrack { track, .. } => {
                let file_uuid = Uuid::parse_str(track).ok()?;
                file_ids_by_uuid(main_db, &[file_uuid])
                    .await
                    .ok()?
                    .get(&file_uuid)
                    .copied()
            }
            PlayingItem::Online(_, Some(file)) => Some(file.id),
            _ => None,
        }?;

        let cover_art_id = match get_cover_art_id_by_track_id(main_db, track_id).await {
            Ok(Some(id)) => Some(id),
            _ => None,
        }?;

        get_primary_color_by_cover_art_id(main_db, cover_art_id)
            .await
            .ok()
    }
}
