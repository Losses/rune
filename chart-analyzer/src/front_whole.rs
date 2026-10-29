//! The front end as one whole-array pass, as the Python has it: every spectrogram held whole, the
//! medians taken over all of it. It is what `front` (which streams) must give bit for bit, so
//! it lives only in the tests, which compare the two with several block sizes.

use crate::front::{
    CHOP, CHROMA_COMPRESSION, CHROMA_RANGE, CN, COMPRESSION, ChromaTimbre, EPS32, Front, HOP,
    LOUD_FLOOR, MEDIAN_SIZE, N, SR, TIMBRE_BANDS, TIMBRE_COEFFS, TIMBRE_FLOOR, TIMBRE_RANGE,
    band_ranges, energy_sums, hz_bin, mean_f32, power_sums,
};
use crate::ndimage::{maximum_filter1d, median_filter1d};
use crate::np::{geomspace, hanning, pairwise_sum, round_half_even};
use realfft::RealFftPlanner;

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
        let change: Vec<f32> = now
            .iter()
            .zip(&reference)
            .map(|(a, r)| (a - r).max(0.0))
            .collect();
        debug_assert_eq!(change.len(), width);
        out[t + lag] = mean_f32(&change);
    }
    out
}

/// `spectral_flux(L)`: the spectrum's raw change (over two frames) per frame, which `feel`
/// measures.
pub fn spectral_flux(l: &Spectrogram) -> Vec<f64> {
    let mut out = vec![0.0; l.frames];
    for (t, o) in out.iter_mut().enumerate().skip(2) {
        let change: Vec<f32> = l
            .row(t)
            .iter()
            .zip(l.row(t - 2))
            .map(|(a, b)| (a - b).max(0.0))
            .collect();
        *o = f64::from(pairwise_sum(&change));
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
    let mut e = Energies {
        p: vec![0.0; s.frames],
        h: vec![0.0; s.frames],
        low: vec![0.0; s.frames],
        mid: vec![0.0; s.frames],
        high: vec![0.0; s.frames],
    };
    for t in 0..s.frames {
        let (mut energy_p, mut energy_h) = (vec![0.0f32; bins], vec![0.0f32; bins]);
        for b in 0..bins {
            let i = t * bins + b;
            let (v, h2, p2) = (s.data[i], hm.data[i] * hm.data[i], pm.data[i] * pm.data[i]);
            let mask = h2 / (h2 + p2 + EPS32);
            let (h, p) = (v * mask, v * (1.0 - mask));
            (energy_p[b], energy_h[b]) = (p * p, h * h);
            s.data[i] = (COMPRESSION * v).ln_1p();
            hm.data[i] = (COMPRESSION * h).ln_1p();
            pm.data[i] = (COMPRESSION * p).ln_1p();
        }
        [e.p[t], e.h[t], e.low[t], e.mid[t], e.high[t]] = energy_sums(&energy_p, &energy_h);
    }
    (s, pm, hm, e)
}

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
    let bands = band_ranges(&band_of);
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
        let mut chroma = [0.0f64; 12];
        for (b, &v) in c.row(t).iter().enumerate() {
            if let Some(pc) = pitch_class[b] {
                chroma[pc] += f64::from((CHROMA_COMPRESSION * (v * v)).ln_1p());
            }
        }
        let (total, weighted, bands) = power_sums(c.row(t), &f, &bands);
        let chroma = chroma.map(|v| v as f32);
        let norm = chroma
            .iter()
            .map(|&v| f64::from(v) * f64::from(v))
            .sum::<f64>()
            .sqrt();
        let norm = (norm as f32 + EPS32) as f64;
        out.chroma.push(chroma.map(|v| (v / norm as f32) as f64));
        let log_bands = bands.map(|v| (v + TIMBRE_FLOOR).ln());
        out.timbre.push(std::array::from_fn(|j| {
            log_bands
                .iter()
                .zip(&dct[j])
                .map(|(&l, w)| f64::from(l) * w)
                .sum()
        }));
        out.loud
            .push(f64::from(10.0 * (total + LOUD_FLOOR).log10()));
        out.cent.push(weighted / f64::from(total + EPS32));
    }
    out
}

/// `front(x)` of the Python.
pub fn front_whole(x: &[f32]) -> Front {
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
