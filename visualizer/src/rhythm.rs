//! A real-time beat tracker, so that presets can move with the music's tempo instead of only its
//! loudness. MilkDrop gives presets three band levels (`bass`, `mid`, `treb`) relative to their
//! recent average and nothing about time; this adds the tempo, a beat clock that runs ahead of
//! the detected onsets, and onsets per drum-like band.
//!
//! The method is the textbook one (Müller, *Fundamentals of Music Processing*, ch. 6):
//!
//! 1. **Onsets.** A short-time spectrum every 10 ms, log-compressed. The positive change of each
//!    bin from two hops before, against the louder of its neighbours then (SuperFlux, Böck and
//!    Widmer 2013), summed over three bands: kick (30 to 160 Hz), snare and the rest (160 Hz to
//!    4 kHz) and hats (5 to 16 kHz). Each band is divided by its own running average, so a quiet
//!    track and a loud one look alike, and the square root of their sum is the onset strength.
//! 2. **Tempo.** Every quarter second, the autocorrelation of the last 8 s of onset strength,
//!    with each lag scored together with its multiples and its half (a beat repeats over the
//!    bar and is divided in two, a syncopated accent every three eighths is neither), weighted
//!    by a prior around 150 bpm against octave errors (Ellis 2007). The prior sits above the usual
//!    120 so that between a tempo and its double fitting about as well, the faster one wins:
//!    better too fast than too slow. A different tempo has to win
//!    for a second before it replaces the current one, which otherwise follows small changes
//!    smoothly.
//! 3. **Beat clock.** A phase that advances by one per beat period, pulled towards the phase the
//!    recent onsets agree on (a comb over the last eight beats, Davies and Plumbley 2007) like a
//!    phase-locked loop: the rate changes, the phase never jumps back, and the clock keeps
//!    running through breaks. Because it is a clock, its beats can be shown when they happen
//!    instead of after the onset has been heard and detected.
//!
//! Everything is driven by the samples pushed, not the wall clock, so it pauses with the music
//! and runs the same in tests.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

/// What the tracker knows at the last sample pushed.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rhythm {
    /// Beats per minute, 0 until a tempo is found.
    pub bpm: f64,
    /// Beats since the tracker started: whole beats counted, the fraction is the phase (0 on the
    /// beat). It only ever grows.
    pub beat: f64,
    /// How sure the tracker is of tempo and phase, 0 to 1. Low in silence, rubato, ambient music.
    pub confidence: f64,
    /// Onsets of the three bands, 0 to 1: 1 on a hit as strong as the strongest of the last few
    /// seconds, decaying within about 0.1 s.
    pub kick: f64,
    pub snare: f64,
    pub hat: f64,
    /// Loudness of the last second against the last 20 seconds: about 1, more in a chorus or a
    /// drop, less in a break. 0 in silence.
    pub energy: f64,
}

/// Onset strengths per second.
const HOPS_PER_SECOND: f64 = 100.0;
/// The window of the spectrum, in seconds (rounded up to a power of two in samples).
const WINDOW_SECONDS: f64 = 0.02;
/// SuperFlux compares with the spectrum this many hops before.
const FLUX_LAG: usize = 2;
/// Band edges in Hz.
const BANDS: [(f64, f64); 3] = [(30.0, 160.0), (160.0, 4_000.0), (5_000.0, 16_000.0)];
/// `ln(1 + COMPRESSION * amplitude)`.
const COMPRESSION: f32 = 1_000.0;
/// Average flux per bin below which a band counts as silent rather than very soft.
const FLUX_FLOOR: f32 = 0.02;
/// Time constant of a band's running average flux, in seconds.
const FLUX_AVERAGE_SECONDS: f64 = 3.0;

/// Onset strength kept, in seconds.
const HISTORY_SECONDS: f64 = 10.0;
/// Onset strength the tempo is found from, in seconds.
const TEMPO_WINDOW_SECONDS: f64 = 8.0;
/// The least onset strength before a first tempo, in seconds.
const TEMPO_MIN_SECONDS: f64 = 2.5;
/// How often the tempo is estimated, in hops.
const TEMPO_EVERY: u32 = 25;
const MIN_BPM: f64 = 50.0;
const MAX_BPM: f64 = 220.0;
/// Centre and width (in octaves) of the tempo prior; above 120 so that the faster of two
/// readings that fit about as well wins.
const PRIOR_BPM: f64 = 150.0;
const PRIOR_OCTAVES: f64 = 1.0;
/// The multiples of a lag that make its score, with their weights. Tuned on songs mixed from
/// Apple Loops of known tempo: with these, 2% of the estimates on songs with drums were neither
/// the tempo nor its double or half, against 19% for the lag alone with decreasing multiples.
const MULTIPLES: [(f64, f64); 5] = [(0.5, 0.5), (1.0, 1.0), (2.0, 1.0), (3.0, 1.0), (4.0, 1.0)];
/// Tempi within this ratio are the same tempo.
const SAME_TEMPO: f64 = 0.04;
/// A rival tempo replaces the current one when the current one scores below this share of it ...
const KEEP_SHARE: f64 = 0.8;
/// ... in this many estimates in a row.
const RIVAL_ESTIMATES: u32 = 4;
/// How far each estimate moves a tempo that stays the same.
const TEMPO_FOLLOW: f64 = 0.3;

/// Beats the comb that finds the phase looks back over, and the weight of each older one.
const COMB_BEATS: usize = 8;
const COMB_DECAY: f64 = 0.85;
/// Phase-locked loop: share of the phase error corrected per beat (proportional) and added to
/// the rate per beat (integral), and the most the rate may differ from the tempo.
const LOCK_GAIN: f64 = 0.6;
const RATE_GAIN: f64 = 0.05;
const MAX_RATE_CHANGE: f64 = 0.3;
const MAX_RATE_DRIFT: f64 = 0.05;
/// Seconds over which the integral part forgets.
const DRIFT_LEAK_SECONDS: f64 = 8.0;
/// A locked clock follows the best phase within this many beats of its own ...
const PHASE_REACH: f64 = 0.2;
/// ... until a phase elsewhere scores this much better for this long.
const RIVAL_PHASE: f64 = 1.3;
const RIVAL_PHASE_SECONDS: f64 = 1.5;
/// Time constant of the confidence, in seconds.
const CONFIDENCE_SECONDS: f64 = 0.7;
/// Seconds of low confidence after which the next lock may move the phase at once.
const UNLOCK_SECONDS: f64 = 2.0;

/// Decay of the band onsets handed out, in seconds, and of their peak, which scales them.
const ONSET_DECAY_SECONDS: f64 = 0.06;
const PEAK_DECAY_SECONDS: f64 = 5.0;
/// The least peak, in units of the band's average flux: steady music does not make every
/// small change a full hit.
const MIN_PEAK: f32 = 3.0;
/// Time constants of the loudness averages, in seconds, and the power below which it is silent.
const ENERGY_SHORT_SECONDS: f64 = 1.0;
const ENERGY_LONG_SECONDS: f64 = 20.0;
const SILENT_POWER: f64 = 1e-7;

pub struct BeatTracker {
    sample_rate: u32,
    hop: usize,
    /// Hops per second (`sample_rate / hop`).
    rate: f64,

    // Spectrum
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    /// The last `window.len()` mono samples, oldest first at `ring_pos`.
    ring: Vec<f32>,
    ring_pos: usize,
    /// Samples since the last hop, and their power.
    pending: usize,
    pending_power: f64,
    frame: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// Log spectra of the last `FLUX_LAG + 1` hops, the newest at `spectra_pos`.
    spectra: Vec<Vec<f32>>,
    spectra_pos: usize,
    bins: [(usize, usize); 3],
    hops: u64,

    // Onsets
    flux_average: [f32; 3],
    /// The last few normalised fluxes of each band, for its local threshold.
    recent: [[f32; 8]; 3],
    peak: [f32; 3],
    onset: [f64; 3],
    /// Onset strength, the newest at `odf_pos`.
    odf: Vec<f32>,
    odf_pos: usize,
    odf_len: usize,

    // Tempo
    /// Hops per beat, 0 while there is no tempo.
    period: f64,
    tempo_confidence: f64,
    rival: Option<(f64, u32)>,
    since_tempo: u32,
    acf: Vec<f64>,

    // Beat clock
    beat: f64,
    /// The integral part of the loop: the rate against the tempo.
    drift: f64,
    /// Beats per hop at the last hop.
    beat_rate: f64,
    locked: bool,
    unsure_hops: u32,
    rival_hops: u32,
    /// The comb's score per phase, see [`BeatTracker::score_phases`].
    comb: Vec<f64>,
    confidence: f64,

    // Loudness
    power_short: f64,
    power_long: f64,
}

impl BeatTracker {
    pub fn new(sample_rate: u32) -> Self {
        let sample_rate = sample_rate.max(8_000);
        let hop = (f64::from(sample_rate) / HOPS_PER_SECOND).round() as usize;
        let size = ((f64::from(sample_rate) * WINDOW_SECONDS) as usize).next_power_of_two();
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(size);
        let window = (0..size)
            .map(|i| {
                let x = std::f32::consts::TAU * i as f32 / size as f32;
                0.5 - 0.5 * x.cos()
            })
            .collect::<Vec<_>>();
        let bin_hz = f64::from(sample_rate) / size as f64;
        let nyquist_bin = size / 2;
        let bins = BANDS.map(|(lo, hi)| {
            let lo = ((lo / bin_hz).round() as usize).clamp(1, nyquist_bin);
            let hi = ((hi / bin_hz).round() as usize).clamp(lo + 1, nyquist_bin + 1);
            (lo, hi)
        });
        let rate = f64::from(sample_rate) / hop as f64;
        let odf_len = (HISTORY_SECONDS * rate) as usize;
        Self {
            sample_rate,
            hop,
            rate,
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            frame: vec![0.0; size],
            fft,
            ring: vec![0.0; size],
            ring_pos: 0,
            pending: 0,
            pending_power: 0.0,
            spectra: vec![vec![0.0; nyquist_bin + 1]; FLUX_LAG + 1],
            spectra_pos: 0,
            window,
            bins,
            hops: 0,
            flux_average: [0.0; 3],
            recent: [[0.0; 8]; 3],
            peak: [MIN_PEAK; 3],
            onset: [0.0; 3],
            odf: vec![0.0; odf_len],
            odf_pos: 0,
            odf_len: 0,
            period: 0.0,
            tempo_confidence: 0.0,
            rival: None,
            since_tempo: 0,
            acf: Vec::new(),
            beat: 0.0,
            drift: 1.0,
            beat_rate: 0.0,
            locked: false,
            unsure_hops: 0,
            rival_hops: 0,
            comb: Vec::new(),
            confidence: 0.0,
            power_short: 0.0,
            power_long: 0.0,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Feed interleaved samples.
    pub fn push(&mut self, interleaved: &[f32], channels: usize) {
        let channels = channels.max(1);
        for frame in interleaved.chunks_exact(channels) {
            let mono = frame.iter().sum::<f32>() / channels as f32;
            self.ring[self.ring_pos] = mono;
            self.ring_pos = (self.ring_pos + 1) % self.ring.len();
            self.pending += 1;
            self.pending_power += f64::from(mono * mono);
            if self.pending == self.hop {
                self.step();
            }
        }
    }

    /// The rhythm at the last sample pushed.
    pub fn rhythm(&self) -> Rhythm {
        let beat = self.beat + self.beat_rate * self.pending as f64 / self.hop as f64;
        let energy = if self.power_long > SILENT_POWER {
            (self.power_short / self.power_long).sqrt().min(4.0)
        } else {
            0.0
        };
        Rhythm {
            // The clock's own correction makes up for the tempo's coarse lags.
            bpm: if self.period > 0.0 {
                60.0 * self.rate * self.drift / self.period
            } else {
                0.0
            },
            beat,
            confidence: self.confidence,
            kick: self.onset[0],
            snare: self.onset[1],
            hat: self.onset[2],
            energy,
        }
    }

    /// One hop: spectrum, onsets, and every so often the tempo; then the clock.
    fn step(&mut self) {
        let power = self.pending_power / self.pending as f64;
        self.pending = 0;
        self.pending_power = 0.0;
        self.hops += 1;
        let short = 1.0 / (ENERGY_SHORT_SECONDS * self.rate);
        let long = 1.0 / (ENERGY_LONG_SECONDS * self.rate);
        self.power_short += (power - self.power_short) * short;
        self.power_long = if self.power_long == 0.0 {
            power
        } else {
            self.power_long + (power - self.power_long) * long
        };

        let flux = self.flux();
        // The plain mean of everything so far until there is enough for the running average.
        let average = (1.0 / (FLUX_AVERAGE_SECONDS * self.rate)).max(1.0 / self.hops as f64) as f32;
        let onset_decay = (-1.0 / (ONSET_DECAY_SECONDS * self.rate)).exp();
        let peak_decay = (-1.0 / (PEAK_DECAY_SECONDS * self.rate)).exp() as f32;
        let mut strength = 0.0;
        for (b, flux) in flux.into_iter().enumerate() {
            self.flux_average[b] += (flux - self.flux_average[b]) * average;
            let relative = flux / self.flux_average[b].max(FLUX_FLOOR);
            strength += relative;
            // A hit stands out from the band's last few hops.
            let recent = &mut self.recent[b];
            let local = recent.iter().sum::<f32>() / recent.len() as f32;
            recent.rotate_right(1);
            recent[0] = relative;
            let hit = (relative - local - 1.0).max(0.0);
            // The peak grows by at most double per hop, so that the first hits after silence do
            // not make the next seconds' look small.
            let peak = self.peak[b];
            self.peak[b] = (peak * peak_decay).max(hit.min(2.0 * peak)).max(MIN_PEAK);
            let level = f64::from((hit / self.peak[b]).min(1.0));
            self.onset[b] = (self.onset[b] * onset_decay).max(level);
        }
        self.odf_pos = (self.odf_pos + 1) % self.odf.len();
        self.odf[self.odf_pos] = strength.sqrt();
        self.odf_len = (self.odf_len + 1).min(self.odf.len());

        self.since_tempo += 1;
        if self.since_tempo >= TEMPO_EVERY {
            self.since_tempo = 0;
            self.estimate_tempo();
        }
        self.advance_clock(power);
    }

    /// SuperFlux of each band: the average positive change per bin.
    fn flux(&mut self) -> [f32; 3] {
        let size = self.ring.len();
        for (i, (out, w)) in self.frame.iter_mut().zip(&self.window).enumerate() {
            *out = self.ring[(self.ring_pos + i) % size] * w;
        }
        if self
            .fft
            .process_with_scratch(&mut self.frame, &mut self.spectrum, &mut self.scratch)
            .is_err()
        {
            return [0.0; 3];
        }
        // Amplitude of a sine of the bin's frequency: |X| * 2 / sum(window) = |X| * 4 / size.
        let scale = 4.0 / size as f32;
        self.spectra_pos = (self.spectra_pos + 1) % self.spectra.len();
        let now = self.spectra_pos;
        let before = (now + 1) % self.spectra.len();
        for (y, x) in self.spectra[now].iter_mut().zip(&self.spectrum) {
            *y = (1.0 + COMPRESSION * x.norm() * scale).ln();
        }
        if self.hops <= FLUX_LAG as u64 {
            return [0.0; 3];
        }
        let (current, previous) = (&self.spectra[now], &self.spectra[before]);
        let last = previous.len() - 1;
        self.bins.map(|(lo, hi)| {
            let mut sum = 0.0;
            for k in lo..hi {
                let reference = previous[k - 1]
                    .max(previous[k])
                    .max(previous[(k + 1).min(last)]);
                sum += (current[k] - reference).max(0.0);
            }
            sum / (hi - lo) as f32
        })
    }

    /// Onset strength `hops` before the newest, interpolated.
    fn odf_at(&self, hops: f64) -> f64 {
        if hops < 0.0 || hops > (self.odf_len - 1) as f64 {
            return 0.0;
        }
        let i = hops.floor() as usize;
        let t = hops - i as f64;
        let n = self.odf.len();
        let at = |k: usize| f64::from(self.odf[(self.odf_pos + n - k.min(self.odf_len - 1)) % n]);
        at(i) * (1.0 - t) + at(i + 1) * t
    }

    fn estimate_tempo(&mut self) {
        if (self.odf_len as f64) < TEMPO_MIN_SECONDS * self.rate {
            return;
        }
        let window = self
            .odf_len
            .min((TEMPO_WINDOW_SECONDS * self.rate) as usize);
        let n = self.odf.len();
        let mut x: Vec<f64> = (0..window)
            .map(|k| f64::from(self.odf[(self.odf_pos + n - k) % n]))
            .collect();
        let mean = x.iter().sum::<f64>() / window as f64;
        x.iter_mut().for_each(|v| *v -= mean);
        let max_lag = window - window / 4;
        self.acf.clear();
        self.acf.extend((0..max_lag).map(|lag| {
            let sum: f64 = x[..window - lag]
                .iter()
                .zip(&x[lag..])
                .map(|(a, b)| a * b)
                .sum();
            sum / (window - lag) as f64
        }));
        if self.acf[0] <= 1e-9 {
            self.tempo_confidence = 0.0;
            return;
        }

        let shortest = 60.0 * self.rate / MAX_BPM;
        let longest = 60.0 * self.rate / MIN_BPM;
        let mut best = (0.0, f64::MIN);
        let mut lag = shortest;
        while lag <= longest {
            let score = self.tempo_score(lag);
            if score > best.1 {
                best = (lag, score);
            }
            lag += 0.25;
        }
        let (best_lag, best_score) = best;
        if best_score <= 0.0 {
            self.tempo_confidence = 0.0;
            return;
        }
        let peak = self.acf_at(best_lag) / self.acf[0];
        self.tempo_confidence = ((peak - 0.05) / 0.25).clamp(0.0, 1.0);

        if self.period == 0.0 || !self.locked && self.confidence < 0.2 {
            self.set_period(best_lag);
            return;
        }
        let same = |a: f64, b: f64| (a / b - 1.0).abs() < SAME_TEMPO;
        // The current tempo, refined around where it is.
        let mut current = (self.period, f64::MIN);
        let mut lag = self.period * (1.0 - SAME_TEMPO);
        while lag <= self.period * (1.0 + SAME_TEMPO) {
            let score = self.tempo_score(lag);
            if score > current.1 {
                current = (lag, score);
            }
            lag += 0.1;
        }
        if same(best_lag, self.period) || current.1 >= KEEP_SHARE * best_score {
            self.rival = None;
            self.period += (current.0 - self.period) * TEMPO_FOLLOW;
            return;
        }
        let count = match self.rival {
            Some((lag, count)) if same(lag, best_lag) => count + 1,
            _ => 1,
        };
        if count >= RIVAL_ESTIMATES {
            self.set_period(best_lag);
        } else {
            self.rival = Some((best_lag, count));
        }
    }

    /// A tempo of a new kind: the clock may move to its phase at once.
    fn set_period(&mut self, lag: f64) {
        self.period = lag;
        self.rival = None;
        self.drift = 1.0;
        self.locked = false;
    }

    /// The autocorrelation of a lag and its multiples, weighted by the prior.
    fn tempo_score(&self, lag: f64) -> f64 {
        let mut score = 0.0;
        for (multiple, weight) in MULTIPLES {
            let at = lag * multiple;
            if at >= (self.acf.len() - 1) as f64 {
                continue;
            }
            // Multiples blur with the quantised lag: take the best within a hop or so.
            let spread = (multiple - 1.0).max(0.0) * 0.5;
            let value = [-spread, 0.0, spread]
                .iter()
                .map(|d| self.acf_at(at + d))
                .fold(f64::MIN, f64::max);
            score += weight * value;
        }
        let bpm = 60.0 * self.rate / lag;
        let octaves = (bpm / PRIOR_BPM).log2() / PRIOR_OCTAVES;
        score * (-0.5 * octaves * octaves).exp()
    }

    fn acf_at(&self, lag: f64) -> f64 {
        let i = (lag.floor() as usize).min(self.acf.len() - 2);
        let t = (lag - i as f64).clamp(0.0, 1.0);
        self.acf[i] * (1.0 - t) + self.acf[i + 1] * t
    }

    /// The comb's score for each offset of the beat grid (hops since the last beat in the onset
    /// strength), into `self.comb`; how clearly the best one stands out, 0 to 1.
    fn score_phases(&mut self) -> f64 {
        let period = self.period;
        let beats = COMB_BEATS.min(((self.odf_len as f64) / period) as usize);
        self.comb.clear();
        if beats < 2 {
            return 0.0;
        }
        for i in 0..period.ceil() as usize {
            let mut score = 0.0;
            let mut weight = 1.0;
            for k in 0..beats {
                score += weight * self.odf_at(i as f64 + k as f64 * period);
                weight *= COMB_DECAY;
            }
            self.comb.push(score);
        }
        let best = self.comb.iter().copied().fold(f64::MIN, f64::max);
        let mean = self.comb.iter().sum::<f64>() / self.comb.len() as f64;
        let clarity = if best > 1e-9 {
            (best - mean) / best
        } else {
            0.0
        };
        ((clarity - 0.25) / 0.35).clamp(0.0, 1.0)
    }

    /// The best offset of the comb within `reach` beats of `expected` (hops), and its score.
    fn best_phase(&self, expected: f64, reach: f64) -> (f64, f64) {
        let n = self.comb.len();
        let mut best = (expected, f64::MIN);
        for (i, &score) in self.comb.iter().enumerate() {
            let apart = ((i as f64 - expected) / self.period + 0.5).rem_euclid(1.0) - 0.5;
            if apart.abs() <= reach && score > best.1 {
                best = (i as f64, score);
            }
        }
        // Between hops, where a parabola through the best and its neighbours peaks.
        let i = best.0 as usize;
        if n >= 3 && best.1 > f64::MIN {
            let (a, b, c) = (self.comb[(i + n - 1) % n], best.1, self.comb[(i + 1) % n]);
            let curve = a - 2.0 * b + c;
            if curve < 0.0 {
                best.0 += (0.5 * (a - c) / curve).clamp(-0.5, 0.5);
            }
        }
        best
    }

    fn advance_clock(&mut self, power: f64) {
        if self.period == 0.0 {
            return;
        }
        // `rhythm` showed the clock up to here between the hops, at the old rate.
        let shown = self.beat + self.beat_rate * (self.hop - 1) as f64 / self.hop as f64;
        let clarity = self.score_phases();
        // The onset strength peaks about half a window after the sound starts.
        let latency = self.ring.len() as f64 / 2.0 / self.hop as f64;
        // Where the clock says the last beat is in the onset strength.
        let expected = (self.beat.fract() * self.period - latency).rem_euclid(self.period);
        let error_of =
            |offset: f64| ((offset - expected) / self.period + 0.5).rem_euclid(1.0) - 0.5;
        let (near, near_score) = self.best_phase(expected, PHASE_REACH);
        let (anywhere, best_score) = self.best_phase(expected, 0.5);

        let audible = self.power_short > SILENT_POWER || power > SILENT_POWER;
        let sure = self.tempo_confidence * clarity * f64::from(u8::from(audible));
        let smoothing = 1.0 / (CONFIDENCE_SECONDS * self.rate);
        self.confidence += (sure - self.confidence) * smoothing;
        if self.confidence < 0.15 {
            self.unsure_hops += 1;
            if f64::from(self.unsure_hops) > UNLOCK_SECONDS * self.rate {
                self.locked = false;
            }
        } else {
            self.unsure_hops = 0;
        }
        // Another phase that keeps beating the one followed takes over.
        if self.locked && best_score > RIVAL_PHASE * near_score {
            self.rival_hops += 1;
            if f64::from(self.rival_hops) > RIVAL_PHASE_SECONDS * self.rate {
                self.locked = false;
            }
        } else {
            self.rival_hops = 0;
        }

        let mut correction = 0.0;
        if !self.locked {
            if clarity > 0.5 {
                // Move to the observed phase, forwards only.
                self.beat += error_of(anywhere).rem_euclid(1.0);
                self.locked = true;
                self.rival_hops = 0;
            }
        } else {
            correction = error_of(near) * clarity;
        }
        // The integral part leaks back to the tempo, so that noise does not build up in it.
        self.drift += RATE_GAIN * correction / self.period
            - (self.drift - 1.0) / (DRIFT_LEAK_SECONDS * self.rate);
        self.drift = self.drift.clamp(1.0 - MAX_RATE_DRIFT, 1.0 + MAX_RATE_DRIFT);
        let pull = (LOCK_GAIN * correction).clamp(-MAX_RATE_CHANGE, MAX_RATE_CHANGE);
        self.beat_rate = self.drift * (1.0 + pull) / self.period;
        self.beat = (self.beat + self.beat_rate).max(shown);
    }
}

#[cfg(test)]
mod tests_support {
    use super::*;

    pub const RATE: u32 = 44_100;
    /// Samples per push, a frame of video at 60 fps.
    pub const CHUNK: usize = 735;

    /// A kick drum: a falling sine with a click, `seconds` long, on the beats of `bpm(t)`.
    pub fn drums(seconds: f64, bpm: impl Fn(f64) -> f64, hats: bool) -> (Vec<f32>, Vec<f64>) {
        let total = (seconds * f64::from(RATE)) as usize;
        let mut out = vec![0.0f32; total];
        let mut beats = Vec::new();
        let mut t = 0.25;
        let mut seed = 1u32;
        let mut noise = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0
        };
        while t < seconds {
            beats.push(t);
            let period = 60.0 / bpm(t);
            let start = (t * f64::from(RATE)) as usize;
            for i in 0..(0.25 * f64::from(RATE)) as usize {
                if start + i >= total {
                    break;
                }
                let s = i as f64 / f64::from(RATE);
                let freq = 50.0 + 100.0 * (-s * 30.0).exp();
                let body = (std::f64::consts::TAU * freq * s).sin() * (-s * 12.0).exp();
                let click = f64::from(noise()) * (-s * 400.0).exp() * 0.3;
                out[start + i] += (0.6 * (body + click)) as f32;
            }
            if hats {
                let off = ((t + period / 2.0) * f64::from(RATE)) as usize;
                for i in 0..(0.05 * f64::from(RATE)) as usize {
                    if off + i < total {
                        let s = i as f32 / RATE as f32;
                        out[off + i] += noise() * (-s * 90.0).exp() * 0.15;
                    }
                }
            }
            t += period;
        }
        (out, beats)
    }

    /// Run `samples` through a tracker in video-frame chunks, returning the rhythm after each
    /// chunk with the time of its last sample.
    pub fn track(samples: &[f32]) -> Vec<(f64, Rhythm)> {
        let mut tracker = BeatTracker::new(RATE);
        let mut out = Vec::new();
        let mut done = 0;
        for chunk in samples.chunks(CHUNK) {
            tracker.push(chunk, 1);
            done += chunk.len();
            out.push((done as f64 / f64::from(RATE), tracker.rhythm()));
        }
        out
    }

    /// Times at which the beat count went up.
    pub fn ticks(run: &[(f64, Rhythm)]) -> Vec<f64> {
        run.windows(2)
            .filter(|w| w[1].1.beat.floor() > w[0].1.beat.floor())
            .map(|w| {
                // Interpolate where the count crossed the integer.
                let (t0, b0) = (w[0].0, w[0].1.beat);
                let (t1, b1) = (w[1].0, w[1].1.beat);
                t0 + (t1 - t0) * (b1.floor() - b0) / (b1 - b0)
            })
            .collect()
    }

    /// The distance from `t` to the nearest of `beats`.
    pub fn off(t: f64, beats: &[f64]) -> f64 {
        beats.iter().map(|b| (t - b).abs()).fold(f64::MAX, f64::min)
    }
}

#[cfg(test)]
mod tests {
    use super::BeatTracker;
    use super::tests_support::*;

    #[test]
    fn locks_to_a_steady_beat() {
        for (bpm, hats) in [(90.0, false), (120.0, true), (140.0, true)] {
            let (samples, beats) = drums(16.0, |_| bpm, hats);
            let run = track(&samples);
            let (_, last) = run.last().unwrap();
            assert!((last.bpm / bpm - 1.0).abs() < 0.015, "{bpm}: {last:?}");
            assert!(last.confidence > 0.6, "{bpm}: {last:?}");
            let late: Vec<_> = ticks(&run).into_iter().filter(|&t| t > 8.0).collect();
            assert!(
                late.len() as f64 >= 7.0 * bpm / 60.0,
                "{bpm}: {} ticks",
                late.len()
            );
            for t in late {
                assert!(off(t, &beats) < 0.03, "{bpm}: tick at {t:.3} is off");
            }
        }
    }

    /// Hats on every off-beat as loud as the kicks on the beat: a pulse at either speed is right,
    /// and the faster one is taken (better too fast than too slow).
    #[test]
    fn off_beat_hats_give_the_double() {
        let (samples, beats) = drums(16.0, |_| 90.0, true);
        let run = track(&samples);
        let (_, last) = run.last().unwrap();
        let ratio = last.bpm / 90.0;
        assert!((ratio - 2.0).abs() < 0.03, "{last:?}");
        let halves: Vec<f64> = beats.iter().flat_map(|b| [*b, b + 60.0 / 180.0]).collect();
        for t in ticks(&run).into_iter().filter(|&t| t > 8.0) {
            assert!(off(t, &halves) < 0.03, "tick at {t:.3} is off");
        }
    }

    #[test]
    fn follows_a_tempo_change() {
        let (samples, beats) = drums(30.0, |t| if t < 12.0 { 124.0 } else { 100.0 }, false);
        let run = track(&samples);
        let (_, last) = run.last().unwrap();
        assert!((last.bpm / 100.0 - 1.0).abs() < 0.015, "{last:?}");
        for t in ticks(&run).into_iter().filter(|&t| t > 22.0) {
            assert!(off(t, &beats) < 0.03, "tick at {t:.3} is off");
        }
    }

    /// Also between hops, where the clock is extrapolated, and across a change of tempo.
    #[test]
    fn the_beat_only_grows() {
        let (samples, _) = drums(20.0, |t| if t < 10.0 { 128.0 } else { 172.0 }, true);
        let mut tracker = BeatTracker::new(RATE);
        let mut last = 0.0;
        for chunk in samples.chunks(97) {
            tracker.push(chunk, 1);
            let beat = tracker.rhythm().beat;
            assert!(beat >= last, "{last} then {beat}");
            last = beat;
        }
    }

    #[test]
    fn is_unsure_of_silence_and_noise() {
        let silence = vec![0.0f32; 10 * RATE as usize];
        let (_, last) = *track(&silence).last().unwrap();
        assert_eq!(last.confidence, 0.0);
        assert_eq!(last.energy, 0.0);

        let mut seed = 7u32;
        let noise: Vec<f32> = (0..10 * RATE as usize)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((seed >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0) * 0.3
            })
            .collect();
        let (_, last) = *track(&noise).last().unwrap();
        assert!(last.confidence < 0.2, "{last:?}");
        assert!((last.energy - 1.0).abs() < 0.2, "{last:?}");
    }

    #[test]
    fn kicks_show_on_the_kick_band() {
        let (samples, beats) = drums(6.0, |_| 120.0, false);
        let run = track(&samples);
        // Right after a kick (two or three frames on, the window being 23 ms) the kick band is up.
        for &b in beats.iter().filter(|&&b| b > 2.0 && b < 5.5) {
            let (_, r) = run.iter().find(|(t, _)| *t > b + 0.04).unwrap();
            assert!(r.kick > 0.5, "kick at {b:.2}: {r:?}");
        }
        let (_, between) = run.iter().find(|(t, _)| *t > beats[6] + 0.3).unwrap();
        assert!(between.kick < 0.1, "{between:?}");
    }
}
