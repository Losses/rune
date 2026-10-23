//! The spectrum, waveform and live levels a scene reads, from the PCM that plays.

use std::collections::VecDeque;
use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};

use super::inputs::Levels;

/// Spectrum columns: 30 Hz to 16 kHz on a log scale.
pub const SPECTRUM_WIDTH: usize = 256;
/// Spectrum rows: the newest first, one per frame.
pub const SPECTRUM_ROWS: usize = 128;
pub const WAVEFORM_WIDTH: usize = 1024;
const FFT_SIZE: usize = 2048;
const LOW_HZ: f32 = 30.0;
const HIGH_HZ: f32 = 16_000.0;
/// The range of decibels shown, under the recent peak.
const RANGE_DB: f32 = 70.0;
/// How fast the peak the spectrum is scaled to falls, in dB per second.
const PEAK_FALL_DB: f32 = 3.0;
/// Bands of the live levels, in Hz.
const BANDS: [(f32, f32); 3] = [(20.0, 250.0), (250.0, 4_000.0), (4_000.0, 16_000.0)];
/// Time constants of the levels: rising, falling, and the long average they are measured against.
const LEVEL_ATTACK: f32 = 0.03;
const LEVEL_RELEASE: f32 = 0.15;
const LEVEL_AVERAGE: f32 = 5.0;

pub struct Analyzer {
    sample_rate: f32,
    /// The latest stereo frames, oldest first.
    recent: VecDeque<[f32; 2]>,
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    input: Vec<f32>,
    output: Vec<realfft::num_complex::Complex<f32>>,
    /// Row-major, row 0 the newest.
    pub spectrum: Vec<f32>,
    pub waveform: Vec<[f32; 2]>,
    peak_db: f32,
    level: [f32; 3],
    average: [f32; 3],
}

impl Analyzer {
    pub fn new(sample_rate: u32) -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
        let window = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_SIZE as f32).cos())
            .collect();
        Self {
            sample_rate: sample_rate.max(1) as f32,
            recent: VecDeque::with_capacity(FFT_SIZE),
            input: fft.make_input_vec(),
            output: fft.make_output_vec(),
            fft,
            window,
            spectrum: vec![0.0; SPECTRUM_WIDTH * SPECTRUM_ROWS],
            waveform: vec![[0.0; 2]; WAVEFORM_WIDTH],
            peak_db: -60.0,
            level: [0.0; 3],
            average: [0.0; 3],
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate as u32
    }

    pub fn push(&mut self, interleaved: &[f32], channels: usize) {
        let channels = channels.max(1);
        for frame in interleaved.chunks_exact(channels) {
            let left = frame[0];
            let right = if channels > 1 { frame[1] } else { left };
            if self.recent.len() == FFT_SIZE {
                self.recent.pop_front();
            }
            self.recent.push_back([left, right]);
        }
    }

    /// One frame `dt` seconds after the last: a new spectrum row, the waveform and the levels.
    pub fn update(&mut self, dt: f32) -> Levels {
        let pad = FFT_SIZE - self.recent.len();
        for (i, x) in self.input.iter_mut().enumerate() {
            let s = if i < pad {
                0.0
            } else {
                let [l, r] = self.recent[i - pad];
                0.5 * (l + r)
            };
            *x = s * self.window[i];
        }
        let _ = self.fft.process(&mut self.input, &mut self.output);
        // Power per bin, scaled so a full-scale sine is about 0 dB.
        let norm = 4.0 / (FFT_SIZE as f32 * FFT_SIZE as f32 * 0.25);
        let power: Vec<f32> = self.output.iter().map(|c| c.norm_sqr() * norm).collect();
        let hz_per_bin = self.sample_rate / FFT_SIZE as f32;

        self.spectrum
            .copy_within(0..SPECTRUM_WIDTH * (SPECTRUM_ROWS - 1), SPECTRUM_WIDTH);
        let mut row = [0.0f32; SPECTRUM_WIDTH];
        let mut frame_peak = f32::MIN;
        for (k, value) in row.iter_mut().enumerate() {
            let edge = |k: f32| LOW_HZ * (HIGH_HZ / LOW_HZ).powf(k / SPECTRUM_WIDTH as f32);
            let (lo, hi) = (
                edge(k as f32) / hz_per_bin,
                edge(k as f32 + 1.0) / hz_per_bin,
            );
            let (a, b) = (lo.ceil() as usize, hi.floor() as usize);
            let p = if b > a && b < power.len() {
                power[a..b].iter().sum::<f32>() / (b - a) as f32
            } else {
                // Narrower than a bin: interpolate at the middle.
                let x = (0.5 * (lo + hi)).min(power.len() as f32 - 1.001);
                let i = x as usize;
                let f = x - i as f32;
                power[i] * (1.0 - f) + power[i + 1] * f
            };
            let db = 10.0 * (p + 1e-12).log10();
            frame_peak = frame_peak.max(db);
            *value = db;
        }
        self.peak_db = frame_peak.max(self.peak_db - PEAK_FALL_DB * dt).max(-60.0);
        let floor = self.peak_db - RANGE_DB;
        for (cell, db) in self.spectrum[..SPECTRUM_WIDTH].iter_mut().zip(row) {
            *cell = ((db - floor) / RANGE_DB).clamp(0.0, 1.0);
        }

        let start = self.recent.len().saturating_sub(WAVEFORM_WIDTH);
        let pad = WAVEFORM_WIDTH - (self.recent.len() - start);
        for (i, w) in self.waveform.iter_mut().enumerate() {
            *w = if i < pad {
                [0.0; 2]
            } else {
                self.recent[start + i - pad]
            };
        }

        let mut levels = [0.0f32; 3];
        for (band, (lo, hi)) in BANDS.iter().enumerate() {
            let a = (lo / hz_per_bin).ceil() as usize;
            let b = ((hi / hz_per_bin).floor() as usize).min(power.len());
            let now: f32 = power[a.min(b)..b].iter().sum();
            let tau = if now > self.level[band] {
                LEVEL_ATTACK
            } else {
                LEVEL_RELEASE
            };
            self.level[band] += (now - self.level[band]) * (1.0 - (-dt / tau).exp());
            self.average[band] +=
                (self.level[band] - self.average[band]) * (1.0 - (-dt / LEVEL_AVERAGE).exp());
            levels[band] = if self.average[band] > 1e-9 {
                (self.level[band] / self.average[band]).min(4.0)
            } else {
                0.0
            };
        }
        Levels {
            bass: levels[0],
            mids: levels[1],
            treble: levels[2],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, rate: f32, n: usize) -> Vec<f32> {
        (0..n)
            .flat_map(|i| {
                let s = (std::f32::consts::TAU * hz * i as f32 / rate).sin() * 0.5;
                [s, s]
            })
            .collect()
    }

    #[test]
    fn a_tone_lights_its_column() {
        let mut a = Analyzer::new(48_000);
        a.push(&sine(1_000.0, 48_000.0, 4096), 2);
        a.update(1.0 / 60.0);
        let row = &a.spectrum[..SPECTRUM_WIDTH];
        let top = (0..SPECTRUM_WIDTH)
            .max_by(|&i, &j| row[i].total_cmp(&row[j]))
            .unwrap();
        let expected =
            (SPECTRUM_WIDTH as f32 * (1_000.0f32 / LOW_HZ).ln() / (HIGH_HZ / LOW_HZ).ln()) as usize;
        assert!(
            top.abs_diff(expected) <= 2,
            "peak at {top}, expected {expected}"
        );
        assert!(row[top] > 0.95);
        assert!(row[10] < 0.3, "far from the tone: {}", row[10]);
        // The newest row moves down a row on the next frame.
        a.update(1.0 / 60.0);
        assert!(a.spectrum[SPECTRUM_WIDTH + top] > 0.95);
        assert_eq!(
            a.waveform[WAVEFORM_WIDTH - 1][0],
            a.waveform[WAVEFORM_WIDTH - 1][1]
        );
    }

    #[test]
    fn levels_are_measured_against_their_average() {
        let mut a = Analyzer::new(48_000);
        let mut levels = Levels::default();
        for _ in 0..600 {
            a.push(&sine(80.0, 48_000.0, 800), 2);
            levels = a.update(1.0 / 60.0);
        }
        assert!(
            (levels.bass - 1.0).abs() < 0.2,
            "steady bass is about 1: {}",
            levels.bass
        );
    }
}
