use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
};

use anyhow::Result;
use sea_orm::DatabaseConnection;

use fsio::FsIo;
use playback::player::PlayingItem;

use super::{
    MediaFileHandle, PlayingFileMetadataProvider, PlayingItemMetadataSummary,
    independent_file::IndependentFileProcessor, library_item::LibraryItemProcessor,
};

pub struct PlayingItemActionDispatcher {
    in_library_processor: Box<dyn PlayingFileMetadataProvider + Send + Sync>,
    independent_file_processor: Box<dyn PlayingFileMetadataProvider + Send + Sync>,
}

impl PlayingItemActionDispatcher {
    pub fn new() -> Self {
        Self {
            in_library_processor: Box::new(LibraryItemProcessor),
            independent_file_processor: Box::new(IndependentFileProcessor),
        }
    }

    async fn process_with_both_processors<'a, T, F, Fut>(
        &'a self,
        main_db: &'a DatabaseConnection,
        items: &'a [PlayingItem],
        processor_fn: F,
    ) -> Result<Vec<T>>
    where
        F: Fn(
                &'a (dyn PlayingFileMetadataProvider + Send + Sync),
                &'a DatabaseConnection,
                &'a [PlayingItem],
            ) -> Fut
            + Send
            + Sync,
        Fut: Future<Output = Result<Vec<T>>> + Send + 'a,
        T: HasPlayingItem,
    {
        let in_library_results = processor_fn(&*self.in_library_processor, main_db, items).await?;
        let independent_results =
            processor_fn(&*self.independent_file_processor, main_db, items).await?;

        Ok(Self::merge_and_sort_results(
            items,
            in_library_results,
            independent_results,
        ))
    }

    fn merge_and_sort_results<T: HasPlayingItem>(
        items: &[PlayingItem],
        mut results1: Vec<T>,
        mut results2: Vec<T>,
    ) -> Vec<T> {
        let item_index_map: HashMap<&PlayingItem, usize> = items
            .iter()
            .enumerate()
            .map(|(index, item)| (item, index))
            .collect();

        let mut all_results = Vec::new();
        all_results.append(&mut results1);
        all_results.append(&mut results2);

        all_results.sort_by_key(|result| {
            item_index_map
                .get(&result.playing_item())
                .cloned()
                .unwrap_or(usize::MAX)
        });

        all_results
    }

    pub async fn get_file_handle(
        &self,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<Vec<MediaFileHandle>> {
        self.process_with_both_processors(main_db, items, |processor, db, items| {
            Box::pin(processor.get_file_handle(db, items))
        })
        .await
    }

    pub async fn get_file_path(
        &self,
        lib_path: &Path,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, PathBuf>> {
        let in_library_results = self
            .in_library_processor
            .get_file_path(lib_path, main_db, items)
            .await?;
        let independent_results = self
            .independent_file_processor
            .get_file_path(lib_path, main_db, items)
            .await?;

        let mut result_map = HashMap::new();
        result_map.extend(in_library_results);
        result_map.extend(independent_results);

        Ok(result_map)
    }

    pub async fn get_metadata_summary(
        &self,
        fsio: &FsIo,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<Vec<PlayingItemMetadataSummary>> {
        self.process_with_both_processors(main_db, items, |processor, db, items| {
            Box::pin(processor.get_metadata_summary(fsio, db, items))
        })
        .await
    }

    pub async fn bake_cover_art(
        &self,
        fsio: &FsIo,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, String>> {
        let in_library_results = self
            .in_library_processor
            .bake_cover_art(fsio, main_db, items)
            .await?;
        let independent_results = self
            .independent_file_processor
            .bake_cover_art(fsio, main_db, items)
            .await?;

        let mut result_map = HashMap::new();
        result_map.extend(in_library_results);
        result_map.extend(independent_results);

        Ok(result_map)
    }

    pub async fn get_cover_art_primary_color(
        &self,
        fsio: &FsIo,
        main_db: &DatabaseConnection,
        item: &PlayingItem,
    ) -> Option<i32> {
        let processor: &(dyn PlayingFileMetadataProvider + Send + Sync) = match item {
            PlayingItem::InLibrary(_) => &*self.in_library_processor,
            PlayingItem::IndependentFile(_) => &*self.independent_file_processor,
            PlayingItem::Unknown => return None,
        };

        processor
            .get_cover_art_primary_color(fsio, main_db, item)
            .await
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
