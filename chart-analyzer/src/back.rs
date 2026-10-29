//! The back end, a port of the Python prototype's `back()`: the decisions over a whole
//! song, taken from `front`'s curves. First the rhythm: the onset strengths, the tempo, the
//! beats and the meter with the downbeats (`rhythm`, which keeps what the rest needs in
//! `Rhythm`). Then the sections and phrase starts (`sections`), the event streams' peaks and the
//! lead (`streams`), and the curves.
//!
//! Where the Python's arrays are `float32` the values here are `f32`s held in `f64`s and the
//! arithmetic that matters for the numbers it writes is done in `f32`, see each function.

use crate::chart::{Beat, Chart, Curves, Meter, Tempo as ChartTempo};
use crate::front::{CFPS, CHOP, EPS, EPS32, FPS, Front, SR};
use crate::ndimage::{gaussian_filter1d, uniform_filter1d};
use crate::np::{floor_div, interp, mean, median, modulo, percentile, round_to, searchsorted, std};
use crate::sections::{SectionInput, feature_stack, phrase_starts, sections};
use crate::streams::{GAPS, lead_stream, pick};
use crate::tempo::{dp_beats, feel, near_max, tempo, tempo_window};

/// A prefix shorter than this is too short for the tempo windows (8 s, see `window_acfs`).
pub const MIN_SECONDS: f64 = 6.0;

/// The width (frames) of the Gaussian that smooths each raw onset curve.
const ODF_SIGMA: f64 = 1.0;
/// The percentile of a curve that reads 1: a song's strong onsets read about 1.
const SCALE_PERCENTILE: f64 = 98.0;
/// No onset reads more than this.
const ODF_CEILING: f64 = 1.5;
/// The window (s) a band's level is measured against, for the beat's onset strength.
const LOCAL_SECONDS: f64 = 3.0;
/// A band's local level is at least this fraction of its mean over the song.
const LOCAL_FLOOR: f64 = 0.1;
/// Calibrated on the built and rubato test songs, see evaluate.py. Pulse clarity (the
/// autocorrelation peak) from this much counts a little as a pulse, and fully `CLARITY_RANGE`
/// above.
const CLARITY_FROM: f64 = 0.1;
const CLARITY_RANGE: f64 = 0.25;
/// Below this pulse (the 75th percentile of the windows' clarity) the song has no beats.
const MIN_PULSE: f64 = 0.3;
/// The percentile of the windows' pulse that is the song's.
const PULSE_PERCENTILE: f64 = 75.0;
/// A step of more than this many periods between beats has a beat missing.
const SKIPPED_BEAT: f64 = 1.5;
/// How many beats the half-beat check compares, either side.
const HALF_BEAT_SPAN: usize = 16;
/// The half-beat check needs more than this many beats.
const HALF_BEAT_MIN: usize = 8;
/// Beats sit on 10 ms frames: span this many to read the tempo finely.
const BPM_SPAN: usize = 16;
/// Kick and snare strength are read within this many frames of a beat.
const NEAR: usize = 3;
/// Meter and downbeats need more beats than this.
const METER_MIN_BEATS: usize = 16;
/// The meters tried, with their priors (a bar of three is less likely than four).
const METERS: [(usize, f64); 2] = [(4, 1.0), (3, 0.8)];
/// What each feature says about a downbeat, per beat: harmony change, kick, snare, loudness rise.
/// A bar starts where the harmony changes and the kick lands; the snare's backbeat falls between
/// downbeats.
const DOWNBEAT_WEIGHTS: [f64; 4] = [1.0, 0.7, -0.7, 0.3];
/// How far the best meter and downbeat phase must lead the next to be sure.
const METER_MARGIN: f64 = 0.6;
/// A beat's strength reads 1 at this much onset strength.
const STRENGTH_FULL: f64 = 3.0;
/// The width (s) of the Gaussian that smooths the percussive and harmonic energy for the share.
const PERCUSSIVE_SIGMA: f64 = 0.1;
/// The brightness curve spans these percentiles of the song's moments with sound, which are the
/// ones louder than `SOUND` (0 to 1).
const BRIGHT_LOW_PERCENTILE: f64 = 5.0;
const BRIGHT_HIGH_PERCENTILE: f64 = 95.0;
const SOUND: f32 = 0.1;
/// The loudness curve spans the 5th to the 99th percentile of the song, at most this many dB.
const LOUD_TOP_PERCENTILE: f64 = 99.0;
const LOUD_FLOOR_PERCENTILE: f64 = 5.0;
const LOUD_RANGE_DB: f64 = 40.0;

/// How many fine and coarse frames of a front-end result lie wholly inside the first `samples`
/// samples (all of them for the whole song), so that no frame has heard past the cut. Returns
/// the samples, the fine frames and the coarse frames.
pub fn cut(f: &Front, samples: Option<usize>) -> (usize, usize, usize) {
    match samples {
        Some(s) if s < f.samples => {
            let frames = |n: usize, hop: usize| {
                (floor_div(s as i64 - (n / 2) as i64, hop as i64) + 1).max(0) as usize
            };
            (
                s,
                frames(crate::front::N, crate::front::HOP),
                frames(crate::front::CN, crate::front::CHOP),
            )
        }
        _ => (f.samples, f.flux.len(), f.loud.len()),
    }
}

/// `scale01(x, q)` of `f32` values (held in `f64`s): `x` divided by its q-th percentile, so a
/// song's strong onsets read about 1. The division is in `f32`, as numpy does it with a
/// `float32` array and a `float64` scalar.
pub fn scale01(x: &[f64], q: f64) -> Vec<f64> {
    let reference = if x.iter().any(|&v| v > 0.0) {
        percentile(x, q)
    } else {
        1.0
    };
    let divisor = (reference + EPS) as f32;
    x.iter().map(|&v| f64::from(v as f32 / divisor)).collect()
}

/// The onset streams, each scaled to its own song.
pub struct Odfs {
    pub low: Vec<f64>,
    pub mid: Vec<f64>,
    pub high: Vec<f64>,
    pub harmonic: Vec<f64>,
}

/// The onset streams of the first `nf` frames: low, mid, high percussive and harmonic note
/// onsets, smoothed (a `float32` result), scaled to the song and capped.
pub fn odf_streams(f: &Front, nf: usize) -> Odfs {
    let one = |raw: &[f64]| -> Vec<f64> {
        let smooth: Vec<f64> = gaussian_filter1d(&raw[..nf], ODF_SIGMA)
            .iter()
            .map(|&v| f64::from(v as f32))
            .collect();
        scale01(&smooth, SCALE_PERCENTILE)
            .into_iter()
            .map(|v| v.min(ODF_CEILING))
            .collect()
    };
    Odfs {
        low: one(&f.raw_low),
        mid: one(&f.raw_mid),
        high: one(&f.raw_high),
        harmonic: one(&f.raw_harmonic),
    }
}

/// The onset strength for the beat: the whole-spectrum bands, each against its local level,
/// then the square root of their sum, in units of its standard deviation.
pub fn onset_strength(f: &Front, nf: usize) -> Vec<f64> {
    let size = (LOCAL_SECONDS * FPS) as usize;
    let mut sum = vec![0.0; nf];
    for band in [&f.band_low, &f.band_mid, &f.band_high] {
        let band = &band[..nf];
        let local = uniform_filter1d(band, size);
        let floor = LOCAL_FLOOR * mean(band) + EPS;
        for ((s, b), l) in sum.iter_mut().zip(band).zip(&local) {
            *s += b / l.max(floor);
        }
    }
    let odf: Vec<f64> = sum.iter().map(|v| v.sqrt()).collect();
    let s = std(&odf) + EPS;
    odf.iter().map(|v| v / s).collect()
}

/// What the rhythm half of `back()` finds, and 1b's sections, lead and curves need.
pub struct Rhythm {
    pub odfs: Odfs,
    /// The onset strength for the beat.
    pub odf: Vec<f64>,
    pub song_feel: f64,
    /// How fast the song sounds in each window and the windows' centres (s).
    pub feels: Vec<f64>,
    pub fcenters: Vec<f64>,
    /// Per fine frame, the beat period following the tempo through the song.
    pub period: Vec<f64>,
    /// The tempo windows' centres (s) and how clear the pulse is in each, 0 to 1.
    pub ccenters: Vec<f64>,
    pub conf_w: Vec<f64>,
    /// The song's pulse, 0 to 1.
    pub song_pulse: f64,
    pub tempo: ChartTempo,
    /// Beat times (s), empty when the pulse is too weak.
    pub beats_t: Vec<f64>,
    /// The coarse frames' loudness curve, 0 to 1, and their times.
    pub loud01: Vec<f64>,
    pub ctimes: Vec<f64>,
    pub meter: Meter,
    pub beats: Vec<Beat>,
}

/// The loudness curve of the first `nc` coarse frames, 0 to 1 against the song's own range. A
/// `float32` curve: the Python's `float32` loudness minus and over `float64` scalars stays
/// `float32`, the scalars cast to it.
fn loudness01(loud: &[f64]) -> Vec<f64> {
    let top = percentile(loud, LOUD_TOP_PERCENTILE);
    let floor_db = percentile(loud, LOUD_FLOOR_PERCENTILE).max(top - LOUD_RANGE_DB);
    let (floor, range) = (floor_db as f32, (top - floor_db + EPS) as f32);
    loud.iter()
        .map(|&l| f64::from(((l as f32 - floor) / range).clamp(0.0, 1.0)))
        .collect()
}

/// The beats (frame numbers) of the song: found by DP from the onset strength steered by kick
/// and snare, with the skipped beats put back and the beats moved if the half-beat points carry
/// more kick and snare.
fn find_beats(odf: &[f64], km: &[f64], period: &[f64]) -> Vec<usize> {
    let mut b = dp_beats(&period_steered(odf, km), period);
    // Where kick and snare fall on every other beat the path may step two beats at a time: put
    // the missing beats back, never slower than the tempo found above.
    if b.len() > 2 {
        let mut with: Vec<usize> = b.clone();
        for w in b.windows(2) {
            if (w[1] - w[0]) as f64 > SKIPPED_BEAT * period[w[0]] {
                with.push((w[0] + w[1]) / 2);
            }
        }
        with.sort_unstable();
        b = with;
    }
    // Where the half-beat points carry more kick and snare than the beats around them (16
    // beats either way), the beats there are off by half.
    let mids: Vec<usize> = b.windows(2).map(|w| (w[0] + w[1]) / 2).collect();
    if mids.len() > HALF_BEAT_MIN {
        let before = &b[..b.len() - 1];
        let on = uniform_filter1d(&near_max(km, before, NEAR), HALF_BEAT_SPAN);
        let off = uniform_filter1d(&near_max(km, &mids, NEAR), HALF_BEAT_SPAN);
        b = (0..mids.len())
            .map(|i| if off[i] > on[i] { mids[i] } else { before[i] })
            .collect();
    }
    b
}

/// The onset strength with kick and snare added: they sit on the beat more than hats do, so
/// they also steer where the beats go.
fn period_steered(odf: &[f64], km: &[f64]) -> Vec<f64> {
    let s = std(km) + EPS;
    odf.iter().zip(km).map(|(o, k)| o + k / s).collect()
}

/// The tempo in bpm from the beats' spacing over `BPM_SPAN` beats, `fallback` when there are
/// too few beats.
fn bpm_from_beats(beats_t: &[f64], fallback: f64) -> f64 {
    let span = BPM_SPAN.min(beats_t.len().saturating_sub(1));
    if span == 0 {
        return fallback;
    }
    let ibi: Vec<f64> = (span..beats_t.len())
        .map(|i| (beats_t[i] - beats_t[i - span]) / span as f64)
        .collect();
    60.0 / median(&ibi)
}

/// Meter and downbeat phase from kick strength and harmonic change per beat, and the beats with
/// their bar, position, strength and confidence.
fn meter_and_downbeats(f: &Front, nc: usize, r: &Rhythm) -> (Meter, Vec<Beat>) {
    let none = (
        Meter {
            beats_per_bar: 4,
            confidence: 0.0,
        },
        vec![],
    );
    let beats_t = &r.beats_t;
    if beats_t.len() <= METER_MIN_BEATS {
        return none;
    }
    let ctimes = &r.ctimes;
    let bi: Vec<usize> = beats_t
        .iter()
        .map(|&t| searchsorted(ctimes, t).min(ctimes.len() - 1))
        .collect();
    let chroma = &f.chroma[..nc];
    let mut bchroma: Vec<[f64; 12]> = Vec::with_capacity(bi.len());
    for (i, &a) in bi.iter().enumerate() {
        let c = bi.get(i + 1).copied().unwrap_or(ctimes.len());
        let frames = &chroma[a..c.max(a + 1)];
        let mut v = [0.0; 12];
        for fr in frames {
            v.iter_mut().zip(fr).for_each(|(x, y)| *x += y);
        }
        v.iter_mut().for_each(|x| *x /= frames.len() as f64);
        let norm = v.iter().map(|x| x * x).sum::<f64>().sqrt() + EPS;
        bchroma.push(v.map(|x| x / norm));
    }
    let n = bi.len();
    let change: Vec<f64> = (0..n)
        .map(|i| {
            if i == 0 {
                0.0
            } else {
                1.0 - bchroma[i]
                    .iter()
                    .zip(&bchroma[i - 1])
                    .map(|(a, b)| a * b)
                    .sum::<f64>()
            }
        })
        .collect();
    let fi: Vec<usize> = beats_t.iter().map(|&t| (t * FPS) as usize).collect();
    let kick = near_max(&r.odfs.low, &fi, NEAR);
    let snare = near_max(&r.odfs.mid, &fi, NEAR);
    let bl: Vec<f64> = bi.iter().map(|&i| r.loud01[i]).collect();
    let rise: Vec<f64> = (0..n)
        .map(|i| {
            if i == 0 {
                0.0
            } else {
                (bl[i] - bl[i - 1]).max(0.0)
            }
        })
        .collect();
    let feats = zscore_columns(&[change, kick, snare, rise]);
    let mut results: Vec<(f64, usize, usize)> = vec![];
    for (m, prior) in METERS {
        for p in 0..m {
            let mut score = 0.0;
            for (c, col) in feats.iter().enumerate() {
                let on: Vec<f64> = (0..n).filter(|i| i % m == p).map(|i| col[i]).collect();
                let off: Vec<f64> = (0..n).filter(|i| i % m != p).map(|i| col[i]).collect();
                score += (mean(&on) - mean(&off)) * DOWNBEAT_WEIGHTS[c];
            }
            results.push((score * prior, m, p));
        }
    }
    // Sorted from the best, ties by the larger meter and phase, as sorting the tuples does.
    results.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let (s, m, p) = results[0];
    let rival = results
        .iter()
        .skip(1)
        .find(|r| (r.1, r.2) != (m, p))
        .map_or(s, |r| r.0);
    let meter = Meter {
        beats_per_bar: m as u32,
        confidence: round_to(((s - rival) / METER_MARGIN).clamp(0.0, 1.0), 2),
    };
    let beats = beats_t
        .iter()
        .enumerate()
        .map(|(i, &t)| {
            let shifted = i as i64 - p as i64;
            Beat {
                t: round_to(t, 3),
                bar: floor_div(shifted, m as i64),
                pos: modulo(shifted, m as i64),
                strength: round_to((r.odf[(t * FPS) as usize] / STRENGTH_FULL).min(1.0), 2),
                confidence: round_to(interp(t, &r.ccenters, &r.conf_w), 2),
            }
        })
        .collect();
    (meter, beats)
}

/// `zscore(X)` per column: the column minus its mean over its standard deviation.
fn zscore_columns(cols: &[Vec<f64>]) -> Vec<Vec<f64>> {
    cols.iter()
        .map(|c| {
            let (m, s) = (mean(c), std(c) + EPS);
            c.iter().map(|v| (v - m) / s).collect()
        })
        .collect()
}

/// The rhythm half of `back()`: onset strengths, tempo, beats, meter and downbeats of the first
/// `nf` fine and `nc` coarse frames.
pub fn rhythm(f: &Front, nf: usize, nc: usize) -> Rhythm {
    let odfs = odf_streams(f, nf);
    let odf = onset_strength(f, nf);
    let ctimes: Vec<f64> = (0..nc).map(|i| i as f64 / CFPS).collect();
    let loud01 = loudness01(&f.loud[..nc]);
    let mut r = Rhythm {
        odfs,
        odf,
        song_feel: 0.0,
        feels: vec![],
        fcenters: vec![],
        period: vec![],
        ccenters: vec![],
        conf_w: vec![],
        song_pulse: 0.0,
        tempo: ChartTempo {
            bpm: 0.0,
            confidence: 0.0,
            feel: 0.0,
        },
        beats_t: vec![],
        loud01,
        ctimes,
        meter: Meter {
            beats_per_bar: 4,
            confidence: 0.0,
        },
        beats: vec![],
    };
    // How fast it sounds is a curve of its own, so it is there for a song too short for a tempo.
    let fl = feel(&f.flux[..nf]);
    if nf < tempo_window() {
        r.feels = fl.values;
        r.fcenters = fl.centers;
        return r;
    }
    let act_sum: f64 = fl.act.iter().sum();
    r.song_feel = fl
        .values
        .iter()
        .zip(&fl.act)
        .map(|(v, a)| v * a)
        .sum::<f64>()
        / (act_sum + EPS);
    r.feels = fl.values;
    r.fcenters = fl.centers;
    let t = tempo(&r.odf, r.song_feel);
    let periods: Vec<f64> = t.periods.iter().map(|&p| p as f64).collect();
    // The beat period at every frame, following the tempo through the song.
    r.period = (0..nf)
        .map(|i| interp(i as f64 / FPS, &t.centers, &periods))
        .collect();
    let bpm0 = 60.0 * FPS / median(&periods);
    r.conf_w = t
        .clarity
        .iter()
        .map(|c| ((c - CLARITY_FROM) / CLARITY_RANGE).clamp(0.0, 1.0))
        .collect();
    r.ccenters = t.centers;
    r.song_pulse = percentile(&r.conf_w, PULSE_PERCENTILE);
    r.tempo = ChartTempo {
        bpm: 0.0,
        confidence: round_to(r.song_pulse, 2),
        feel: round_to(r.song_feel, 2),
    };
    if r.song_pulse >= MIN_PULSE {
        let km: Vec<f64> = r
            .odfs
            .low
            .iter()
            .zip(&r.odfs.mid)
            .map(|(a, b)| a + b)
            .collect();
        let b = find_beats(&r.odf, &km, &r.period);
        r.beats_t = b.iter().map(|&i| i as f64 / FPS).collect();
        r.tempo.bpm = round_to(bpm_from_beats(&r.beats_t, bpm0), 2);
    }
    let (meter, beats) = meter_and_downbeats(f, nc, &r);
    r.meter = meter;
    r.beats = beats;
    r
}

/// The share of the sound that is percussive, smoothed (the energies are `float32`, so are the
/// smoothed ones and what is made of them), at each coarse frame.
fn percussive_curve(f: &Front, nf: usize, ctimes: &[f64]) -> Vec<f64> {
    let smooth = |energy: &[f64]| -> Vec<f32> {
        gaussian_filter1d(&energy[..nf], PERCUSSIVE_SIGMA * FPS)
            .iter()
            .map(|&v| v as f32)
            .collect()
    };
    let (pe, he) = (smooth(&f.energy_p), smooth(&f.energy_h));
    let share: Vec<f64> = pe
        .iter()
        .zip(&he)
        .map(|(&p, &h)| f64::from(p / (p + h + EPS32)))
        .collect();
    let ftimes: Vec<f64> = (0..nf).map(|i| i as f64 / FPS).collect();
    ctimes.iter().map(|&t| interp(t, &ftimes, &share)).collect()
}

/// The brightness curve: the log of the centroid, between its 5th and 95th percentile over the
/// moments with sound, 0 to 1.
fn brightness_curve(cent: &[f64], loud01: &[f64]) -> Vec<f64> {
    let lc: Vec<f64> = cent.iter().map(|c| (c + 1.0).log2()).collect();
    let sound: Vec<f64> = lc
        .iter()
        .zip(loud01)
        .filter(|&(_, &l)| l as f32 > SOUND)
        .map(|(&c, _)| c)
        .collect();
    let (q5, q95) = if sound.is_empty() {
        (0.0, 1.0)
    } else {
        (
            percentile(&sound, BRIGHT_LOW_PERCENTILE),
            percentile(&sound, BRIGHT_HIGH_PERCENTILE),
        )
    };
    lc.iter()
        .map(|c| ((c - q5) / (q95 - q5 + EPS)).clamp(0.0, 1.0))
        .collect()
}

/// `values` rounded to the 3 decimals the curves are written in.
fn r3(values: &[f64]) -> Vec<f64> {
    values.iter().map(|&v| round_to(v, 3)).collect()
}

/// The chart of the first `samples` samples of a `front` result (the whole song by default), as
/// if the song ended there.
pub fn back(f: &Front, samples: Option<usize>) -> Chart {
    let (samples, nf, nc) = cut(f, samples);
    let duration = samples as f64 / SR as f64;
    let r = rhythm(f, nf, nc);
    let mut chart = Chart::empty(round_to(duration, 3));
    chart.tempo = r.tempo.clone();
    chart.beats = r.beats.clone();
    chart.meter = r.meter.clone();

    let stack = feature_stack(&f.chroma[..nc], &f.timbre[..nc], &r.loud01);
    chart.sections = sections(&SectionInput {
        stack: &stack,
        ctimes: &r.ctimes,
        loud01: &r.loud01,
        beats_t: &r.beats_t,
        beats: &r.beats,
        meter_confidence: r.meter.confidence,
        duration,
    });

    let [low, mid, high, harmonic] =
        [&r.odfs.low, &r.odfs.mid, &r.odfs.high, &r.odfs.harmonic].map(|odf| odf.as_slice());
    chart.streams.low.events = pick(low, GAPS[0]);
    chart.streams.mid.events = pick(mid, GAPS[1]);
    chart.streams.high.events = pick(high, GAPS[2]);
    chart.streams.harmonic.events = pick(harmonic, GAPS[3]);
    chart.streams.phrase.events = phrase_starts(&stack, &r.ctimes, &r.loud01);
    chart.lead = lead_stream(
        f,
        nf,
        [
            &chart.streams.low.events,
            &chart.streams.mid.events,
            &chart.streams.high.events,
            &chart.streams.harmonic.events,
        ],
        &r.loud01,
        &r.ctimes,
        duration,
    );

    let bconf: Vec<f64> = r
        .ctimes
        .iter()
        .map(|&t| {
            if r.conf_w.is_empty() {
                0.0
            } else {
                interp(t, &r.ccenters, &r.conf_w)
            }
        })
        .collect();
    let feel_curve: Vec<f64> = r
        .ctimes
        .iter()
        .map(|&t| interp(t, &r.fcenters, &r.feels))
        .collect();
    chart.curves = Curves {
        hop: round_to(CHOP as f64 / SR as f64, 6),
        loudness: r3(&r.loud01),
        brightness: r3(&brightness_curve(&f.cent[..nc], &r.loud01)),
        percussive: r3(&percussive_curve(f, nf, &r.ctimes)),
        beat_conf: r3(&bconf),
        feel: r3(&feel_curve),
    };
    chart
}
