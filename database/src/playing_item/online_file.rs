use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use futures::future::join_all;
use log::warn;
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use ::discovery::{client::CertValidator, config::get_config_dir};
use ::fsio::FsIo;
use ::http_request::{
    BodyExt, Empty, Request, StatusCode, Uri, create_https_client, send_http_request,
};
use ::http_request::{Bytes, ClientConfig};
use ::metadata::{
    cover_art::extract_cover_art_from_stream, streaming::get_metadata_and_codec_from_stream,
};
use ::playback::{player::PlayingItem, stream_utils::create_stream_media_source_from_url};

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

async fn get_summary_for_online_in_library_file(
    item: &PlayingItem,
    host: &str,
    file_id: i32,
) -> Result<PlayingItemMetadataSummary> {
    let config_path = get_config_dir()?;
    let cert_validator = Arc::new(
        CertValidator::new(config_path)
            .await
            .with_context(|| "Failed to create the cert validator")?,
    );
    let client_config = Arc::new(cert_validator.into_client_config());

    let metadata = get_media_metadata(host, client_config, file_id as i64).await?;
    Ok(PlayingItemMetadataSummary {
        item: item.clone(),
        title: metadata.file.title,
        artist: metadata.file.artist,
        album: metadata.file.album,
        duration: metadata.file.duration,
        track_number: metadata.file.track_number,
    })
}

async fn get_summary_for_online_file(
    item: &PlayingItem,
    url: &str,
) -> Result<PlayingItemMetadataSummary> {
    let source = create_stream_media_source_from_url(url).await?;
    let mime_type = ""; // Let symphonia detect
    let (metadata, duration) = get_metadata_and_codec_from_stream(source, mime_type).await?;

    let mut summary = PlayingItemMetadataSummary {
        item: item.clone(),
        title: String::new(),
        artist: String::new(),
        album: String::new(),
        duration,
        track_number: 0,
    };

    for (key, value) in metadata {
        match key.to_lowercase().as_str() {
            "title" => summary.title = value,
            "artist" => summary.artist = value,
            "album" => summary.album = value,
            "track" => summary.track_number = value.parse().unwrap_or(0),
            _ => {}
        }
    }

    Ok(summary)
}

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
        items: &[PlayingItem],
    ) -> Result<Vec<PlayingItemMetadataSummary>> {
        let futures = items.iter().filter_map(|item| {
            if let PlayingItem::Online(url, online_file_opt) = item {
                Some(async move {
                    let result = if let Some(online_file) = online_file_opt {
                        get_summary_for_online_in_library_file(
                            item,
                            &online_file.host,
                            online_file.id,
                        )
                        .await
                    } else {
                        get_summary_for_online_file(item, url).await
                    };
                    (item.clone(), result)
                })
            } else {
                None
            }
        });

        let results = join_all(futures).await;
        let mut summaries = Vec::new();
        for (item, result) in results {
            match result {
                Ok(summary) => summaries.push(summary),
                Err(e) => warn!("Failed to get metadata for {item:?}: {e}"),
            }
        }

        Ok(summaries)
    }

    async fn bake_cover_art(
        &self,
        fsio: &FsIo,
        lib_path: &Path,
        _main_db: &DatabaseConnection,
        items: &[PlayingItem],
    ) -> Result<HashMap<PlayingItem, String>> {
        let config_path = get_config_dir()?;
        let cert_validator = Arc::new(
            CertValidator::new(config_path)
                .await
                .with_context(|| "Failed to create the cert validator")?,
        );
        let client_config = Arc::new(cert_validator.into_client_config());

        let futures = items.iter().filter_map(|item| {
            if let PlayingItem::Online(url, online_file_opt) = item {
                let lib_path = lib_path.to_path_buf();
                let item_clone = item.clone();
                let url_clone = url.clone();
                let online_file_opt_clone = online_file_opt.clone();

                let client_config = Arc::clone(&client_config);
                Some(async move {
                    let cover_art_data = if let Some(online_file) = online_file_opt_clone {
                        get_cover_art(&online_file.host, client_config, online_file.id as i64)
                            .await
                            .map(|bytes| bytes.to_vec())
                    } else {
                        let source = create_stream_media_source_from_url(&url_clone).await?;
                        extract_cover_art_from_stream(source, "")
                            .await
                            .map(|cover_art| cover_art.data)
                    };

                    match cover_art_data {
                        Ok(data) => {
                            let file_name = format!("{}.jpg", Uuid::new_v4());
                            let cache_dir = lib_path.join("cache").join("covers");
                            if !fsio.exists(&cache_dir).unwrap_or(false) {
                                fsio.create_dir_all(&cache_dir)?;
                            }
                            let file_path = cache_dir.join(file_name);
                            fsio.write(&file_path, &data).await?;
                            Ok((item_clone, file_path.to_string_lossy().to_string()))
                        }
                        Err(e) => Err(anyhow!(
                            "Failed to get cover art for {:?}: {}",
                            item_clone,
                            e
                        )),
                    }
                })
            } else {
                None
            }
        });

        let results = join_all(futures).await;
        let mut cover_art_map = HashMap::new();
        for result in results {
            match result {
                Ok((item, path)) => {
                    cover_art_map.insert(item, path);
                }
                Err(e) => warn!("{e}"),
            }
        }

        Ok(cover_art_map)
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
