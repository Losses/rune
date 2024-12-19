use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::Result;
use async_trait::async_trait;
use dunce::canonicalize;
use sea_orm::DatabaseConnection;

use playback::player::PlayingItem;

use crate::actions::{
    cover_art::{
        bake_cover_art_by_file_ids, get_cover_art_id_by_track_id, get_primary_color_by_cover_art_id,
    },
    file::get_files_by_ids,
    metadata::get_metadata_summary_by_file_ids,
};

use super::{MediaFileHandle, PlayingFileMetadataProvider, PlayingItemMetadataSummary};

pub fn extract_in_library_ids(items: Vec<PlayingItem>) -> Vec<i32> {
    items
        .into_iter()
        .filter_map(|item| {
            if let PlayingItem::InLibrary(id) = item {
                Some(id)
            } else {
                None
            }
        })
        .collect()
}

pub struct LibraryItemProcessor;

#[async_trait]
impl PlayingFileMetadataProvider for LibraryItemProcessor {
    async fn get_file_handle(
        &self,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<Vec<MediaFileHandle>> {
        let in_library_ids = extract_in_library_ids(items.to_vec());

        Ok(get_files_by_ids(main_db, &in_library_ids)
            .await?
            .into_iter()
            .map(|x| x.into())
            .collect())
    }

    async fn get_file_path(
        &self,
        lib_path: &Path,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, PathBuf>> {
        let handles = self.get_file_handle(main_db, items).await?;

        let mut result: HashMap<PlayingItem, PathBuf> = HashMap::new();

        for handle in handles {
            let file_path = canonicalize(
                Path::new(lib_path)
                    .join(handle.directory.clone())
                    .join(handle.file_name.clone()),
            )?;

            result.insert(handle.item.clone(), file_path);
        }

        Ok(result)
    }

    async fn get_metadata_summary(
        &self,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<Vec<PlayingItemMetadataSummary>> {
        let in_library_ids = extract_in_library_ids(items.to_vec());

        let result: Vec<PlayingItemMetadataSummary> =
            get_metadata_summary_by_file_ids(main_db, in_library_ids)
                .await?
                .into_iter()
                .map(|x| x.into())
                .collect();

        Ok(result)
    }

    async fn bake_cover_art(
        &self,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, String>> {
        let in_library_ids = extract_in_library_ids(items.to_vec());

        let result: HashMap<i32, String> =
            bake_cover_art_by_file_ids(main_db, in_library_ids).await?;

        let mut converted_result = HashMap::new();

        for (id, cover_art) in result {
            converted_result.insert(PlayingItem::InLibrary(id), cover_art);
        }

        Ok(converted_result)
    }

    async fn get_cover_art_primary_color(
        &self,
        main_db: &DatabaseConnection,
        item: &PlayingItem,
    ) -> Option<i32> {
        match item {
            PlayingItem::InLibrary(track_id) => {
                if let Some(id) = get_cover_art_id_by_track_id(main_db, *track_id)
                    .await
                    .ok()
                    .flatten()
                {
                    get_primary_color_by_cover_art_id(main_db, id).await.ok()
                } else {
                    None
                }
            }
            PlayingItem::IndependentFile(_) => None,
            PlayingItem::Unknown => None,
        }
    }
}
