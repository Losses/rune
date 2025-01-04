/**
 * Source: https://github.com/darksv/rusty-chromaprint/blob/main/chromaprint/Cargo.toml
 * Original License: MIT
 */
use std::path::Path;

use anyhow::Context;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use rusty_chromaprint::{Configuration, Fingerprinter};

pub fn calc_fingerprint(
    path: impl AsRef<Path>,
    config: &Configuration,
) -> anyhow::Result<Vec<u32>> {
    let path = path.as_ref();
    let src = std::fs::File::open(path).context("failed to open file")?;
    let mss = MediaSourceStream::new(Box::new(src), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .context("unsupported format")?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .context("no supported audio tracks")?;

    let dec_opts: DecoderOptions = Default::default();

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &dec_opts)
        .context("unsupported codec")?;

    let track_id = track.id;

    let mut printer = Fingerprinter::new(config);
    let sample_rate = track
        .codec_params
        .sample_rate
        .context("missing sample rate")?;
    let channels = track
        .codec_params
        .channels
        .context("missing audio channels")?
        .count() as u32;
    printer
        .start(sample_rate, channels)
        .context("initializing fingerprinter")?;

    let mut sample_buf = None;

    while let Ok(packet) = format.next_packet() {

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                if sample_buf.is_none() {
                    let spec = *audio_buf.spec();
                    let duration = audio_buf.capacity() as u64;
                    sample_buf = Some(SampleBuffer::<i16>::new(duration, spec));
                }

                if let Some(buf) = &mut sample_buf {
                    buf.copy_interleaved_ref(audio_buf);
                    printer.consume(buf.samples());
                }
            }
            Err(Error::DecodeError(_)) => (),
            Err(_) => break,
        }
    }

    printer.finish();
    Ok(printer.fingerprint().to_vec())
}
