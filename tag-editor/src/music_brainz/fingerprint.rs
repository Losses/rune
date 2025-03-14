use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use base64::Engine;
pub use rusty_chromaprint::{
    match_fingerprints, Configuration, FingerprintCompressor, Fingerprinter, Segment,
};
use symphonia::core::audio::{AudioBufferRef, SampleBuffer};
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

struct AudioReader {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    sample_rate: u32,
    channel_count: usize,
}

impl AudioReader {
    fn new(path: &impl AsRef<Path>) -> anyhow::Result<Self> {
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

        let format = probed.format;

        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .context("no supported audio tracks")?;

        let track_id = track.id;

        let dec_opts: DecoderOptions = Default::default();

        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &dec_opts)
            .context("unsupported codec")?;

        let sample_rate = track
            .codec_params
            .sample_rate
            .context("missing sample rate")?;
        let channel_count = track
            .codec_params
            .channels
            .context("missing audio channels")?
            .count();

        Ok(Self {
            format,
            decoder,
            track_id,
            sample_rate,
            channel_count,
        })
    }

    fn next_buffer(&mut self) -> Result<AudioBufferRef<'_>, Error> {
        let packet = loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                err => break err,
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            break Ok(packet);
        };
        packet.and_then(|pkt| self.decoder.decode(&pkt))
    }
}

pub fn calc_fingerprint(
    path: impl AsRef<Path>,
    config: &Configuration,
) -> anyhow::Result<(Vec<u32>, Duration)> {
    let mut reader = AudioReader::new(&path).context("initializing audio reader")?;

    let mut printer = Fingerprinter::new(config);

    let channel_count: u32 = reader
        .channel_count
        .try_into()
        .context("converting channel count")?;
    printer
        .start(reader.sample_rate, channel_count)
        .context("initializing fingerprinter")?;

    let mut sample_buf = None;
    let mut total_frames = 0;

    loop {
        let audio_buf = match reader.next_buffer() {
            Ok(buffer) => buffer,
            Err(Error::DecodeError(err)) => Err(Error::DecodeError(err))?,
            Err(_) => break,
        };

        if sample_buf.is_none() {
            let spec = *audio_buf.spec();
            let duration = audio_buf.capacity() as u64;
            sample_buf = Some(SampleBuffer::<i16>::new(duration, spec));
        }

        if let Some(buf) = &mut sample_buf {
            let frame_size = audio_buf.frames();
            total_frames += frame_size;

            buf.copy_interleaved_ref(audio_buf);
            let frame_data = buf.samples();
            printer.consume(&frame_data[..frame_size * reader.channel_count]);
        }
    }

    printer.finish();

    let raw_fingerprint = printer.fingerprint().to_vec();
    let duration = Duration::from_secs_f64(total_frames as f64 / reader.sample_rate as f64);

    Ok((raw_fingerprint, duration))
}

pub fn encode_fingerprint(
    raw_fingerprint: Vec<u32>,
    config: &Configuration,
    raw: bool,
    signed: bool,
) -> String {
    if raw {
        if signed {
            // Convert to signed integers and join as a comma-separated string
            raw_fingerprint
                .iter()
                .map(|&x| x as i32)
                .map(|x| x.to_string())
                .collect::<Vec<String>>()
                .join(",")
        } else {
            // Join as a comma-separated string
            raw_fingerprint
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<String>>()
                .join(",")
        }
    } else {
        // Compress the fingerprint and encode it in base64
        let compressed_fingerprint = FingerprintCompressor::from(config).compress(&raw_fingerprint);
        BASE64_URL_SAFE_NO_PAD.encode(&compressed_fingerprint)
    }
}
