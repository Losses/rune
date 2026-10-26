//! Decoding a file the player can play into the analysis's input: the whole file as mono `f32`
//! at 22050 Hz. The file is opened and probed the way `analysis`'s metadata reader does and
//! decoded with the player's codecs (`fsio_media_source::get_codecs`: all of Symphonia's, with
//! FDK-AAC and libopus for AAC and Opus), so every file the player can play can be analysed.
//!
//! Resampling is a windowed-sinc interpolator (rubato's `SincFixedIn`: 256 taps, Blackman-Harris
//! window, cubic interpolation between 256 oversampled filters), the cutoff as high as the
//! window allows without a ripple in the pass band.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
    calculate_cutoff,
};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{CODEC_TYPE_NULL, DecoderOptions},
    errors::Error,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};

use ::fsio::FsIo;
use ::fsio_media_source::FsioMediaSource;

use crate::front::SR;

/// How many packets in a row may fail to decode before the file is given up on. One bad frame
/// in the middle of a file is skipped, as the player does.
const MAX_CONSECUTIVE_DECODE_ERRORS: usize = 16;
/// The resampler's filter length (taps) and the number of oversampled filters.
const SINC_LEN: usize = 256;
const OVERSAMPLING: usize = 256;
/// Frames the resampler takes at a time.
const CHUNK: usize = 4096;

/// `decode`: the whole file at `path` as mono `f32` at 22050 Hz (the channels' mean).
pub fn decode(fsio: &FsIo, path: &Path) -> Result<Vec<f32>> {
    let (mono, rate) = decode_native(fsio, path)?;
    resample(mono, rate, SR as u32)
}

/// The whole file as mono `f32` at its own sample rate, which is returned with it.
fn decode_native(fsio: &FsIo, path: &Path) -> Result<(Vec<f32>, u32)> {
    let src = fsio
        .open(path, "r")
        .with_context(|| format!("failed to open {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(FsioMediaSource::new(src)), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let format_opts = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };
    let mut format = symphonia::default::get_probe()
        .format(&hint, mss, &format_opts, &MetadataOptions::default())
        .map_err(|e| anyhow!("unsupported format: {e}"))?
        .format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow!("no supported audio track"))?;
    let (track_id, params) = (track.id, track.codec_params.clone());
    let mut decoder = fsio_media_source::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(|e| anyhow!("no decoder for the codec: {e}"))?;

    let expected = params.n_frames.unwrap_or(0) as usize;
    let mut mono = Vec::with_capacity(expected);
    let (mut rate, mut errors) = (params.sample_rate.unwrap_or(0), 0);
    let mut buffer: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // The end of the stream is an end-of-file error.
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => bail!("failed to read the file: {e}"),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(Error::DecodeError(e)) => {
                errors += 1;
                if errors > MAX_CONSECUTIVE_DECODE_ERRORS {
                    bail!("too many undecodable packets: {e}");
                }
                continue;
            }
            Err(e) => bail!("failed to decode the file: {e}"),
        };
        errors = 0;
        let spec = *decoded.spec();
        rate = spec.rate;
        let channels = spec.channels.count().max(1);
        let buf = match &mut buffer {
            Some(b) if b.capacity() >= decoded.capacity() * channels => b,
            slot => slot.insert(SampleBuffer::new(decoded.capacity() as u64, spec)),
        };
        buf.copy_interleaved_ref(decoded);
        mono.extend(
            buf.samples()
                .chunks_exact(channels)
                .map(|f| f.iter().sum::<f32>() / channels as f32),
        );
    }
    if rate == 0 {
        bail!("the file has no sample rate");
    }
    Ok((mono, rate))
}

/// `samples` at `from` Hz as `to` Hz, all of them: the length is `samples.len() * to / from`
/// rounded, and what comes out lines up with what went in (rubato's sinc resampler has no
/// delay to remove, which the impulse test checks).
pub fn resample(samples: Vec<f32>, from: u32, to: u32) -> Result<Vec<f32>> {
    if from == to || samples.is_empty() {
        return Ok(samples);
    }
    let window = WindowFunction::BlackmanHarris2;
    let parameters = SincInterpolationParameters {
        sinc_len: SINC_LEN,
        f_cutoff: calculate_cutoff(SINC_LEN, window),
        oversampling_factor: OVERSAMPLING,
        interpolation: SincInterpolationType::Cubic,
        window,
    };
    let ratio = f64::from(to) / f64::from(from);
    let mut resampler = SincFixedIn::<f32>::new(ratio, 1.0, parameters, CHUNK, 1)
        .map_err(|e| anyhow!("cannot resample {from} Hz to {to} Hz: {e}"))?;
    let n = samples.len();
    let wanted = (n as f64 * ratio).round() as usize;
    let mut chunk_out = resampler.output_buffer_allocate(true);
    let mut out = Vec::with_capacity(wanted + CHUNK);
    let mut pos = 0;
    while n - pos >= resampler.input_frames_next() {
        let end = pos + resampler.input_frames_next();
        let (used, written) = resampler
            .process_into_buffer(&[&samples[pos..end]], &mut chunk_out, None)
            .map_err(|e| anyhow!("resampling failed: {e}"))?;
        out.extend_from_slice(&chunk_out[0][..written]);
        pos += used;
    }
    // The rest of the input, then silence until the filter has said everything it has.
    let mut rest = (pos < n).then(|| &samples[pos..]);
    while out.len() < wanted {
        let input = rest.take().map(|r| [r]);
        let (_, written) = resampler
            .process_partial_into_buffer(input.as_ref().map(|i| i.as_slice()), &mut chunk_out, None)
            .map_err(|e| anyhow!("resampling failed: {e}"))?;
        out.extend_from_slice(&chunk_out[0][..written]);
    }
    out.truncate(wanted);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f64, rate: u32, seconds: f64) -> Vec<f32> {
        (0..(rate as f64 * seconds) as usize)
            .map(|i| (2.0 * std::f64::consts::PI * hz * i as f64 / f64::from(rate)).sin() as f32)
            .collect()
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
    }

    #[test]
    fn resampling_keeps_length_and_passband() {
        let out = resample(sine(1000.0, 44100, 3.0), 44100, 22050).unwrap();
        assert_eq!(out.len(), 66150);
        // A 1 kHz sine keeps its level (rms 1/sqrt 2) away from the edges.
        assert!((rms(&out[2000..64000]) - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.005);
        // And is in phase with the sine, sampled half an output sample later (see below).
        let want = sine(1000.0, 44100, 3.0);
        let want: Vec<f32> = want.iter().skip(1).step_by(2).copied().collect();
        let err = out[2000..64000]
            .iter()
            .zip(&want[2000..64000])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(err < 0.01, "{err}");
    }

    #[test]
    fn resampling_removes_what_the_new_rate_cannot_hold() {
        // 15 kHz is above 22050 Hz's Nyquist frequency: it must not alias down to 7 kHz.
        let out = resample(sine(15000.0, 44100, 2.0), 44100, 22050).unwrap();
        assert!(rms(&out[2000..40000]) < 0.01);
    }

    #[test]
    fn resampling_adds_no_delay() {
        // An impulse at 10000 of 44100 Hz is at 5000 of 22050 Hz; the peak straddles 4999 and 5000
        // (the output grid sits half an output sample off the input's).
        let mut x = vec![0.0f32; 40000];
        x[10000] = 1.0;
        let out = resample(x, 44100, 22050).unwrap();
        let peak = (0..out.len())
            .max_by(|&a, &b| out[a].total_cmp(&out[b]))
            .unwrap();
        assert!((4999..=5000).contains(&peak), "{peak}");
    }

    #[test]
    fn same_rate_is_untouched() {
        let x = sine(440.0, 22050, 0.1);
        assert_eq!(resample(x.clone(), 22050, 22050).unwrap(), x);
    }

    #[test]
    fn a_file_that_is_not_audio_is_an_error() {
        let dir = std::env::temp_dir().join(format!("chart_analyzer_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("noise.flac");
        std::fs::write(&path, b"this is not a flac file at all, just text").unwrap();
        assert!(decode(&FsIo::new(), &path).is_err());
        assert!(decode(&FsIo::new(), &dir.join("missing.flac")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
