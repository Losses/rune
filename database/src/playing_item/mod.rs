use std::{
    collections::HashMap,
    fs,
    future::Future,
    path::{Path, PathBuf},
};

use anyhow::Result;
use async_trait::async_trait;
use dunce::canonicalize;
use sea_orm::DatabaseConnection;

use metadata::{
    cover_art::extract_cover_art_binary,
    crc::media_crc32,
    describe::{describe_file, get_codec_information_from_path, FileDescription},
    reader::get_metadata,
};
use playback::player::PlayingItem;

use crate::{
    actions::{
        cover_art::{
            bake_cover_art_by_file_ids, get_cover_art_id_by_track_id,
            get_primary_color_by_cover_art_id, COVER_TEMP_DIR,
        },
        file::get_files_by_ids,
        metadata::{extract_number, get_metadata_summary_by_file_ids, MetadataSummary},
    },
    entities::media_files,
};

#[derive(Clone)]
pub struct MediaFileHandle {
    pub item: PlayingItem,
    pub file_name: String,
    pub directory: String,
    pub extension: String,
    pub last_modified: String,
}

impl From<media_files::Model> for MediaFileHandle {
    fn from(x: media_files::Model) -> Self {
        MediaFileHandle {
            item: PlayingItem::InLibrary(x.id),
            file_name: x.file_name,
            directory: x.directory,
            extension: x.extension,
            last_modified: x.last_modified,
        }
    }
}

impl From<FileDescription> for MediaFileHandle {
    fn from(x: FileDescription) -> Self {
        MediaFileHandle {
            item: PlayingItem::IndependentFile(x.full_path),
            file_name: x.file_name,
            directory: x.directory,
            extension: x.extension,
            last_modified: x.last_modified,
        }
    }
}

#[derive(Clone)]
pub struct PlayingItemMetadataSummary {
    pub item: PlayingItem,
    pub artist: String,
    pub album: String,
    pub title: String,
    pub track_number: i32,
    pub duration: f64,
}

impl Default for PlayingItemMetadataSummary {
    fn default() -> Self {
        PlayingItemMetadataSummary {
            item: PlayingItem::Unknown,
            artist: String::from("Unknown Artist"),
            album: String::from("Unknown Album"),
            title: String::from("Unknown Title"),
            track_number: 0,
            duration: 0.0,
        }
    }
}

impl From<MetadataSummary> for PlayingItemMetadataSummary {
    fn from(x: MetadataSummary) -> Self {
        PlayingItemMetadataSummary {
            item: PlayingItem::InLibrary(x.id),
            artist: x.artist,
            album: x.album,
            title: x.title,
            track_number: x.track_number,
            duration: x.duration,
        }
    }
}

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

pub fn extract_independent_file_paths(items: Vec<PlayingItem>) -> Vec<PathBuf> {
    items
        .into_iter()
        .filter_map(|item| {
            if let PlayingItem::IndependentFile(path) = item {
                Some(path)
            } else {
                None
            }
        })
        .collect()
}

#[async_trait]
pub trait PlayingFileMetadataProvider {
    async fn get_file_handle(
        &self,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<Vec<MediaFileHandle>>;

    async fn get_file_path(
        &self,
        lib_path: &Path,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, PathBuf>>;

    async fn get_metadata_summary(
        &self,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<Vec<PlayingItemMetadataSummary>>;

    async fn bake_cover_art(
        &self,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, String>>;

    async fn get_cover_art_primary_color(
        &self,
        main_db: &DatabaseConnection,
        item: &PlayingItem,
    ) -> Option<i32>;
}

pub struct LibraryItemProcessor;
pub struct IndependentFileProcessor;

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

#[async_trait]
impl PlayingFileMetadataProvider for IndependentFileProcessor {
    async fn get_file_handle(
        &self,
        _main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<Vec<MediaFileHandle>> {
        let independent_paths = extract_independent_file_paths(items.to_vec());

        let independent_handles: Vec<MediaFileHandle> = independent_paths
            .into_iter()
            .filter_map(|x| match describe_file(&x, &None) {
                Ok(file_desc) => Some(file_desc.into()),
                Err(e) => {
                    eprintln!("Warning: Failed to describe file {}: {}", x.display(), e);
                    None
                }
            })
            .collect();

        Ok(independent_handles)
    }

    async fn get_file_path(
        &self,
        _lib_path: &Path,
        _main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, PathBuf>> {
        let independent_paths = extract_independent_file_paths(items.to_vec());

        let mut result: HashMap<PlayingItem, PathBuf> = HashMap::new();

        for handle in independent_paths {
            result.insert(PlayingItem::IndependentFile(handle.clone()), handle);
        }

        Ok(result)
    }

    async fn get_metadata_summary(
        &self,
        _main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<Vec<PlayingItemMetadataSummary>> {
        let independent_paths = extract_independent_file_paths(items.to_vec());

        let result = independent_paths
            .iter()
            .filter_map(|path| {
                let file_name = path.file_name()?.to_str()?.to_string();

                match path.to_str() {
                    Some(xs) => {
                        let metadata: Result<Vec<(String, String)>> = get_metadata(xs, None);
                        let codec: Result<(u32, f64)> = get_codec_information_from_path(path);

                        match (metadata, codec) {
                            (Ok(metadata_vec), Ok((_, duration))) => {
                                let metadata: HashMap<_, _> =
                                    metadata_vec.iter().cloned().collect();

                                let parsed_disk_number = metadata
                                    .get("disc_number")
                                    .and_then(|s| extract_number(s))
                                    .unwrap_or(0);

                                let parsed_track_number = metadata
                                    .get("track_number")
                                    .and_then(|s| extract_number(s))
                                    .unwrap_or(0);

                                let track_number = parsed_disk_number * 1000 + parsed_track_number;

                                Some(PlayingItemMetadataSummary {
                                    item: PlayingItem::IndependentFile(path.to_path_buf()),
                                    artist: metadata.get("artist").cloned().unwrap_or_default(),
                                    album: metadata.get("album").cloned().unwrap_or_default(),
                                    title: metadata
                                        .get("track_title")
                                        .cloned()
                                        .unwrap_or(file_name),
                                    track_number,
                                    duration,
                                })
                            }
                            _ => None,
                        }
                    }
                    None => None,
                }
            })
            .collect();

        Ok(result)
    }

    async fn bake_cover_art(
        &self,
        _main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, String>> {
        let independent_paths = extract_independent_file_paths(items.to_vec());

        let mut result_map = HashMap::new();

        for path in independent_paths {
            let cover_art_option = extract_cover_art_binary(&path);

            if let Some(cover_art) = cover_art_option {
                let path_str = path.to_string_lossy();
                let crc32 = media_crc32(path_str.as_bytes(), 0, 0, path_str.len());

                let file_name = format!("{:08x}", crc32);
                let color_file_name = format!("{}.color", file_name);

                let image_file: PathBuf = COVER_TEMP_DIR.clone().join(file_name);
                let color_file: PathBuf = COVER_TEMP_DIR.clone().join(color_file_name);

                if !image_file.exists() {
                    fs::write(path.clone(), cover_art.data)?;
                }

                if !color_file.exists() {
                    fs::write(path.clone(), format!("{:?}", cover_art.primary_color))?;
                }

                result_map.insert(PlayingItem::IndependentFile(path.clone()), image_file);
            }
        }

        Ok(HashMap::new())
    }

    async fn get_cover_art_primary_color(
        &self,
        _main_db: &DatabaseConnection,
        item: &PlayingItem,
    ) -> Option<i32> {
        match item {
            PlayingItem::IndependentFile(ref path) => {
                // Calculate the CRC32 for the file path
                let path_str = path.to_string_lossy();
                let crc32 = media_crc32(path_str.as_bytes(), 0, 0, path_str.len());
                let file_name = format!("{:08x}", crc32);
                let color_file_name = format!("{}.color", file_name);
                let color_file: PathBuf = COVER_TEMP_DIR.clone().join(color_file_name);

                // Check if the color file exists
                if color_file.exists() {
                    // Read the color from the file
                    if let Ok(color_str) = fs::read_to_string(&color_file) {
                        if let Ok(color) = color_str.trim().parse::<i32>() {
                            return Some(color);
                        }
                    }
                } else {
                    // Attempt to bake cover art
                    if (self.bake_cover_art(_main_db, &[item.clone()]).await).is_ok() {
                        // Try reading the color again
                        if let Ok(color_str) = fs::read_to_string(&color_file) {
                            if let Ok(color) = color_str.trim().parse::<i32>() {
                                return Some(color);
                            }
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }
}

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
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<Vec<PlayingItemMetadataSummary>> {
        self.process_with_both_processors(main_db, items, |processor, db, items| {
            Box::pin(processor.get_metadata_summary(db, items))
        })
        .await
    }

    pub async fn bake_cover_art(
        &self,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, String>> {
        let in_library_results = self
            .in_library_processor
            .bake_cover_art(main_db, items)
            .await?;
        let independent_results = self
            .independent_file_processor
            .bake_cover_art(main_db, items)
            .await?;

        let mut result_map = HashMap::new();
        result_map.extend(in_library_results);
        result_map.extend(independent_results);

        Ok(result_map)
    }

    pub async fn get_cover_art_primary_color(
        &self,
        main_db: &DatabaseConnection,
        item: &PlayingItem,
    ) -> Option<i32> {
        let processor: &(dyn PlayingFileMetadataProvider + Send + Sync) = match item {
            PlayingItem::InLibrary(_) => &*self.in_library_processor,
            PlayingItem::IndependentFile(_) => &*self.independent_file_processor,
            PlayingItem::Unknown => return None,
        };

        processor.get_cover_art_primary_color(main_db, item).await
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
