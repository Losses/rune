//! The front end, a port of the Python prototype's `front()`: the part of the analysis that
//! looks only at its own neighbourhood in time, so it is computed once for a whole song and
//! cut at any point. The spectra, the percussive and harmonic split, the raw onset curves, the
//! chroma and timbre, the raw energies. Nothing here is scaled against the song or decided
//! over it, that is `back`.
//!
//! The Python holds the whole song's spectrograms; this streams. `FrontBuilder` takes the
//! samples as they come, keeps only the window the STFT needs, and runs the spectra through the
//! harmonic/percussive split and the onset curves a block of frames at a time, with the margin
//! every local operation reaches over (the medians, the 3-bin maximum, the flux's lag) kept
//! from the block before and borrowed from the block after. The two ends of the song are
//! handled as the whole-array version does: scipy's `reflect` where a median runs off the
//! first or last frame, numpy's zero padding in the STFT. The result is bit for bit the
//! whole-array one (`front_whole`, which only the tests have, is compared with several block
//! sizes), so only the curves that are small stay whole: nothing holds a spectrogram of the song.
//!
//! The spectra are `f32`, as in numpy; the curves made from them are stored as `f64` holding
//! the `f32` values the Python has, and everything after is `f64`.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

use crate::ndimage::{maximum_filter1d, median_filter1d};
use crate::np::{geomspace, hanning, pairwise_sum, round_half_even};

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
pub(crate) const EPS32: f32 = 1e-9;

/// How many frames the harmonic median covers over time (80 ms), and the percussive median over
/// frequency (what the HPSS mask is made from).
pub(crate) const MEDIAN_SIZE: usize = 17;
/// `log1p(COMPRESSION * amplitude)` of the spectra.
pub(crate) const COMPRESSION: f32 = 1000.0;
/// The lowest and highest frequency (Hz) whose bins vote for a pitch class in the chroma.
pub(crate) const CHROMA_RANGE: (f64, f64) = (80.0, 5000.0);
/// Compression of a bin's power before it votes for its pitch class.
pub(crate) const CHROMA_COMPRESSION: f32 = 100.0;
/// The timbre's log-spaced bands: from, to (Hz) and count, and how many DCT coefficients it keeps.
pub(crate) const TIMBRE_RANGE: (f64, f64) = (60.0, 10000.0);
pub(crate) const TIMBRE_BANDS: usize = 24;
pub(crate) const TIMBRE_COEFFS: usize = 12;
/// Keeps the log of an empty timbre band finite.
pub(crate) const TIMBRE_FLOOR: f32 = 1e-8;
/// Keeps the log of a silent frame's loudness finite.
pub(crate) const LOUD_FLOOR: f32 = 1e-10;

/// How many frames of the fine spectra the front end works through at a time.
pub const BLOCK: usize = 256;
/// The frames either side of a block the harmonic median reaches (half its window).
const MARGIN: usize = MEDIAN_SIZE / 2;
/// The longest lag of the onset curves: that many frames of the log spectra are kept from one
/// block to the next.
const MAX_LAG: usize = 4;
/// Samples the STFT takes at a time, so that a long slice does not make a long buffer.
const PUSH: usize = 8192;

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

/// A windowed spectrum made as the samples come: `Stft` is `stft(x, n, hop)` of the Python, one
/// row of `n / 2 + 1` magnitudes per frame (the signal padded by `n / 2` zeros each side,
/// frames centred every `hop` samples, magnitudes divided by `n / 4`), and keeps only the
/// samples the next frame still needs.
pub struct Stft {
    n: usize,
    hop: usize,
    win: Vec<f32>,
    scale: f32,
    fft: Arc<dyn RealToComplex<f64>>,
    input: Vec<f64>,
    output: Vec<Complex<f64>>,
    scratch: Vec<Complex<f64>>,
    row: Vec<f32>,
    /// The samples from the song's `base`-th on, as far as they were pushed.
    buf: Vec<f32>,
    base: usize,
    /// The next frame to make, and how many samples were pushed.
    next: usize,
    len: usize,
}

impl Stft {
    pub fn new(n: usize, hop: usize) -> Stft {
        let fft = RealFftPlanner::<f64>::new().plan_fft_forward(n);
        Stft {
            n,
            hop,
            win: hanning(n).iter().map(|&w| w as f32).collect(),
            scale: (n / 4) as f32,
            input: fft.make_input_vec(),
            output: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
            row: Vec::with_capacity(n / 2 + 1),
            buf: Vec::new(),
            base: 0,
            next: 0,
            len: 0,
        }
    }

    /// Makes frame `t` into `row`; the samples it covers outside the song are 0.
    fn frame(&mut self, t: usize) {
        let start = (t * self.hop) as isize - (self.n / 2) as isize - self.base as isize;
        for (j, v) in self.input.iter_mut().enumerate() {
            let i = start + j as isize;
            let s = if i >= 0 && (i as usize) < self.buf.len() {
                self.buf[i as usize]
            } else {
                0.0
            };
            *v = (s * self.win[j]) as f64;
        }
        let done =
            self.fft
                .process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch);
        debug_assert!(done.is_ok(), "the buffers are the plan's own");
        self.row.clear();
        let scale = self.scale;
        self.row
            .extend(self.output.iter().map(|c| c.norm() as f32 / scale));
    }

    /// Takes the next samples of the song, and calls `emit` with the row of each frame they
    /// completed.
    pub fn push(&mut self, x: &[f32], emit: &mut impl FnMut(&[f32])) {
        for part in x.chunks(PUSH) {
            self.buf.extend_from_slice(part);
            self.len += part.len();
            while self.next * self.hop + self.n / 2 <= self.len {
                self.frame(self.next);
                emit(&self.row);
                self.next += 1;
            }
            let needed = (self.next * self.hop).saturating_sub(self.n / 2);
            if needed > self.base {
                self.buf.drain(..needed - self.base);
                self.base = needed;
            }
        }
    }

    /// The song is over: calls `emit` with the rest of the frames, the last ones over the zeros
    /// past its end. There are `ceil((samples + 1) / hop)` in all.
    pub fn finish(&mut self, emit: &mut impl FnMut(&[f32])) {
        let frames = (self.len + 1).div_ceil(self.hop);
        while self.next < frames {
            self.frame(self.next);
            emit(&self.row);
            self.next += 1;
        }
    }
}

/// `hz_bin(f, n)`: the bin of `f` Hz in an `n`-point spectrum (Python's `round`).
pub fn hz_bin(f: f64, n: usize) -> usize {
    round_half_even(f * n as f64 / SR as f64) as usize
}

/// `np.mean` of a `float32` row: summed pairwise in `f32`, divided in `f32`.
pub(crate) fn mean_f32(values: &[f32]) -> f64 {
    f64::from(pairwise_sum(values) / values.len() as f32)
}

/// The sums `hpss` takes of a frame's percussive and harmonic energies per bin (`np.sum` of a
/// `float32` row, pairwise in `f32`): the percussive and harmonic ones in all, and the
/// percussive one below 160 Hz, from 160 Hz to 4 kHz and from 5 kHz.
pub(crate) fn energy_sums(p: &[f32], h: &[f32]) -> [f64; 5] {
    let (low_end, mid_end, high_start) = (hz_bin(160.0, N), hz_bin(4000.0, N), hz_bin(5000.0, N));
    [
        pairwise_sum(p),
        pairwise_sum(h),
        pairwise_sum(&p[..low_end]),
        pairwise_sum(&p[low_end..mid_end]),
        pairwise_sum(&p[high_start..]),
    ]
    .map(f64::from)
}

/// Rows of `bins` values, the frames from `first` on: a spectrogram's window in time.
struct Rows {
    bins: usize,
    first: usize,
    data: Vec<f32>,
}

impl Rows {
    fn new(bins: usize) -> Rows {
        Rows {
            bins,
            first: 0,
            data: Vec::new(),
        }
    }

    /// One past the last frame held.
    fn end(&self) -> usize {
        self.first + self.data.len() / self.bins
    }

    fn row(&self, t: usize) -> &[f32] {
        let at = (t - self.first) * self.bins;
        &self.data[at..at + self.bins]
    }

    /// Forgets the frames before `t`.
    fn drop_before(&mut self, t: usize) {
        if t > self.first {
            self.data.drain(..(t - self.first) * self.bins);
            self.first = t;
        }
    }
}

/// Scratch the maximum filter of the onset curves reuses.
#[derive(Default)]
struct FluxScratch {
    buf: Vec<f32>,
    reference: Vec<f32>,
    change: Vec<f32>,
}

/// `superflux(L, lo, hi, lag)`, made block by block: the positive change of each bin of the
/// band `lo` to `hi` Hz from `lag` frames before, against the loudest of its neighbours then
/// (Böck and Widmer 2013), averaged over the band. The first `lag` frames are 0.
struct Superflux {
    lo: usize,
    hi: usize,
    lag: usize,
    out: Vec<f64>,
}

impl Superflux {
    fn new(lo: f64, hi: f64, lag: usize) -> Superflux {
        debug_assert!(lag <= MAX_LAG);
        Superflux {
            lo: hz_bin(lo, N),
            hi: hz_bin(hi, N),
            lag,
            out: Vec::new(),
        }
    }

    /// Appends the values of frames `from..to` of `l`, which holds the `lag` frames before
    /// `from` as well.
    fn push(&mut self, l: &Rows, from: usize, to: usize, scratch: &mut FluxScratch) {
        let width = self.hi - self.lo;
        scratch.reference.resize(width, 0.0);
        for t in from..to {
            if t < self.lag {
                self.out.push(0.0);
                continue;
            }
            let before = &l.row(t - self.lag)[self.lo..self.hi];
            maximum_filter1d(before, 3, &mut scratch.buf, &mut scratch.reference);
            let now = &l.row(t)[self.lo..self.hi];
            scratch.change.clear();
            scratch.change.extend(
                now.iter()
                    .zip(&scratch.reference)
                    .map(|(a, r)| (a - r).max(0.0)),
            );
            self.out.push(mean_f32(&scratch.change));
        }
    }
}

/// `spectral_flux(L)`, appended for frames `from..to` of `l` (which holds two frames before
/// `from` as well): the spectrum's raw change (over two frames) per frame, which `feel` measures.
fn push_spectral_flux(out: &mut Vec<f64>, l: &Rows, from: usize, to: usize) {
    for t in from..to {
        if t < 2 {
            out.push(0.0);
            continue;
        }
        let change: Vec<f32> = l
            .row(t)
            .iter()
            .zip(l.row(t - 2))
            .map(|(a, b)| (a - b).max(0.0))
            .collect();
        out.push(f64::from(pairwise_sum(&change)));
    }
}

/// The energies of the percussive and harmonic parts, per frame.
#[derive(Default)]
struct Energies {
    p: Vec<f64>,
    h: Vec<f64>,
    low: Vec<f64>,
    mid: Vec<f64>,
    high: Vec<f64>,
}

/// The median of each bin over `MEDIAN_SIZE` frames of `rows` (the harmonic part stands out).
fn median_over_time(rows: &[f32], bins: usize) -> Vec<f32> {
    let frames = rows.len() / bins;
    let mut out = vec![0.0; rows.len()];
    let (mut col, mut res) = (vec![0.0f32; frames], vec![0.0f32; frames]);
    let (mut buf, mut sorted) = (Vec::new(), Vec::new());
    for b in 0..bins {
        for (c, v) in col.iter_mut().zip(rows.iter().skip(b).step_by(bins)) {
            *c = *v;
        }
        median_filter1d(&col, MEDIAN_SIZE, &mut buf, &mut sorted, &mut res);
        for (o, v) in out.iter_mut().skip(b).step_by(bins).zip(&res) {
            *o = *v;
        }
    }
    out
}

/// The median over `MEDIAN_SIZE` bins of each frame of `rows` (the percussive part stands out).
fn median_over_bins(rows: &[f32], bins: usize) -> Vec<f32> {
    let mut out = vec![0.0; rows.len()];
    let (mut buf, mut sorted) = (Vec::new(), Vec::new());
    for (row, res) in rows.chunks(bins).zip(out.chunks_mut(bins)) {
        median_filter1d(row, MEDIAN_SIZE, &mut buf, &mut sorted, res);
    }
    out
}

/// Harmonic/percussive separation (Fitzgerald 2010) and the log spectra, a block of frames at a
/// time, and the onset curves made from them. The spectra `s` come in; the frames of a block
/// are made once the `MARGIN` frames after it have come (or the song is over), from the
/// `MARGIN` frames before it too.
struct Hpss {
    block: usize,
    /// The magnitude spectra from the first frame the next block's median reaches.
    s: Rows,
    /// The next frame to make.
    done: usize,
    /// `L = log1p(1000 S)` and the percussive and harmonic parts' `LP` and `LH`, the last
    /// `MAX_LAG` frames made (and the block just made).
    l: Rows,
    lp: Rows,
    lh: Rows,
    e: Energies,
    raw_low: Superflux,
    raw_mid: Superflux,
    raw_high: Superflux,
    raw_harmonic: Superflux,
    band_low: Superflux,
    band_mid: Superflux,
    band_high: Superflux,
    flux: Vec<f64>,
    scratch: FluxScratch,
}

impl Hpss {
    fn new(block: usize) -> Hpss {
        let bins = N / 2 + 1;
        Hpss {
            block,
            s: Rows::new(bins),
            done: 0,
            l: Rows::new(bins),
            lp: Rows::new(bins),
            lh: Rows::new(bins),
            e: Energies::default(),
            raw_low: Superflux::new(30.0, 160.0, 2),
            raw_mid: Superflux::new(160.0, 4000.0, 2),
            raw_high: Superflux::new(5000.0, 11000.0, 2),
            raw_harmonic: Superflux::new(250.0, 5000.0, 4),
            band_low: Superflux::new(30.0, 160.0, 2),
            band_mid: Superflux::new(160.0, 4000.0, 2),
            band_high: Superflux::new(5000.0, 11000.0, 2),
            flux: Vec::new(),
            scratch: FluxScratch::default(),
        }
    }

    /// Takes the next frame's spectrum.
    fn push(&mut self, row: &[f32]) {
        self.s.data.extend_from_slice(row);
        self.run(false);
    }

    /// Makes the blocks that can be made: all the frames when the song is over, else whole
    /// blocks of those with their margin after them.
    fn run(&mut self, over: bool) {
        loop {
            let have = self.s.end();
            let ready = if over {
                have
            } else {
                have.saturating_sub(MARGIN)
            };
            let todo = ready.saturating_sub(self.done);
            if todo == 0 || (!over && todo < self.block) {
                break;
            }
            let to = self.done + todo.min(self.block);
            self.make(self.done, to);
            self.done = to;
        }
    }

    /// Frames `from..to` of everything.
    fn make(&mut self, from: usize, to: usize) {
        let bins = self.s.bins;
        // The spectra the medians of these frames read: running off the song's first or last
        // frame is the one place the median reflects, and the chunk ends there too.
        let lo = from.saturating_sub(MARGIN);
        let hi = (to + MARGIN).min(self.s.end());
        let chunk = &self.s.data[(lo - self.s.first) * bins..(hi - self.s.first) * bins];
        let hm = median_over_time(chunk, bins);
        let pm = median_over_bins(&chunk[(from - lo) * bins..(to - lo) * bins], bins);
        let (mut energy_p, mut energy_h) = (vec![0.0f32; bins], vec![0.0f32; bins]);
        for t in from..to {
            for b in 0..bins {
                let i = (t - lo) * bins + b;
                let (v, h2, p2) = (
                    chunk[i],
                    hm[i] * hm[i],
                    pm[i - (from - lo) * bins] * pm[i - (from - lo) * bins],
                );
                let mask = h2 / (h2 + p2 + EPS32);
                let (h, p) = (v * mask, v * (1.0 - mask));
                (energy_p[b], energy_h[b]) = (p * p, h * h);
                self.l.data.push((COMPRESSION * v).ln_1p());
                self.lp.data.push((COMPRESSION * p).ln_1p());
                self.lh.data.push((COMPRESSION * h).ln_1p());
            }
            let [p, h, low, mid, high] = energy_sums(&energy_p, &energy_h);
            self.e.p.push(p);
            self.e.h.push(h);
            self.e.low.push(low);
            self.e.mid.push(mid);
            self.e.high.push(high);
        }
        let s = &mut self.scratch;
        self.raw_low.push(&self.lp, from, to, s);
        self.raw_mid.push(&self.lp, from, to, s);
        self.raw_high.push(&self.lp, from, to, s);
        self.raw_harmonic.push(&self.lh, from, to, s);
        self.band_low.push(&self.l, from, to, s);
        self.band_mid.push(&self.l, from, to, s);
        self.band_high.push(&self.l, from, to, s);
        push_spectral_flux(&mut self.flux, &self.l, from, to);
        let keep = to.saturating_sub(MAX_LAG);
        for rows in [&mut self.l, &mut self.lp, &mut self.lh] {
            rows.drop_before(keep);
        }
        self.s.drop_before(to.saturating_sub(MARGIN));
    }
}

/// The bins of each timbre band, as ranges (the bands are runs of bins): `band_of` says which
/// band each bin is in.
pub(crate) fn band_ranges(band_of: &[Option<usize>]) -> [std::ops::Range<usize>; TIMBRE_BANDS] {
    std::array::from_fn(|k| {
        let first = band_of.iter().position(|&b| b == Some(k));
        let last = band_of.iter().rposition(|&b| b == Some(k));
        first.zip(last).map_or(0..0, |(a, b)| a..b + 1)
    })
}

/// The sums of `chroma_timbre` over a coarse frame's spectrum `row`, as numpy makes them (`sum`
/// of a `float32` row is pairwise in `f32`, of the `float64` products with the frequencies
/// `f` pairwise in `f64`): the power in all, the power weighted by the bins' frequencies, and
/// the power in each timbre band.
pub(crate) fn power_sums(
    row: &[f32],
    f: &[f64],
    bands: &[std::ops::Range<usize>; TIMBRE_BANDS],
) -> (f32, f64, [f32; TIMBRE_BANDS]) {
    let pw: Vec<f32> = row.iter().map(|&v| v * v).collect();
    let weighted: Vec<f64> = pw
        .iter()
        .zip(f)
        .map(|(&p, &hz)| f64::from(p) * hz)
        .collect();
    let in_band = std::array::from_fn(|k| pairwise_sum(&pw[bands[k].clone()]));
    (pairwise_sum(&pw), pairwise_sum(&weighted), in_band)
}

/// The curves `chroma_timbre` makes per coarse frame.
pub struct ChromaTimbre {
    pub chroma: Vec<[f64; 12]>,
    pub timbre: Vec<[f64; TIMBRE_COEFFS]>,
    pub loud: Vec<f64>,
    pub cent: Vec<f64>,
}

/// `chroma_timbre(C)`, a coarse frame at a time: the 12 chroma (each bin from 80 Hz to 5 kHz
/// votes for its pitch class), 12 timbre coefficients (log energy in 24 log-spaced bands, then
/// a DCT), the loudness (dB) and the centroid (Hz).
struct ChromaTimbreStage {
    f: Vec<f64>,
    /// The pitch class of the bins that vote, MIDI number mod 12.
    pitch_class: Vec<Option<usize>>,
    bands: [std::ops::Range<usize>; TIMBRE_BANDS],
    dct: Vec<Vec<f64>>,
    out: ChromaTimbre,
}

impl ChromaTimbreStage {
    fn new() -> ChromaTimbreStage {
        let f: Vec<f64> = (0..CN / 2 + 1)
            .map(|i| (i * SR) as f64 / CN as f64)
            .collect();
        let pitch_class = f
            .iter()
            .map(|&hz| {
                (CHROMA_RANGE.0..=CHROMA_RANGE.1).contains(&hz).then(|| {
                    (round_half_even(12.0 * (hz / 440.0).log2() + 69.0) as i64).rem_euclid(12)
                        as usize
                })
            })
            .collect();
        let edges = geomspace(TIMBRE_RANGE.0, TIMBRE_RANGE.1, TIMBRE_BANDS + 1);
        let band_of: Vec<Option<usize>> = f
            .iter()
            .map(|&hz| (0..TIMBRE_BANDS).find(|&k| hz >= edges[k] && hz < edges[k + 1]))
            .collect();
        let dct = (1..=TIMBRE_COEFFS)
            .map(|j| {
                (0..TIMBRE_BANDS)
                    .map(|k| (std::f64::consts::PI / 24.0 * (k as f64 + 0.5) * j as f64).cos())
                    .collect()
            })
            .collect();
        ChromaTimbreStage {
            f,
            pitch_class,
            bands: band_ranges(&band_of),
            dct,
            out: ChromaTimbre {
                chroma: Vec::new(),
                timbre: Vec::new(),
                loud: Vec::new(),
                cent: Vec::new(),
            },
        }
    }

    /// Takes the next coarse frame's spectrum.
    fn push(&mut self, row: &[f32]) {
        let mut chroma = [0.0f64; 12];
        for (b, &v) in row.iter().enumerate() {
            if let Some(pc) = self.pitch_class[b] {
                chroma[pc] += f64::from((CHROMA_COMPRESSION * (v * v)).ln_1p());
            }
        }
        let (total, weighted, bands) = power_sums(row, &self.f, &self.bands);
        let chroma = chroma.map(|v| v as f32);
        let norm = chroma
            .iter()
            .map(|&v| f64::from(v) * f64::from(v))
            .sum::<f64>()
            .sqrt();
        let norm = (norm as f32 + EPS32) as f64;
        self.out
            .chroma
            .push(chroma.map(|v| (v / norm as f32) as f64));
        let log_bands = bands.map(|v| (v + TIMBRE_FLOOR).ln());
        self.out.timbre.push(std::array::from_fn(|j| {
            log_bands
                .iter()
                .zip(&self.dct[j])
                .map(|(&l, w)| f64::from(l) * w)
                .sum()
        }));
        self.out
            .loud
            .push(f64::from(10.0 * (total + LOUD_FLOOR).log10()));
        self.out.cent.push(weighted / f64::from(total + EPS32));
    }
}

/// `front(x)` of the Python, made as the samples come: `push` them (any number at a time),
/// then `finish`.
pub struct FrontBuilder {
    fine: Stft,
    coarse: Stft,
    hpss: Hpss,
    timbre: ChromaTimbreStage,
    samples: usize,
}

impl Default for FrontBuilder {
    fn default() -> Self {
        Self::with_block(BLOCK)
    }
}

impl FrontBuilder {
    pub fn new() -> FrontBuilder {
        FrontBuilder::default()
    }

    /// A builder that makes `block` frames of the fine spectra at a time (at least 1), which
    /// changes the memory it uses and nothing it returns.
    pub fn with_block(block: usize) -> FrontBuilder {
        FrontBuilder {
            fine: Stft::new(N, HOP),
            coarse: Stft::new(CN, CHOP),
            hpss: Hpss::new(block.max(1)),
            timbre: ChromaTimbreStage::new(),
            samples: 0,
        }
    }

    /// The next samples of the song: mono, 22050 Hz.
    pub fn push(&mut self, x: &[f32]) {
        self.samples += x.len();
        self.fine.push(x, &mut |row| self.hpss.push(row));
        self.coarse.push(x, &mut |row| self.timbre.push(row));
    }

    /// The song is over: its `Front`.
    pub fn finish(mut self) -> Front {
        self.fine.finish(&mut |row| self.hpss.push(row));
        self.coarse.finish(&mut |row| self.timbre.push(row));
        self.hpss.run(true);
        let (h, c) = (self.hpss, self.timbre.out);
        Front {
            samples: self.samples,
            raw_low: h.raw_low.out,
            raw_mid: h.raw_mid.out,
            raw_high: h.raw_high.out,
            raw_harmonic: h.raw_harmonic.out,
            band_low: h.band_low.out,
            band_mid: h.band_mid.out,
            band_high: h.band_high.out,
            flux: h.flux,
            chroma: c.chroma,
            timbre: c.timbre,
            loud: c.loud,
            cent: c.cent,
            energy_p: h.e.p,
            energy_h: h.e.h,
            share_low: h.e.low,
            share_mid: h.e.mid,
            share_high: h.e.high,
        }
    }
}

/// `front(x)` of the Python.
pub fn front(x: &[f32]) -> Front {
    let mut builder = FrontBuilder::new();
    builder.push(x);
    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::front_whole::front_whole;

    /// A noisy signal with a few kinds of events, `seconds` long: a deterministic LCG for the
    /// noise, a tone that comes and goes and clicks, so every curve has something in it.
    fn signal(seconds: f64) -> Vec<f32> {
        let n = (seconds * SR as f64) as usize;
        let mut state = 12345u32;
        (0..n)
            .map(|i| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = (state >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                let t = i as f64 / SR as f64;
                let tone = (std::f64::consts::TAU * 440.0 * t).sin() * (t * 3.0).sin().abs();
                let click = if i % 5000 < 40 { 0.8 } else { 0.0 };
                (0.2 * tone) as f32 + 0.05 * noise + click
            })
            .collect()
    }

    fn assert_same(a: &Front, b: &Front) {
        assert_eq!(a.samples, b.samples);
        fn curves(f: &Front) -> Vec<&Vec<f64>> {
            vec![
                &f.raw_low,
                &f.raw_mid,
                &f.raw_high,
                &f.raw_harmonic,
                &f.band_low,
                &f.band_mid,
                &f.band_high,
                &f.flux,
                &f.loud,
                &f.cent,
                &f.energy_p,
                &f.energy_h,
                &f.share_low,
                &f.share_mid,
                &f.share_high,
            ]
        }
        for (i, (x, y)) in curves(a).into_iter().zip(curves(b)).enumerate() {
            assert_eq!(x, y, "curve {i}");
        }
        assert_eq!(a.chroma, b.chroma);
        assert_eq!(a.timbre, b.timbre);
    }

    #[test]
    fn blocks_give_the_whole_array_front_end_exactly() {
        let x = signal(3.0);
        let whole = front_whole(&x);
        // Blocks shorter than the margins, around them, and longer than the song.
        for block in [1, 3, 7, 8, 9, 16, 17, 100, 256, 5000] {
            let mut b = FrontBuilder::with_block(block);
            // Odd slices, so the STFT's buffer is cut everywhere.
            x.chunks(4099).for_each(|part| b.push(part));
            assert_same(&b.finish(), &whole);
        }
    }

    #[test]
    fn a_song_shorter_than_a_block_or_a_median_is_exact_too() {
        // 0 samples, one frame, a few frames (fewer than the median's 17) and a frame or two
        // past a block.
        for len in [0, 1, 100, 512, 1500, 3000, 6000, SR / 4] {
            let x = signal(1.0)[..len].to_vec();
            let whole = front_whole(&x);
            for block in [1, 4, 256] {
                let mut b = FrontBuilder::with_block(block);
                b.push(&x);
                assert_same(&b.finish(), &whole);
            }
        }
    }
}
