//! The front end, a port of the Python prototype's `front()`: the part of the analysis that
//! looks only at its own neighbourhood in time, so it is computed once for a whole song and
//! cut at any point. The spectra, the percussive and harmonic split, the raw onset curves, the
//! chroma and timbre, the raw energies. Nothing here is scaled against the song or decided
//! over it, that is `back`.
//!
//! The spectra are `f32`, as in numpy; the curves made from them are stored as `f64` holding
//! the `f32` values the Python has, and everything after is `f64`.

use realfft::RealFftPlanner;

use crate::ndimage::{maximum_filter1d, median_filter1d};
use crate::np::{geomspace, hanning, round_half_even};

/// Sample rate the analysis runs at.
pub const SR: usize = 22050;
/// Fine frames: onsets and beats. Window and hop, in samples.
pub const N: usize = 1024;
pub const HOP: usize = 220;
/// Fine frames per second.
pub const FPS: f64 = SR as f64 / HOP as f64;
/// Coarse frames: chroma, timbre, curves.
pub const CN: usize = 4096;
pub const CHOP: usize = 1102;
/// Coarse frames per second.
pub const CFPS: f64 = SR as f64 / CHOP as f64;
/// Keeps a division by a sum that may be zero finite.
pub const EPS: f64 = 1e-9;
/// `EPS` where the Python adds it to a `float32` array.
const EPS32: f32 = 1e-9;

/// How many frames the harmonic median covers over time (80 ms), and the percussive median over
/// frequency (what the HPSS mask is made from).
const MEDIAN_SIZE: usize = 17;
/// `log1p(COMPRESSION * amplitude)` of the spectra.
const COMPRESSION: f32 = 1000.0;
/// The lowest and highest frequency (Hz) whose bins vote for a pitch class in the chroma.
const CHROMA_RANGE: (f64, f64) = (80.0, 5000.0);
/// Compression of a bin's power before it votes for its pitch class.
const CHROMA_COMPRESSION: f32 = 100.0;
/// The timbre's log-spaced bands: from, to (Hz) and count, and how many DCT coefficients it keeps.
const TIMBRE_RANGE: (f64, f64) = (60.0, 10000.0);
const TIMBRE_BANDS: usize = 24;
const TIMBRE_COEFFS: usize = 12;
/// Keeps the log of an empty timbre band finite.
const TIMBRE_FLOOR: f32 = 1e-8;
/// Keeps the log of a silent frame's loudness finite.
const LOUD_FLOOR: f32 = 1e-10;

/// A magnitude spectrogram, `frames` rows of `bins` values.
pub struct Spectrogram {
    pub frames: usize,
    pub bins: usize,
    pub data: Vec<f32>,
}

impl Spectrogram {
    pub fn row(&self, t: usize) -> &[f32] {
        &self.data[t * self.bins..(t + 1) * self.bins]
    }
}

/// Everything `front()` returns.
pub struct Front {
    /// Length of the audio, in samples.
    pub samples: usize,
    /// Onset streams from the percussive part: low, mid, high.
    pub raw_low: Vec<f64>,
    pub raw_mid: Vec<f64>,
    pub raw_high: Vec<f64>,
    /// The harmonic part's note onsets, from 250 Hz: the notes of a melody more than the bass.
    pub raw_harmonic: Vec<f64>,
    /// The three whole-spectrum bands, for the beat's onset strength.
    pub band_low: Vec<f64>,
    pub band_mid: Vec<f64>,
    pub band_high: Vec<f64>,
    /// The spectrum's raw change per frame, which `feel` measures.
    pub flux: Vec<f64>,
    /// Per coarse frame: 12 chroma, 12 timbre coefficients, loudness (dB), centroid (Hz).
    pub chroma: Vec<[f64; 12]>,
    pub timbre: Vec<[f64; TIMBRE_COEFFS]>,
    pub loud: Vec<f64>,
    pub cent: Vec<f64>,
    /// Per fine frame: the energy of the percussive and harmonic parts, and of the percussive
    /// one below 160 Hz, from 160 Hz to 4 kHz and from 5 kHz.
    pub energy_p: Vec<f64>,
    pub energy_h: Vec<f64>,
    pub share_low: Vec<f64>,
    pub share_mid: Vec<f64>,
    pub share_high: Vec<f64>,
}

/// `stft(x, n, hop)`: Hann-windowed spectra of frames centred every `hop` samples (the signal
/// padded by `n / 2` zeros each side), magnitudes divided by `n / 4`.
pub fn stft(x: &[f32], n: usize, hop: usize) -> Spectrogram {
    let frames = (x.len() + 1).div_ceil(hop);
    let bins = n / 2 + 1;
    let win: Vec<f32> = hanning(n).iter().map(|&w| w as f32).collect();
    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(n);
    let (mut input, mut output) = (fft.make_input_vec(), fft.make_output_vec());
    let mut scratch = fft.make_scratch_vec();
    let scale = (n / 4) as f32;
    let mut data = Vec::with_capacity(frames * bins);
    for t in 0..frames {
        let start = (t * hop) as isize - (n / 2) as isize;
        for (j, v) in input.iter_mut().enumerate() {
            let i = start + j as isize;
            let s = if i >= 0 && (i as usize) < x.len() {
                x[i as usize]
            } else {
                0.0
            };
            *v = (s * win[j]) as f64;
        }
        let done = fft.process_with_scratch(&mut input, &mut output, &mut scratch);
        debug_assert!(done.is_ok(), "the buffers are the plan's own");
        data.extend(output.iter().map(|c| c.norm() as f32 / scale));
    }
    Spectrogram { frames, bins, data }
}

/// `hz_bin(f, n)`: the bin of `f` Hz in an `n`-point spectrum (Python's `round`).
pub fn hz_bin(f: f64, n: usize) -> usize {
    round_half_even(f * n as f64 / SR as f64) as usize
}

/// The mean of `f32` values, summed in `f64` and rounded to `f32` as numpy's mean is.
fn mean_f32(values: impl Iterator<Item = f32>, count: usize) -> f64 {
    (values.map(f64::from).sum::<f64>() as f32 / count as f32) as f64
}

/// `superflux(L, lo, hi, lag)`: the positive change of each bin of the band `lo` to `hi` Hz
/// from `lag` frames before, against the loudest of its neighbours then (Böck and Widmer 2013),
/// averaged over the band. The first `lag` frames are 0.
pub fn superflux(l: &Spectrogram, lo: f64, hi: f64, lag: usize) -> Vec<f64> {
    let (lo, hi) = (hz_bin(lo, N), hz_bin(hi, N));
    let width = hi - lo;
    let (mut buf, mut reference) = (Vec::new(), vec![0.0f32; width]);
    let mut out = vec![0.0; l.frames];
    for t in 0..l.frames.saturating_sub(lag) {
        maximum_filter1d(&l.row(t)[lo..hi], 3, &mut buf, &mut reference);
        let now = &l.row(t + lag)[lo..hi];
        out[t + lag] = mean_f32(
            now.iter().zip(&reference).map(|(a, r)| (a - r).max(0.0)),
            width,
        );
    }
    out
}

/// `spectral_flux(L)`: the spectrum's raw change (over two frames) per frame, which `feel`
/// measures.
pub fn spectral_flux(l: &Spectrogram) -> Vec<f64> {
    let mut out = vec![0.0; l.frames];
    for (t, o) in out.iter_mut().enumerate().skip(2) {
        let sum: f64 = l
            .row(t)
            .iter()
            .zip(l.row(t - 2))
            .map(|(a, b)| f64::from((a - b).max(0.0)))
            .sum();
        *o = sum as f32 as f64;
    }
    out
}

/// What the HPSS pass leaves besides the log spectra.
struct Energies {
    p: Vec<f64>,
    h: Vec<f64>,
    low: Vec<f64>,
    mid: Vec<f64>,
    high: Vec<f64>,
}

/// The median of each bin over `MEDIAN_SIZE` frames (the harmonic part stands out).
fn median_over_time(s: &Spectrogram) -> Spectrogram {
    let mut out = Spectrogram {
        frames: s.frames,
        bins: s.bins,
        data: vec![0.0; s.data.len()],
    };
    let (mut col, mut res) = (vec![0.0f32; s.frames], vec![0.0f32; s.frames]);
    let (mut buf, mut sorted) = (Vec::new(), Vec::new());
    for b in 0..s.bins {
        for (c, v) in col.iter_mut().zip(s.data.iter().skip(b).step_by(s.bins)) {
            *c = *v;
        }
        median_filter1d(&col, MEDIAN_SIZE, &mut buf, &mut sorted, &mut res);
        for (o, v) in out.data.iter_mut().skip(b).step_by(s.bins).zip(&res) {
            *o = *v;
        }
    }
    out
}

/// The median over `MEDIAN_SIZE` bins of each frame (the percussive part stands out).
fn median_over_bins(s: &Spectrogram) -> Spectrogram {
    let mut out = Spectrogram {
        frames: s.frames,
        bins: s.bins,
        data: vec![0.0; s.data.len()],
    };
    let (mut buf, mut sorted) = (Vec::new(), Vec::new());
    for t in 0..s.frames {
        let range = t * s.bins..(t + 1) * s.bins;
        median_filter1d(
            s.row(t),
            MEDIAN_SIZE,
            &mut buf,
            &mut sorted,
            &mut out.data[range],
        );
    }
    out
}

/// Harmonic/percussive separation (Fitzgerald 2010) and the log spectra, in place: `s` becomes
/// `L = log1p(1000 S)`, the returned two `LP` and `LH` of the percussive and harmonic parts
/// (the Python's `L, LP, LH`), with the energies of the parts.
fn hpss(mut s: Spectrogram) -> (Spectrogram, Spectrogram, Spectrogram, Energies) {
    let mut hm = median_over_time(&s);
    let mut pm = median_over_bins(&s);
    let bins = s.bins;
    let (low_end, high_start) = (hz_bin(160.0, N), hz_bin(5000.0, N));
    let mid_end = hz_bin(4000.0, N);
    let mut e = Energies {
        p: vec![0.0; s.frames],
        h: vec![0.0; s.frames],
        low: vec![0.0; s.frames],
        mid: vec![0.0; s.frames],
        high: vec![0.0; s.frames],
    };
    for t in 0..s.frames {
        let (mut sum_p, mut sum_h, mut low, mut mid, mut high) =
            (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for b in 0..bins {
            let i = t * bins + b;
            let (v, h2, p2) = (s.data[i], hm.data[i] * hm.data[i], pm.data[i] * pm.data[i]);
            let mask = h2 / (h2 + p2 + EPS32);
            let (h, p) = (v * mask, v * (1.0 - mask));
            let p_energy = f64::from(p * p);
            sum_p += p_energy;
            sum_h += f64::from(h * h);
            if b < low_end {
                low += p_energy;
            } else if (low_end..mid_end).contains(&b) {
                mid += p_energy;
            }
            if b >= high_start {
                high += p_energy;
            }
            s.data[i] = (COMPRESSION * v).ln_1p();
            hm.data[i] = (COMPRESSION * h).ln_1p();
            pm.data[i] = (COMPRESSION * p).ln_1p();
        }
        e.p[t] = sum_p as f32 as f64;
        e.h[t] = sum_h as f32 as f64;
        e.low[t] = low as f32 as f64;
        e.mid[t] = mid as f32 as f64;
        e.high[t] = high as f32 as f64;
    }
    (s, pm, hm, e)
}

/// The curves `chroma_timbre` makes per coarse frame.
pub struct ChromaTimbre {
    pub chroma: Vec<[f64; 12]>,
    pub timbre: Vec<[f64; TIMBRE_COEFFS]>,
    pub loud: Vec<f64>,
    pub cent: Vec<f64>,
}

/// `chroma_timbre(C)`: per coarse frame the 12 chroma (each bin from 80 Hz to 5 kHz votes for
/// its pitch class), 12 timbre coefficients (log energy in 24 log-spaced bands, then a DCT),
/// the loudness (dB) and the centroid (Hz).
pub fn chroma_timbre(c: &Spectrogram) -> ChromaTimbre {
    let f: Vec<f64> = (0..c.bins).map(|i| (i * SR) as f64 / CN as f64).collect();
    // The pitch class of the bins that vote, MIDI number mod 12.
    let pitch_class: Vec<Option<usize>> = f
        .iter()
        .map(|&hz| {
            (CHROMA_RANGE.0..=CHROMA_RANGE.1).contains(&hz).then(|| {
                (round_half_even(12.0 * (hz / 440.0).log2() + 69.0) as i64).rem_euclid(12) as usize
            })
        })
        .collect();
    let edges = geomspace(TIMBRE_RANGE.0, TIMBRE_RANGE.1, TIMBRE_BANDS + 1);
    let band_of: Vec<Option<usize>> = f
        .iter()
        .map(|&hz| (0..TIMBRE_BANDS).find(|&k| hz >= edges[k] && hz < edges[k + 1]))
        .collect();
    let dct: Vec<Vec<f64>> = (1..=TIMBRE_COEFFS)
        .map(|j| {
            (0..TIMBRE_BANDS)
                .map(|k| (std::f64::consts::PI / 24.0 * (k as f64 + 0.5) * j as f64).cos())
                .collect()
        })
        .collect();
    let mut out = ChromaTimbre {
        chroma: Vec::with_capacity(c.frames),
        timbre: Vec::with_capacity(c.frames),
        loud: Vec::with_capacity(c.frames),
        cent: Vec::with_capacity(c.frames),
    };
    for t in 0..c.frames {
        let (mut chroma, mut bands) = ([0.0f64; 12], [0.0f64; TIMBRE_BANDS]);
        let (mut total, mut weighted) = (0.0f64, 0.0f64);
        for (b, &v) in c.row(t).iter().enumerate() {
            let pw = v * v;
            total += f64::from(pw);
            weighted += f64::from(pw) * f[b];
            if let Some(pc) = pitch_class[b] {
                chroma[pc] += f64::from((CHROMA_COMPRESSION * pw).ln_1p());
            }
            if let Some(k) = band_of[b] {
                bands[k] += f64::from(pw);
            }
        }
        let chroma = chroma.map(|v| v as f32);
        let norm = chroma
            .iter()
            .map(|&v| f64::from(v) * f64::from(v))
            .sum::<f64>()
            .sqrt();
        let norm = (norm as f32 + EPS32) as f64;
        out.chroma.push(chroma.map(|v| (v / norm as f32) as f64));
        let log_bands = bands.map(|v| (v as f32 + TIMBRE_FLOOR).ln());
        out.timbre.push(std::array::from_fn(|j| {
            log_bands
                .iter()
                .zip(&dct[j])
                .map(|(&l, w)| f64::from(l) * w)
                .sum()
        }));
        let total = total as f32;
        out.loud
            .push(f64::from(10.0 * (total + LOUD_FLOOR).log10()));
        out.cent.push(weighted / f64::from(total + EPS32));
    }
    out
}

/// `front(x)` of the Python.
pub fn front(x: &[f32]) -> Front {
    let s = stft(x, N, HOP);
    let (l, lp, lh, e) = hpss(s);
    let coarse = chroma_timbre(&stft(x, CN, CHOP));
    Front {
        samples: x.len(),
        raw_low: superflux(&lp, 30.0, 160.0, 2),
        raw_mid: superflux(&lp, 160.0, 4000.0, 2),
        raw_high: superflux(&lp, 5000.0, 11000.0, 2),
        raw_harmonic: superflux(&lh, 250.0, 5000.0, 4),
        band_low: superflux(&l, 30.0, 160.0, 2),
        band_mid: superflux(&l, 160.0, 4000.0, 2),
        band_high: superflux(&l, 5000.0, 11000.0, 2),
        flux: spectral_flux(&l),
        chroma: coarse.chroma,
        timbre: coarse.timbre,
        loud: coarse.loud,
        cent: coarse.cent,
        energy_p: e.p,
        energy_h: e.h,
        share_low: e.low,
        share_mid: e.mid,
        share_high: e.high,
    }
}
