use anyhow::{anyhow, Result};
use stream_download::{storage::temp::TempStorageProvider, Settings, StreamDownload};
use symphonia::core::io::{MediaSource, ReadOnlySource};

pub async fn create_stream_from_url(
    url: &str,
) -> Result<StreamDownload<TempStorageProvider>> {
    StreamDownload::new_http(
        url.parse()?,
        TempStorageProvider::new(),
        Settings::default(),
    )
    .await
    .map_err(|e| anyhow!(e.to_string()))
}

pub async fn create_stream_media_source_from_url(url: &str) -> Result<Box<dyn MediaSource>> {
    let reader = create_stream_from_url(url).await?;
    let source = Box::new(ReadOnlySource::new(reader));
    Ok(source)
}
