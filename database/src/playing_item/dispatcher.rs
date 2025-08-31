use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::Result;
use sea_orm::DatabaseConnection;

use ::fsio::FsIo;
use ::playback::player::PlayingItem;

use super::{
    MediaFileHandle, PlayingFileMetadataProvider, PlayingItemMetadataSummary,
    independent_file::IndependentFileProcessor, library_item::LibraryItemProcessor,
    online_file::OnlineFileProcessor,
};

pub struct PlayingItemActionDispatcher {
    in_library_processor: Box<dyn PlayingFileMetadataProvider + Send + Sync>,
    independent_file_processor: Box<dyn PlayingFileMetadataProvider + Send + Sync>,
    online_file_processor: Box<dyn PlayingFileMetadataProvider + Send + Sync>,
}

impl PlayingItemActionDispatcher {
    pub fn new() -> Self {
        Self {
            in_library_processor: Box::new(LibraryItemProcessor),
            independent_file_processor: Box::new(IndependentFileProcessor),
            online_file_processor: Box::new(OnlineFileProcessor),
        }
    }

    fn sort_results<T: HasPlayingItem>(items: &[PlayingItem], results: &mut [T]) {
        let item_index_map: HashMap<&PlayingItem, usize> = items
            .iter()
            .enumerate()
            .map(|(index, item)| (item, index))
            .collect();

        results.sort_by_key(|result| {
            item_index_map
                .get(&result.playing_item())
                .cloned()
                .unwrap_or(usize::MAX)
        });
    }

    pub async fn get_file_handle(
        &self,
        fsio: &FsIo,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<Vec<MediaFileHandle>> {
        let in_library_results = self
            .in_library_processor
            .get_file_handle(fsio, main_db, items)
            .await?;
        let independent_results = self
            .independent_file_processor
            .get_file_handle(fsio, main_db, items)
            .await?;
        let online_results = self
            .online_file_processor
            .get_file_handle(fsio, main_db, items)
            .await?;

        let mut all_results = Vec::new();
        all_results.extend(in_library_results);
        all_results.extend(independent_results);
        all_results.extend(online_results);

        Self::sort_results(items, &mut all_results);

        Ok(all_results)
    }

    pub async fn get_file_path<P: AsRef<Path>>(
        &self,
        fsio: &FsIo,
        lib_path: &P,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, PathBuf>> {
        let in_library_results = self
            .in_library_processor
            .get_file_path(fsio, lib_path.as_ref(), main_db, items)
            .await?;
        let independent_results = self
            .independent_file_processor
            .get_file_path(fsio, lib_path.as_ref(), main_db, items)
            .await?;
        let online_results = self
            .online_file_processor
            .get_file_path(fsio, lib_path.as_ref(), main_db, items)
            .await?;

        let mut result_map = HashMap::new();
        result_map.extend(in_library_results);
        result_map.extend(independent_results);
        result_map.extend(online_results);

        Ok(result_map)
    }

    pub async fn get_metadata_summary(
        &self,
        fsio: &FsIo,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<Vec<PlayingItemMetadataSummary>> {
        let in_library_results = self
            .in_library_processor
            .get_metadata_summary(fsio, main_db, items)
            .await?;
        let independent_results = self
            .independent_file_processor
            .get_metadata_summary(fsio, main_db, items)
            .await?;
        let online_results = self
            .online_file_processor
            .get_metadata_summary(fsio, main_db, items)
            .await?;

        let mut all_results = Vec::new();
        all_results.extend(in_library_results);
        all_results.extend(independent_results);
        all_results.extend(online_results);

        Self::sort_results(items, &mut all_results);

        Ok(all_results)
    }

    pub async fn bake_cover_art<P: AsRef<Path>>(
        &self,
        fsio: &FsIo,
        lib_path: P,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, String>> {
        let lib_path = lib_path.as_ref();
        let in_library_results = self
            .in_library_processor
            .bake_cover_art(fsio, lib_path, main_db, items)
            .await?;
        let independent_results = self
            .independent_file_processor
            .bake_cover_art(fsio, lib_path, main_db, items)
            .await?;
        let online_results = self
            .online_file_processor
            .bake_cover_art(fsio, lib_path, main_db, items)
            .await?;

        let mut result_map = HashMap::new();
        result_map.extend(in_library_results);
        result_map.extend(independent_results);
        result_map.extend(online_results);

        Ok(result_map)
    }

    pub async fn get_cover_art_primary_color(
        &self,
        fsio: &FsIo,
        lib_path: &Path,
        main_db: &DatabaseConnection,
        item: &PlayingItem,
    ) -> Option<i32> {
        match item {
            PlayingItem::InLibrary(_) | PlayingItem::Online(_, Some(_)) => {
                self.in_library_processor
                    .get_cover_art_primary_color(fsio, lib_path, main_db, item)
                    .await
            }
            PlayingItem::IndependentFile(_) => {
                self.independent_file_processor
                    .get_cover_art_primary_color(fsio, lib_path, main_db, item)
                    .await
            }
            PlayingItem::Online(_, None) => {
                self.online_file_processor
                    .get_cover_art_primary_color(fsio, lib_path, main_db, item)
                    .await
            }
            PlayingItem::Unknown => None,
        }
    }
}

pub trait HasPlayingItem {
    fn playing_item(&self) -> &PlayingItem;
}

impl HasPlayingItem for MediaFileHandle {
    fn playing_item(&self) -> &PlayingItem {
        &self.item
    }
}

impl HasPlayingItem for PlayingItemMetadataSummary {
    fn playing_item(&self) -> &PlayingItem {
        &self.item
    }
}

impl Default for PlayingItemActionDispatcher {
    fn default() -> Self {
        Self::new()
    }
}
