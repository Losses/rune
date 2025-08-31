use anyhow::{Context, Result};
use symphonia::core::{
    formats::{FormatOptions, FormatReader},
    io::{MediaSource, MediaSourceStream},
    meta::MetadataOptions,
    probe::Hint,
};

use ::analysis::utils::audio_metadata_reader::get_codec_information;

use crate::reader::push_tags;

pub async fn get_metadata_and_codec_from_stream(
    source: Box<dyn MediaSource>,
    mime_type: &str,
) -> Result<(Vec<(String, String)>, f64)> {
    let mss = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    hint.with_extension(mime_type);

    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .context("Failed to probe media source")?;

    let mut format: Box<dyn FormatReader> = probed.format;

    let mut metadata_list = Vec::new();
    let blacklist = vec!["encoded_by", "encoder", "comment", "description"];

    if let Some(metadata_rev) = format.metadata().current() {
        push_tags(metadata_rev, &mut metadata_list, &blacklist);
    }

    let track = format.tracks().first().context("No tracks found")?;
    let (_codec_type, duration) = get_codec_information(track)?;

    Ok((metadata_list, duration))
}
