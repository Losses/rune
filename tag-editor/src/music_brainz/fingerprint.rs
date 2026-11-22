use std::{io::ErrorKind, path::Path, time::Duration};

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
    io::{MediaSource, MediaSourceStream},
    meta::MetadataOptions,
    probe::Hint,
};
use symphonia::default as symphonia_default;

use ::fsio::{FsIo, OpenMode};
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
        let src = fsio
            .open(path, OpenMode::Read)
            .context("failed to open file")?;
        let source = FsioMediaSource::new(src);
        Self::from_source(Box::new(source), path.extension().and_then(|e| e.to_str()))
    }

    fn from_source(source: Box<dyn MediaSource>, extension: Option<&str>) -> Result<Self> {
        let mss = MediaSourceStream::new(source, Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = extension {
            hint.with_extension(ext);
        }

        let meta_opts: MetadataOptions = Default::default();
        let fmt_opts: FormatOptions = Default::default();

        let probed = symphonia_default::get_probe()
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

        let decoder = fsio_media_source::get_codecs()
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
    let reader = AudioReader::new(fsio, &path).context("initializing audio reader")?;
    fingerprint_audio(reader, config)
}

fn fingerprint_audio(
    mut reader: AudioReader,
    config: &Configuration,
) -> Result<(Vec<u32>, Duration)> {
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
            // Symphonia ends a stream with UnexpectedEof, and a chained Ogg file
            // asks for a reset where its next stream starts. Any other error means
            // the track was not read to the end, so its fingerprint would be partial.
            Err(Error::IoError(err)) if err.kind() == ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(err) => return Err(anyhow!(err).context("read audio")),
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Cursor, Error as IoError, Read, Result as IoResult, Seek, SeekFrom};

    use ::fsio_media_source::ReadSeekSource;

    use super::*;

    const ASSET: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/startup_0.ogg");

    /// Serves `bytes` like a remote file whose connection drops once: the
    /// first read at or past `fail_at` fails, and every later read succeeds.
    struct FlakyFile {
        bytes: Cursor<Vec<u8>>,
        fail_at: Option<u64>,
    }

    impl Read for FlakyFile {
        fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
            let position = self.bytes.position();
            let Some(fail_at) = self.fail_at else {
                return self.bytes.read(buf);
            };
            if position >= fail_at {
                self.fail_at = None;
                return Err(IoError::other("connection reset"));
            }
            let len = buf.len().min((fail_at - position) as usize);
            self.bytes.read(&mut buf[..len])
        }
    }

    impl Seek for FlakyFile {
        fn seek(&mut self, pos: SeekFrom) -> IoResult<u64> {
            self.bytes.seek(pos)
        }
    }

    fn fingerprint_flaky(
        bytes: Vec<u8>,
        extension: &str,
        fail_at: Option<u64>,
    ) -> Result<(Vec<u32>, Duration)> {
        let file = FlakyFile {
            bytes: Cursor::new(bytes),
            fail_at,
        };
        let reader =
            AudioReader::from_source(Box::new(ReadSeekSource::new(file)), Some(extension))?;
        fingerprint_audio(reader, &Configuration::default())
    }

    #[test]
    fn a_clean_end_of_stream_fingerprints_the_whole_track() -> Result<()> {
        let (fingerprint, duration) = fingerprint_flaky(fs::read(ASSET)?, "ogg", None)?;
        let (full_fingerprint, full_duration) =
            calc_fingerprint(&FsIo::new(), ASSET, &Configuration::default())?;
        assert_eq!(duration, full_duration);
        assert_eq!(duration, Duration::from_nanos(9_083_356_009));
        assert_eq!(fingerprint, full_fingerprint);
        Ok(())
    }

    #[test]
    fn a_chained_ogg_file_fingerprints_its_first_stream() -> Result<()> {
        let mut bytes = fs::read(ASSET)?;
        bytes.extend(fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../assets/startup_1.ogg"
        ))?);
        let chained = fingerprint_flaky(bytes, "ogg", None)?;
        assert_eq!(chained, fingerprint_flaky(fs::read(ASSET)?, "ogg", None)?);
        Ok(())
    }

    #[test]
    fn a_read_error_after_the_first_buffer_fails_the_fingerprint() -> Result<()> {
        let bytes = fs::read(ASSET)?;
        let fail_at = bytes.len() as u64 / 2;
        match fingerprint_flaky(bytes, "ogg", Some(fail_at)) {
            Ok((_, duration)) => {
                panic!("a read error midway returned a fingerprint of {duration:?}")
            }
            Err(error) => assert!(
                format!("{error:#}").contains("connection reset"),
                "unexpected error: {error:#}"
            ),
        }
        Ok(())
    }

    #[test]
    fn a_read_error_on_the_first_buffer_fails_the_fingerprint() {
        // One second of silent 16-bit mono PCM. WAV packets are read after the
        // header, so the read that fails is the one for the first buffer.
        let data_len = 44_100_u32 * 2;
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend((36 + data_len).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16_u32.to_le_bytes());
        wav.extend(1_u16.to_le_bytes()); // PCM
        wav.extend(1_u16.to_le_bytes()); // Mono
        wav.extend(44_100_u32.to_le_bytes());
        wav.extend((44_100_u32 * 2).to_le_bytes());
        wav.extend(2_u16.to_le_bytes()); // Bytes per frame
        wav.extend(16_u16.to_le_bytes()); // Bits per sample
        wav.extend(b"data");
        wav.extend(data_len.to_le_bytes());
        let header_len = wav.len() as u64;
        wav.resize(wav.len() + data_len as usize, 0);

        let error = fingerprint_flaky(wav, "wav", Some(header_len))
            .expect_err("a read error on the first buffer returned a fingerprint");
        assert_eq!(error.to_string(), "No audio data found or reader error");
    }

    #[test]
    fn test_fingerprint_opus() {
        let file_path = "/tmp/test_startup.opus";
        if !Path::new(file_path).exists() {
            return;
        }
        let fsio = FsIo::new();
        let config = Configuration::default();
        let result = calc_fingerprint(&fsio, file_path, &config);
        assert!(
            result.is_ok(),
            "Failed to calculate fingerprint for opus: {:?}",
            result.err()
        );
        let (raw, duration) = result.unwrap();
        assert!(!raw.is_empty(), "Fingerprint should not be empty");
        assert!(duration.as_secs_f32() > 0.0, "Duration should be positive");
        println!(
            "Opus fingerprint items: {}, duration: {:?}",
            raw.len(),
            duration
        );
    }

    #[test]
    fn test_fingerprint_aac() {
        let file_path = "/tmp/test_startup.m4a";
        if !Path::new(file_path).exists() {
            return;
        }
        let fsio = FsIo::new();
        let config = Configuration::default();
        let result = calc_fingerprint(&fsio, file_path, &config);
        assert!(
            result.is_ok(),
            "Failed to calculate fingerprint for aac: {:?}",
            result.err()
        );
        let (raw, duration) = result.unwrap();
        assert!(!raw.is_empty(), "Fingerprint should not be empty");
        assert!(duration.as_secs_f32() > 0.0, "Duration should be positive");
        println!(
            "AAC fingerprint items: {}, duration: {:?}",
            raw.len(),
            duration
        );
    }
}
