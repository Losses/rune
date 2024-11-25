use anyhow::Context;
use anyhow::Result;

use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::FormatReader;
use symphonia::core::formats::Track;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub fn get_format(file_path: &str) -> Result<Box<dyn FormatReader>> {
    // Open the media source.
    let src = std::fs::File::open(file_path).with_context(|| "failed to open media")?;

    // Create the media source stream.
    let mss = MediaSourceStream::new(Box::new(src), Default::default());

    // Create a probe hint using the file's extension.
    let mut hint = Hint::new();
    let ext = file_path.split('.').last().unwrap_or_default();
    hint.with_extension(ext);

    // Use the default options for metadata and format readers.
    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();

    // Probe the media source.
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .with_context(|| "unsupported format")?;

    // Get the instantiated format reader.
    let format = probed.format;

    Ok(format)
}

pub fn get_codec_information(track: &Track) -> Result<(u32, f64), symphonia::core::errors::Error> {
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| symphonia::core::errors::Error::Unsupported("No sample rate found"))?;
    let duration = track
        .codec_params
        .n_frames
        .ok_or_else(|| symphonia::core::errors::Error::Unsupported("No duration found"))?;

    let time_base = track
        .codec_params
        .time_base
        .unwrap_or_else(|| symphonia::core::units::TimeBase::new(1, sample_rate));
    let duration_in_seconds =
        time_base.calc_time(duration).seconds as f64 + time_base.calc_time(duration).frac;

    Ok((sample_rate, duration_in_seconds))
}
