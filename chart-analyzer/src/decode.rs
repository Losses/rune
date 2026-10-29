//! Decoding a file the player can play into the analysis's input: the file as mono `f32` at
//! 22050 Hz, handed on as it is decoded and resampled (see `decode_with`), so that no more than
//! a packet and the resampler's buffers is held. The file is opened and probed the way `analysis`'s metadata reader does and
//! decoded with the player's codecs (`fsio_media_source::get_codecs`: all of Symphonia's, with
//! FDK-AAC and libopus for AAC and Opus), so every file the player can play can be analysed.
//!
//! Resampling is a windowed-sinc interpolator (rubato's `SincFixedIn`: 256 taps, Blackman-Harris
//! window, cubic interpolation between 256 oversampled filters), the cutoff as high as the
//! window allows without a ripple in the pass band.

use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use rubato::{
    Resampler as _, SincFixedIn, SincInterpolationParameters, SincInterpolationType,
    WindowFunction, calculate_cutoff,
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
/// The lowest sample rate taken. A lower one is no music, and resampling it up to the analysis
/// rate would make a huge number of samples from a few frames.
const MIN_SAMPLE_RATE: u32 = 1000;

/// What `decode_with` fails with when its `cancel` flag was set: not a fault of the file, so
/// that a caller can tell it by `error.downcast_ref::<Cancelled>()` and not log it as one.
#[derive(Debug)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// `decode`: the whole file at `path` as mono `f32` at 22050 Hz (the channels' mean), in one
/// vector. `decode_with` does the same without holding the song.
pub fn decode(fsio: &FsIo, path: &Path) -> Result<Vec<f32>> {
    let mut samples = Vec::new();
    decode_with(fsio, path, &AtomicBool::new(false), &mut |chunk| {
        samples.extend_from_slice(chunk)
    })?;
    Ok(samples)
}

/// The file at `path` as mono `f32` at 22050 Hz (the channels' mean), handed to `sink` in
/// pieces as it is decoded and resampled: only a packet and the resampler's buffers are held.
/// Once `cancel` is set (by another thread), it stops at the next packet with [`Cancelled`].
pub fn decode_with(
    fsio: &FsIo,
    path: &Path,
    cancel: &AtomicBool,
    sink: &mut dyn FnMut(&[f32]),
) -> Result<()> {
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

    if params.sample_rate.unwrap_or(1) == 0 {
        bail!("the file has no sample rate");
    }
    let mut resampler: Option<Resampler> = None;
    let (mut errors, mut mono) = (0, Vec::new());
    let mut buffer: Option<SampleBuffer<f32>> = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
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
        let channels = spec.channels.count().max(1);
        let buf = match &mut buffer {
            Some(b) if b.capacity() >= decoded.capacity() * channels => b,
            slot => slot.insert(SampleBuffer::new(decoded.capacity() as u64, spec)),
        };
        buf.copy_interleaved_ref(decoded);
        mono.clear();
        mono.extend(
            buf.samples()
                .chunks_exact(channels)
                .map(|f| f.iter().sum::<f32>() / channels as f32),
        );
        // The rate is the first packet's.
        let r = match &mut resampler {
            Some(r) => r,
            slot => slot.insert(Resampler::new(spec.rate, SR as u32)?),
        };
        r.push(&mono, sink)?;
    }
    if let Some(r) = resampler {
        r.finish(sink)?;
    } else if params.sample_rate.is_none() {
        bail!("the file has no sample rate");
    }
    Ok(())
}

/// `samples` at `from` Hz as `to` Hz, all of them: the length is `samples.len() * to / from`
/// rounded, and what comes out lines up with what went in (rubato's sinc resampler has no
/// delay to remove, which the impulse test checks).
pub fn resample(samples: Vec<f32>, from: u32, to: u32) -> Result<Vec<f32>> {
    let mut out = Vec::with_capacity(samples.len() * to as usize / from as usize + CHUNK);
    let mut resampler = Resampler::new(from, to)?;
    resampler.push(&samples, &mut |chunk| out.extend_from_slice(chunk))?;
    resampler.finish(&mut |chunk| out.extend_from_slice(chunk))?;
    Ok(out)
}

/// A sinc resampler fed as the samples come. What it has made is held back by `HELD` samples,
/// which are the only ones the song's end may cut off (the output is exactly
/// `round(input * to / from)` long).
struct Resampler {
    /// `None` when the rates are the same and the samples pass through.
    sinc: Option<SincFixedIn<f32>>,
    ratio: f64,
    /// Input not yet resampled: less than a chunk.
    pending: Vec<f32>,
    out: Vec<Vec<f32>>,
    /// Output made but not handed on yet.
    held: Vec<f32>,
    /// How many samples came in, and how many were handed on.
    taken: usize,
    given: usize,
}

/// How much output is held back until more has come, so that the end can be trimmed.
const HELD: usize = 2 * CHUNK;

/// Hands on all but the last `keep` samples of `held`, counting them in `given`.
fn release(held: &mut Vec<f32>, given: &mut usize, keep: usize, sink: &mut dyn FnMut(&[f32])) {
    let n = held.len().saturating_sub(keep);
    if n > 0 {
        sink(&held[..n]);
        *given += n;
        held.drain(..n);
    }
}

impl Resampler {
    fn new(from: u32, to: u32) -> Result<Resampler> {
        if from < MIN_SAMPLE_RATE {
            bail!("the sample rate of {from} Hz is too low");
        }
        let ratio = f64::from(to) / f64::from(from);
        let sinc = if from == to {
            None
        } else {
            let window = WindowFunction::BlackmanHarris2;
            let parameters = SincInterpolationParameters {
                sinc_len: SINC_LEN,
                f_cutoff: calculate_cutoff(SINC_LEN, window),
                oversampling_factor: OVERSAMPLING,
                interpolation: SincInterpolationType::Cubic,
                window,
            };
            Some(
                SincFixedIn::<f32>::new(ratio, 1.0, parameters, CHUNK, 1)
                    .map_err(|e| anyhow!("cannot resample {from} Hz to {to} Hz: {e}"))?,
            )
        };
        let out = sinc
            .as_ref()
            .map_or_else(Vec::new, |s| s.output_buffer_allocate(true));
        Ok(Resampler {
            sinc,
            ratio,
            pending: Vec::new(),
            out,
            held: Vec::new(),
            taken: 0,
            given: 0,
        })
    }

    fn push(&mut self, x: &[f32], sink: &mut dyn FnMut(&[f32])) -> Result<()> {
        self.taken += x.len();
        let Some(sinc) = &mut self.sinc else {
            sink(x);
            self.given += x.len();
            return Ok(());
        };
        self.pending.extend_from_slice(x);
        while self.pending.len() >= sinc.input_frames_next() {
            let n = sinc.input_frames_next();
            let (used, written) = sinc
                .process_into_buffer(&[&self.pending[..n]], &mut self.out, None)
                .map_err(|e| anyhow!("resampling failed: {e}"))?;
            self.held.extend_from_slice(&self.out[0][..written]);
            self.pending.drain(..used);
            release(&mut self.held, &mut self.given, HELD, sink);
        }
        Ok(())
    }

    /// The end of the input: the rest of it, then silence until the filter has said everything
    /// it has, and all that is held.
    fn finish(mut self, sink: &mut dyn FnMut(&[f32])) -> Result<()> {
        if let Some(sinc) = &mut self.sinc {
            let wanted = (self.taken as f64 * self.ratio).round() as usize;
            let mut rest = (!self.pending.is_empty()).then_some(&self.pending[..]);
            while self.given + self.held.len() < wanted {
                let input = rest.take().map(|r| [r]);
                let (_, written) = sinc
                    .process_partial_into_buffer(
                        input.as_ref().map(|i| i.as_slice()),
                        &mut self.out,
                        None,
                    )
                    .map_err(|e| anyhow!("resampling failed: {e}"))?;
                self.held.extend_from_slice(&self.out[0][..written]);
            }
            self.held.truncate(wanted.saturating_sub(self.given));
        }
        release(&mut self.held, &mut self.given, 0, sink);
        Ok(())
    }
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
    fn a_tiny_sample_rate_is_refused() {
        assert!(resample(vec![0.0; 100], 1, 22050).is_err());
        assert!(resample(vec![0.0; 100], 999, 22050).is_err());
        assert!(resample(vec![0.0; 100], 1000, 22050).is_ok());
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
