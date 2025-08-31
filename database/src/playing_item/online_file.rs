use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::Result;
use async_trait::async_trait;
use sea_orm::DatabaseConnection;

use ::fsio::FsIo;
use ::playback::player::PlayingItem;

use super::{MediaFileHandle, PlayingFileMetadataProvider, PlayingItemMetadataSummary};

pub fn extract_online_file_urls(items: &[PlayingItem]) -> Vec<(PlayingItem, String)> {
    items
        .iter()
        .filter_map(|item| {
            if let PlayingItem::Online(url, None) = item {
                Some((item.clone(), url.clone()))
            } else {
                None
            }
        })
        .collect()
}

pub struct OnlineFileProcessor;

#[async_trait]
impl PlayingFileMetadataProvider for OnlineFileProcessor {
    async fn get_file_handle(
        &self,
        _fsio: &FsIo,
        _main_db: &DatabaseConnection,
        _items: &[PlayingItem],
    ) -> Result<Vec<MediaFileHandle>> {
        // Online files don't have a traditional file handle
        Ok(vec![])
    }

    async fn get_file_path(
        &self,
        _fsio: &FsIo,
        _lib_path: &Path,
        _main_db: &DatabaseConnection,
        _items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, PathBuf>> {
        // Online files are represented by URLs, not local paths
        Ok(HashMap::new())
    }

    async fn get_metadata_summary(
        &self,
        _fsio: &FsIo,
        _main_db: &DatabaseConnection,
        _items: &[PlayingItem],
    ) -> Result<Vec<PlayingItemMetadataSummary>> {
        Ok(vec![])
    }

    async fn bake_cover_art(
        &self,
        _fsio: &FsIo,
        _lib_path: &Path,
        _main_db: &DatabaseConnection,
        _items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, String>> {
        // Cover art baking for online files might need a different approach
        Ok(HashMap::new())
    }

    async fn get_cover_art_primary_color(
        &self,
        _fsio: &FsIo,
        _lib_path: &Path,
        _main_db: &DatabaseConnection,
        _item: &PlayingItem,
    ) -> Option<i32> {
        None
    }
}
