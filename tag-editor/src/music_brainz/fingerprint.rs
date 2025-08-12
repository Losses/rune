use std::{path::Path, time::Duration};

use anyhow::{Context, Result, anyhow};
use base64::{Engine, prelude::BASE64_URL_SAFE_NO_PAD};
pub use rusty_chromaprint::{
    Configuration, FingerprintCompressor, Fingerprinter, Segment, match_fingerprints,
};
use symphonia::core::{
    audio::{AudioBufferRef, SampleBuffer},
    codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions},
    errors::Error,
    formats::{FormatOptions, FormatReader},
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};

use ::fsio::FsIo;
use ::fsio_media_source::FsioMediaSource;

struct AudioReader {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    sample_rate: u32,
}

impl AudioReader {
    fn new(fsio: &FsIo, path: &impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let src = fsio.open(path, "r").context("failed to open file")?;
        let source = FsioMediaSource::new(src);
        let mss = MediaSourceStream::new(Box::new(source), Default::default());

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

        Ok(Self {
            format,
            decoder,
            track_id,
            sample_rate,
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
    fsio: &FsIo,
    path: impl AsRef<Path>,
    config: &Configuration,
) -> Result<(Vec<u32>, Duration)> {
    let mut reader = AudioReader::new(fsio, &path).context("initializing audio reader")?;
    let mut printer = Fingerprinter::new(config);

    let mut total_frames = 0;

    let sample_rate = reader.sample_rate;

    let first_audio_buf = match reader.next_buffer() {
        Ok(buffer) => buffer,
        Err(Error::DecodeError(err)) => return Err(Error::DecodeError(err).into()),
        Err(_) => return Err(anyhow!("No audio data found or reader error")),
    };

    let num_channels: usize = first_audio_buf.spec().channels.count();

    printer
        .start(sample_rate, num_channels as u32)
        .context("initializing fingerprinter")?;

    let spec = *first_audio_buf.spec();
    let duration = first_audio_buf.capacity() as u64;
    let mut sample_buf = Some(SampleBuffer::<i16>::new(duration, spec));

    if let Some(buf) = &mut sample_buf {
        let frame_size = first_audio_buf.frames();
        total_frames += frame_size;

        buf.copy_interleaved_ref(first_audio_buf);
        let frame_data = buf.samples();
        printer.consume(&frame_data[..frame_size * num_channels]);
    }

    loop {
        let audio_buf = match reader.next_buffer() {
            Ok(buffer) => buffer,
            Err(Error::DecodeError(err)) => Err(Error::DecodeError(err))?,
            Err(_) => break,
        };

        if let Some(buf) = &mut sample_buf {
            let frame_size = audio_buf.frames();
            total_frames += frame_size;

            buf.copy_interleaved_ref(audio_buf);
            let frame_data = buf.samples();
            printer.consume(&frame_data[..frame_size * num_channels]);
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

pub fn calculate_similarity_score(
    segments: &[Segment],
    duration_secs: f32,
    config: &Configuration,
) -> f32 {
    let mut total = 0.0;
    for seg in segments {
        let duration = seg.duration(config);
        let score = 1.0 - (seg.score as f32 / 32.0);
        total += score * duration;
    }
    if duration_secs > 0.0 {
        total / duration_secs
    } else {
        0.0
    }
}

pub fn get_track_duration_in_secs(fingerprint: &[u32], config: &Configuration) -> f32 {
    let item_duration = config.item_duration_in_seconds();
    let num_items = fingerprint.len();
    item_duration * num_items as f32
}
