use std::io::{Read, Seek};

use anyhow::{Context, Result};

use crate::{
    reader::legacy_tags,
    taglib::{self, ReadFields},
};

pub async fn get_metadata_and_codec_from_stream(
    source: impl Read + Seek + Send + 'static,
    hint: &str,
) -> Result<(Vec<(String, String)>, f64)> {
    let hint = hint.to_owned();
    let snapshot =
        tokio::task::spawn_blocking(move || taglib::read(source, &hint, ReadFields::Tags))
            .await
            .context("TagLib stream reader failed")??;
    let duration = snapshot.duration_ms.unwrap_or_default() as f64 / 1000.0;
    Ok((legacy_tags(&snapshot.tags, None), duration))
}
