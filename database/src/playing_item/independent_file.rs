use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use async_trait::async_trait;
use sea_orm::DatabaseConnection;

use ::fsio::FsIo;
use ::metadata::{
    cover_art::extract_cover_art_binary,
    crc::media_crc32,
    describe::{describe_file, get_codec_information_from_node},
    reader::get_metadata,
};
use ::playback::player::PlayingItem;

use crate::actions::{cover_art::COVER_TEMP_DIR, metadata::extract_number};

use super::{MediaFileHandle, PlayingFileMetadataProvider, PlayingItemMetadataSummary};

pub fn extract_independent_file_paths(items: Vec<PlayingItem>) -> Vec<(PlayingItem, String)> {
    items
        .into_iter()
        .filter_map(|item| {
            if let PlayingItem::IndependentFile(path) = item {
                Some((PlayingItem::IndependentFile(path.clone()), path))
            } else {
                None
            }
        })
        .collect()
}

pub struct IndependentFileProcessor;

#[async_trait]
impl PlayingFileMetadataProvider for IndependentFileProcessor {
    async fn get_file_handle(
        &self,
        fsio: &FsIo,
        _main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<Vec<MediaFileHandle>> {
        let independent_paths = extract_independent_file_paths(items.to_vec());

        let independent_handles: Vec<MediaFileHandle> = independent_paths
            .into_iter()
            .filter_map(|(_, x)| {
                let fs_node = fsio.canonicalize_str(&x).ok()?;
                let file_desc = describe_file(&fs_node, &None).ok()?;
                Some(file_desc.into())
            })
            .collect();

        Ok(independent_handles)
    }

    async fn get_file_path(
        &self,
        fsio: &FsIo,
        _lib_path: &Path,
        _main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, PathBuf>> {
        let independent_paths = extract_independent_file_paths(items.to_vec());

        let mut result: HashMap<PlayingItem, PathBuf> = HashMap::new();

        for (playing_item, handle) in independent_paths {
            let fs_node = fsio.canonicalize_str(&handle)?;
            result.insert(playing_item, fs_node.path);
        }

        Ok(result)
    }

    async fn get_metadata_summary(
        &self,
        fsio: &FsIo,
        _main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<Vec<PlayingItemMetadataSummary>> {
        let independent_paths = extract_independent_file_paths(items.to_vec());

        let result = independent_paths
            .iter()
            .filter_map(|(playing_item, path_str)| {
                let fs_node = match fsio.canonicalize_str(path_str) {
                    Ok(x) => x,
                    Err(_) => return None,
                };

                let metadata: Result<Vec<(String, String)>> = get_metadata(&fs_node, None);
                let codec: Result<(u32, f64)> = get_codec_information_from_node(fsio, &fs_node);

                match (metadata, codec) {
                    (Ok(metadata_vec), Ok((_, duration))) => {
                        let metadata: HashMap<_, _> = metadata_vec.iter().cloned().collect();

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
                            item: playing_item.clone(),
                            artist: metadata.get("artist").cloned().unwrap_or_default(),
                            album: metadata.get("album").cloned().unwrap_or_default(),
                            title: metadata
                                .get("track_title")
                                .cloned()
                                .unwrap_or(fs_node.filename),
                            track_number,
                            duration,
                        })
                    }
                    _ => None,
                }
            })
            .collect();

        Ok(result)
    }

    async fn bake_cover_art(
        &self,
        fsio: &FsIo,
        lib_path: &Path,
        _main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, String>> {
        let independent_paths = extract_independent_file_paths(items.to_vec());

        let mut result_map: HashMap<PlayingItem, String> = HashMap::new();

        for (playing_item, path_str) in independent_paths {
            let cover_art_option = extract_cover_art_binary(fsio, Some(lib_path), &path_str);

            if let Some(cover_art) = cover_art_option {
                let crc32 = media_crc32(path_str.as_bytes(), 0, 0, path_str.len());

                let image_file_name = format!("{crc32:08x}");
                let color_file_name = format!("{image_file_name}.color");

                let image_file: PathBuf = COVER_TEMP_DIR.clone().join(image_file_name);
                let color_file: PathBuf = COVER_TEMP_DIR.clone().join(color_file_name);

                if !image_file.exists() {
                    fs::write(image_file.clone(), cover_art.data)?;
                }

                if !color_file.exists() {
                    fs::write(color_file, format!("{:?}", cover_art.primary_color))?;
                }

                result_map.insert(playing_item, image_file.to_string_lossy().to_string());
            }
        }

        Ok(result_map)
    }

    async fn get_cover_art_primary_color(
        &self,
        fsio: &FsIo,
        lib_path: &Path,
        _main_db: &DatabaseConnection,
        item: &PlayingItem,
    ) -> Option<i32> {
        match &item {
            PlayingItem::IndependentFile(path_str) => {
                // Calculate the CRC32 for the file path
                let crc32 = media_crc32(path_str.as_bytes(), 0, 0, path_str.len());
                let image_file_name = format!("{crc32:08x}");
                let color_file_name = format!("{image_file_name}.color");
                let color_file: PathBuf = COVER_TEMP_DIR.clone().join(color_file_name);

                // Check if the color file exists
                if color_file.exists() {
                    // Read the color from the file
                    if let Ok(color_str) = fs::read_to_string(&color_file)
                        && let Ok(color) = color_str.trim().parse::<i32>()
                    {
                        return Some(color);
                    }
                } else {
                    // Attempt to bake cover art
                    if (self
                        .bake_cover_art(fsio, lib_path, _main_db, std::slice::from_ref(item))
                        .await)
                        .is_ok()
                    {
                        // Try reading the color again
                        if let Ok(color_str) = fs::read_to_string(&color_file)
                            && let Ok(color) = color_str.trim().parse::<i32>()
                        {
                            return Some(color);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }
}
