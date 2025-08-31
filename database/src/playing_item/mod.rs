pub mod dispatcher;
pub mod independent_file;
pub mod library_item;
pub mod online_file;

use std::{
    collections::HashMap,
    fmt,
    path::{Path, PathBuf},
};

use anyhow::Result;
use async_trait::async_trait;
use fsio::FsIo;
use sea_orm::DatabaseConnection;

use metadata::describe::FileDescription;
use playback::player::PlayingItem;

use crate::{actions::metadata::MetadataSummary, entities::media_files};

#[derive(Clone, Debug)]
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
            item: PlayingItem::IndependentFile(x.raw_path),
            file_name: x.file_name,
            directory: x.directory,
            extension: x.extension,
            last_modified: x.last_modified,
        }
    }
}

impl fmt::Display for MediaFileHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Item: {}\nFile Name: {}\nDirectory: {}\nExtension: {}\nLast Modified: {}",
            self.item, self.file_name, self.directory, self.extension, self.last_modified
        )
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

#[async_trait]
pub trait PlayingFileMetadataProvider {
    async fn get_file_handle(
        &self,
        fsio: &FsIo,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<Vec<MediaFileHandle>>;

    async fn get_file_path(
        &self,
        fsio: &FsIo,
        lib_path: &Path,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, PathBuf>>;

    async fn get_metadata_summary(
        &self,
        fsio: &FsIo,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<Vec<PlayingItemMetadataSummary>>;

    async fn bake_cover_art(
        &self,
        fsio: &FsIo,
        lib_path: &Path,
        main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, String>>;

    async fn get_cover_art_primary_color(
        &self,
        fsio: &FsIo,
        lib_path: &Path,
        main_db: &DatabaseConnection,
        item: &PlayingItem,
    ) -> Option<i32>;
}
