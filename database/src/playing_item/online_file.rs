use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use http_request::{Bytes, ClientConfig};
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};

use ::fsio::FsIo;
use ::http_request::{
    BodyExt, Empty, Request, StatusCode, Uri, create_https_client, send_http_request,
};
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

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct MediaFileResponse {
    pub id: i32,
    pub path: String,
    pub artist: String,
    pub album: String,
    pub title: String,
    pub duration: f64,
    pub cover_art_id: i32,
    pub track_number: i32,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct AlbumResponse {
    pub id: i32,
    pub name: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ArtistResponse {
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct MediaMetadataResponse {
    pub file: MediaFileResponse,
    pub artists: Vec<ArtistResponse>,
    pub album: AlbumResponse,
}

pub async fn get_media_metadata(
    host: &str,
    config: Arc<ClientConfig>,
    file_id: i64,
) -> Result<MediaMetadataResponse> {
    let uri = Uri::builder()
        .scheme("https")
        .authority(format!("{host}:7863"))
        .path_and_query(format!("/media/{file_id}/metadata"))
        .build()
        .context("Invalid URL format")?;

    let mut sender = create_https_client(host.to_owned(), 7863, config)
        .await
        .context("Failed to create HTTPS client")?;

    let req = Request::builder()
        .uri(uri)
        .header("Accept", "application/json")
        .body(Empty::<Bytes>::new())
        .context("Failed to build request")?;

    let res = send_http_request(&mut sender, req)
        .await
        .context("Failed to execute request")?;

    let status = res.status();
    let body = res
        .into_body()
        .collect()
        .await
        .context("Failed to read response body")?
        .to_bytes();

    if status != StatusCode::OK {
        let error_message = String::from_utf8_lossy(&body);
        return Err(anyhow::anyhow!(
            "Get media metadata failed with status code {}: {}",
            status,
            error_message
        ));
    }

    let response: MediaMetadataResponse =
        serde_json::from_slice(&body).context("Failed to parse media metadata response")?;

    Ok(response)
}

pub async fn get_cover_art(host: &str, config: Arc<ClientConfig>, file_id: i64) -> Result<Bytes> {
    let uri = Uri::builder()
        .scheme("https")
        .authority(format!("{host}:7863"))
        .path_and_query(format!("/media/{file_id}/cover"))
        .build()
        .context("Invalid URL format")?;

    let mut sender = create_https_client(host.to_owned(), 7863, config)
        .await
        .context("Failed to create HTTPS client")?;

    let req = Request::builder()
        .uri(uri)
        .body(Empty::<Bytes>::new())
        .context("Failed to build request")?;

    let res = send_http_request(&mut sender, req)
        .await
        .context("Failed to execute request")?;

    let status = res.status();
    let body = res
        .into_body()
        .collect()
        .await
        .context("Failed to read response body")?
        .to_bytes();

    if status != StatusCode::OK {
        let error_message = String::from_utf8_lossy(&body);
        return Err(anyhow::anyhow!(
            "Get cover art failed with status code {}: {}",
            status,
            error_message
        ));
    }

    Ok(body)
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
