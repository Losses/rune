//! Sections and phrase starts, ported from the "features" and "sections" parts of the prototype
//! (`zscore`, `novelty`, `peaks` and what `back()` does with them): the coarse frames' chroma,
//! timbre and loudness are stacked into one feature per frame, averaged over beats (half
//! seconds when there is no beat), and a Foote novelty curve over their cosine self-similarity
//! says where the music changes. The sections are cut at its peaks, labelled by their
//! similarity to the sections before, and the phrase starts are its peaks at a shorter scale.
//!
//! The Python's `chroma` and `loud01` are `float32` and its `timbre` `float64`: `zscore` of the
//! two `float32` ones is done in `f32` the way numpy sums them (in order down a column of
//! the chroma, pairwise down the loudness's single column), so that the features are numpy's
//! bit for bit; the similarities that follow differ from numpy's matrix product in the last
//! bits only (no BLAS here, plain dot products).

use std::collections::BTreeSet;

use crate::chart::{Beat, Section};
use crate::front::{CFPS, EPS, EPS32};
use crate::ndimage::{maximum_filter1d, uniform_filter1d};
use crate::np::{median, pairwise_sum, round_half_even, round_to, searchsorted};

/// The features of a coarse frame: 12 chroma, 12 timbre coefficients, the loudness.
pub const FEATURES: usize = 25;
pub type Feature = [f64; FEATURES];

/// The loudness's weight against the chroma's and timbre's, whose columns are 1 each.
const LOUD_WEIGHT: f64 = 2.0;

/// The self-similarity is compared this many seconds either side of a point: the kernel's
/// half-width, in units, is this over the unit's length, at least `MIN_KERNEL`.
const COMPARE_SECONDS: f64 = 10.0;
const MIN_KERNEL: usize = 4;
/// Sections are at least this long (s), at least `MIN_SECTION_UNITS` units.
const SECTION_SECONDS: f64 = 8.0;
const MIN_SECTION_UNITS: usize = 2;
/// A novelty peak counts from this height (the highest peak is 1).
const SECTION_THRESHOLD: f64 = 0.2;
/// There are at most this many seconds of song per section.
const SECONDS_PER_SECTION: f64 = 12.0;
/// The downbeat snapping needs the meter at least this sure.
const SNAP_CONFIDENCE: f64 = 0.3;
/// Without a beat the units are this long (s).
const HALF_SECOND: f64 = 0.5;
/// A section is the same as an earlier one from this cosine similarity (the later one is
/// added to the earlier one's prototype).
const SAME_SECTION: f64 = 0.75;
/// How many letters the labels cycle through.
const LABELS: usize = 26;

/// The phrase starts' units (s), kernel half-width (units), the radius (units) and height of
/// the peaks, and the loudness (0 to 1) below which a peak is silence.
const PHRASE_UNIT: f64 = 0.25;
const PHRASE_KERNEL: usize = 8;
const PHRASE_RADIUS: usize = 6;
const PHRASE_THRESHOLD: f64 = 0.3;
const PHRASE_MIN_LOUDNESS: f64 = 0.05;

/// `zscore(X)` of a `float32` column summed `pairwise` (or in order, as numpy sums down the
/// columns of a 2-D array): the values minus their mean over their standard deviation plus
/// `EPS`, in `f32`.
fn zscore_f32(column: &[f32], pairwise: bool) -> Vec<f32> {
    let n = column.len() as f32;
    let sum = |v: &[f32]| {
        if pairwise {
            pairwise_sum(v)
        } else {
            v.iter().fold(0.0, |s, &x| s + x)
        }
    };
    let mean = sum(column) / n;
    let centred: Vec<f32> = column.iter().map(|&v| v - mean).collect();
    let squares: Vec<f32> = centred.iter().map(|&d| d * d).collect();
    let std = (sum(&squares) / n).sqrt();
    centred.iter().map(|&d| d / (std + EPS32)).collect()
}

/// `zscore(X)` of a `float64` column, summed in order.
fn zscore_f64(column: &[f64]) -> Vec<f64> {
    let n = column.len() as f64;
    let mean = column.iter().fold(0.0, |s, &x| s + x) / n;
    let std = (column.iter().fold(0.0, |s, &x| s + (x - mean) * (x - mean)) / n).sqrt();
    column.iter().map(|&v| (v - mean) / (std + EPS)).collect()
}

/// The feature of every coarse frame: `zscore` of the chroma and of the timbre and twice that of
/// the loudness (`loud01`, 0 to 1), side by side.
pub fn feature_stack(chroma: &[[f64; 12]], timbre: &[[f64; 12]], loud01: &[f64]) -> Vec<Feature> {
    let mut stack = vec![[0.0; FEATURES]; chroma.len()];
    for c in 0..12 {
        let column: Vec<f32> = chroma.iter().map(|row| row[c] as f32).collect();
        for (row, z) in stack.iter_mut().zip(zscore_f32(&column, false)) {
            row[c] = f64::from(z);
        }
        let column: Vec<f64> = timbre.iter().map(|row| row[c]).collect();
        for (row, z) in stack.iter_mut().zip(zscore_f64(&column)) {
            row[12 + c] = z;
        }
    }
    let column: Vec<f32> = loud01.iter().map(|&v| v as f32).collect();
    for (row, z) in stack.iter_mut().zip(zscore_f32(&column, true)) {
        row[24] = LOUD_WEIGHT * f64::from(z);
    }
    stack
}

/// The mean of the features `from..to`, summed in order.
fn mean_feature(stack: &[Feature], from: usize, to: usize) -> Feature {
    let mut sum = [0.0; FEATURES];
    for row in &stack[from..to] {
        sum.iter_mut().zip(row).for_each(|(s, v)| *s += v);
    }
    sum.map(|s| s / (to - from) as f64)
}

/// `np.linalg.norm` of a feature.
fn norm(v: &Feature) -> f64 {
    pairwise_sum(&v.map(|x| x * x)).sqrt()
}

fn dot(a: &Feature, b: &Feature) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// `np.linspace(-1, 1, num)`.
fn linspace_unit(num: usize) -> Vec<f64> {
    let step = 2.0 / (num - 1) as f64;
    let mut out: Vec<f64> = (0..num).map(|i| i as f64 * step + -1.0).collect();
    out[num - 1] = 1.0;
    out
}

/// `novelty(X, K)`: the Foote novelty of the cosine self-similarity of the rows of `x`, kernel
/// half-width `k`, as the correlation of a checkerboard kernel (Gaussian-tapered) along the
/// diagonal of the similarity (edge-padded), clipped at 0 and scaled so that its highest value
/// is 1. The similarity is never held whole, only the window under the kernel is made at each
/// point.
pub fn novelty(x: &[Feature], k: usize) -> Vec<f64> {
    let n = x.len();
    if n == 0 {
        return vec![];
    }
    let unit: Vec<Feature> = x
        .iter()
        .map(|row| {
            let norm = norm(row) + EPS;
            row.map(|v| v / norm)
        })
        .collect();
    let m = 2 * k;
    let g: Vec<f64> = linspace_unit(m)
        .iter()
        .map(|v| (-0.5 * (v * 2.0) * (v * 2.0)).exp())
        .collect();
    // The kernel: its taper, positive where both indices are on the same side of the middle.
    let kernel: Vec<f64> = (0..m * m)
        .map(|j| {
            let (a, b) = (j / m, j % m);
            let sign = if (a < k) == (b < k) { 1.0 } else { -1.0 };
            sign * (g[a] * g[b])
        })
        .collect();
    let mut at = vec![0usize; m];
    let mut window = vec![0.0; m * m];
    let mut nov: Vec<f64> = (0..n)
        .map(|i| {
            for (a, slot) in at.iter_mut().enumerate() {
                *slot = (i as isize + a as isize - k as isize).clamp(0, n as isize - 1) as usize;
            }
            for a in 0..m {
                for b in a..m {
                    let s = dot(&unit[at[a]], &unit[at[b]]);
                    window[a * m + b] = s * kernel[a * m + b];
                    window[b * m + a] = s * kernel[b * m + a];
                }
            }
            pairwise_sum(&window).max(0.0)
        })
        .collect();
    let top = nov.iter().copied().fold(f64::NEG_INFINITY, f64::max) + EPS;
    nov.iter_mut().for_each(|v| *v /= top);
    nov
}

/// `peaks(x, radius, thresh)`: the indices of the values that are the highest within `radius`
/// either side (every one of equal values) and at least `thresh`.
pub fn peaks(x: &[f64], radius: usize, thresh: f64) -> Vec<usize> {
    let (mut buf, mut mx) = (Vec::new(), vec![0.0; x.len()]);
    maximum_filter1d(x, 2 * radius + 1, &mut buf, &mut mx);
    (0..x.len())
        .filter(|&i| x[i] == mx[i] && x[i] >= thresh)
        .collect()
}

/// What `sections` needs of the rest of the analysis.
pub struct SectionInput<'a> {
    /// The feature of each coarse frame, and the times and loudness (0 to 1) of the frames.
    pub stack: &'a [Feature],
    pub ctimes: &'a [f64],
    pub loud01: &'a [f64],
    /// The beats (s), as found, and as the chart has them with their bar positions.
    pub beats_t: &'a [f64],
    pub beats: &'a [Beat],
    pub meter_confidence: f64,
    pub duration: f64,
}

/// The sections of the song. The units are the beats (so a section starts on one), or half
/// seconds when there is no beat; each unit's feature is the mean of its frames', lightly
/// smoothed. The novelty is compared 10 s either side; the sections are at least 8 s long and
/// there is at most one per 12 s of song; with a meter that is sure their starts move to the
/// nearest downbeat.
pub fn sections(i: &SectionInput) -> Vec<Section> {
    let nc = i.stack.len();
    if nc == 0 {
        return vec![];
    }
    let (mut edges, unit_t, unit): (Vec<usize>, Vec<f64>, f64) = if i.beats.is_empty() {
        let step = (HALF_SECOND * CFPS) as usize;
        let edges: Vec<usize> = (0..nc).step_by(step).collect();
        let unit_t = edges.iter().map(|&e| i.ctimes[e]).collect();
        (edges, unit_t, step as f64 / CFPS)
    } else {
        let edges = i
            .beats_t
            .iter()
            .map(|&t| searchsorted(i.ctimes, t).min(nc - 1))
            .collect();
        let gaps: Vec<f64> = i.beats_t.windows(2).map(|w| w[1] - w[0]).collect();
        (edges, i.beats_t.to_vec(), median(&gaps))
    };
    let k = MIN_KERNEL.max(round_half_even(COMPARE_SECONDS / unit) as usize);
    let minlen = MIN_SECTION_UNITS.max(round_half_even(SECTION_SECONDS / unit) as usize);
    edges.push(nc);
    let mut u: Vec<Feature> = edges
        .windows(2)
        .map(|w| mean_feature(i.stack, w[0], (w[0] + 1).max(w[1])))
        .collect();
    smooth(&mut u);
    let nov = novelty(&u, k);
    let n = u.len();
    let mut cuts: Vec<usize> = peaks(&nov, minlen, SECTION_THRESHOLD)
        .into_iter()
        .filter(|&c| minlen <= c && c + minlen / 2 <= n)
        .collect();
    // The highest first (equal ones in time order), as many as the song's length allows.
    cuts.sort_by(|&a, &b| nov[b].total_cmp(&nov[a]));
    cuts.truncate(1.max((i.duration / SECONDS_PER_SECTION) as usize));
    cuts.sort_unstable();
    let down: Vec<usize> = (0..i.beats.len())
        .filter(|&b| i.beats[b].pos == 0)
        .collect();
    if !i.beats.is_empty() && i.meter_confidence >= SNAP_CONFIDENCE && !down.is_empty() {
        let snapped: BTreeSet<usize> = cuts.iter().map(|&c| nearest(&down, c)).collect();
        cuts = snapped.into_iter().collect();
    }
    let mut bounds = vec![0];
    bounds.extend(&cuts);
    bounds.push(n);
    let mut prototypes: Vec<Feature> = vec![];
    let mut labels: Vec<usize> = vec![];
    let mut out = vec![];
    for w in bounds.windows(2).filter(|w| w[1] > w[0]) {
        let (a, b) = (w[0], w[1]);
        let mean = mean_feature(&u, a, b);
        let length = norm(&mean) + EPS;
        let v = mean.map(|x| x / length);
        let sims: Vec<f64> = prototypes
            .iter()
            .map(|p| {
                let length = norm(p) + EPS;
                dot(&v, &p.map(|x| x / length))
            })
            .collect();
        let j = sims
            .iter()
            .enumerate()
            .fold(None, |best: Option<usize>, (j, &s)| match best {
                Some(b) if sims[b] >= s => Some(b),
                _ => Some(j),
            });
        match j {
            Some(j) if sims[j] > SAME_SECTION => {
                labels.push(j);
                prototypes[j].iter_mut().zip(&v).for_each(|(p, x)| *p += x);
            }
            _ => {
                labels.push(prototypes.len());
                prototypes.push(v);
            }
        }
        let label = labels[labels.len() - 1];
        let start = if a == 0 { 0.0 } else { unit_t[a] };
        let end = if b >= n { i.duration } else { unit_t[b] };
        let inside: Vec<f32> = (0..nc)
            .filter(|&c| i.ctimes[c] >= start && i.ctimes[c] < end)
            .map(|c| i.loud01[c] as f32)
            .collect();
        out.push(Section {
            start: round_to(start, 3),
            end: round_to(end, 3),
            label: char::from(b'A' + (label % LABELS) as u8).to_string(),
            energy: round_to(mean_loudness(&inside), 2),
            repeat: labels[..labels.len() - 1]
                .iter()
                .filter(|&&l| l == label)
                .count() as u32,
            confidence: if a == 0 { 1.0 } else { round_to(nov[a], 2) },
        });
    }
    out
}

/// The mean of `float32` values as `np.mean` makes it (summed pairwise in `f32`), 0 for none.
fn mean_loudness(values: &[f32]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    f64::from((f64::from(pairwise_sum(values)) / values.len() as f64) as f32)
}

/// `uniform_filter1d(U, 2, axis=0)`: each feature averaged with the unit before it.
fn smooth(u: &mut [Feature]) {
    for c in 0..FEATURES {
        let column: Vec<f64> = u.iter().map(|row| row[c]).collect();
        for (row, v) in u.iter_mut().zip(uniform_filter1d(&column, 2)) {
            row[c] = v;
        }
    }
}

/// The value of `down` nearest `to` (the first of two equally near).
fn nearest(down: &[usize], to: usize) -> usize {
    let distance = |d: usize| d.abs_diff(to);
    down.iter().copied().fold(down[0], |best, d| {
        if distance(d) < distance(best) {
            d
        } else {
            best
        }
    })
}

/// The phrase starts: the novelty (kernel half-width 8) of the features averaged over quarter
/// seconds, its peaks (6 units either side, from 0.3) where the sound is not silent, as
/// `[t, strength]`.
pub fn phrase_starts(stack: &[Feature], ctimes: &[f64], loud01: &[f64]) -> Vec<[f64; 2]> {
    let step = 1.max((PHRASE_UNIT * CFPS) as usize);
    let starts: Vec<usize> = (0..stack.len()).step_by(step).collect();
    let units: Vec<Feature> = starts
        .iter()
        .map(|&a| mean_feature(stack, a, (a + step).min(stack.len())))
        .collect();
    let nov = novelty(&units, PHRASE_KERNEL);
    peaks(&nov, PHRASE_RADIUS, PHRASE_THRESHOLD)
        .into_iter()
        .filter(|&p| loud01[starts[p]] > PHRASE_MIN_LOUDNESS)
        .map(|p| [round_to(ctimes[starts[p]], 3), round_to(nov[p], 2)])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linspace_has_exact_ends() {
        let v = linspace_unit(8);
        assert_eq!((v[0], v[7]), (-1.0, 1.0));
        // np.linspace(-1, 1, 8)[3]
        assert_eq!(v[3], -0.1428571428571429);
    }

    #[test]
    fn zscore_sums_as_numpy_does() {
        let x = [0.1f32, 0.5, 0.2, 0.9, 0.4, 0.3, 0.8, 0.7, 0.05, 0.6];
        // zscore of a float32 array's first column (summed in order), numpy's values
        let want = [
            -1.2692745923995972,
            0.16089391708374023,
            -0.9117324352264404,
            1.5910624265670776,
            -0.19664819538593292,
            -0.5541903376579285,
            1.2335203886032104,
            0.8759781718254089,
            -1.4480456113815308,
            0.5184361338615417,
        ];
        let got: Vec<f64> = zscore_f32(&x, false).into_iter().map(f64::from).collect();
        assert_eq!(got, want);
        // and of the single column (n, 1), summed pairwise
        let x = [0.9f32, 0.2, 0.4, 0.3, 0.8, 0.1, 0.7, 0.6, 0.5, 0.35];
        let want = [
            1.666344165802002,
            -1.1443567276000977,
            -0.3412993252277374,
            -0.7428280115127563,
            1.2648155689239502,
            -1.5458855628967285,
            0.8632867336273193,
            0.46175819635391235,
            0.06022936850786209,
            -0.5420637130737305,
        ];
        let got: Vec<f64> = zscore_f32(&x, true).into_iter().map(f64::from).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn novelty_matches_the_python() {
        // novelty(X, 4) of 14 rows that change at the 8th, numpy's values
        let x: Vec<Feature> = (0..14)
            .map(|i| {
                std::array::from_fn(|j| {
                    (0.5 * i as f64 * (j + 1) as f64 + j as f64).sin()
                        + if i >= 7 && j % 3 == 0 { 3.0 } else { 0.0 }
                })
            })
            .collect();
        let want = [
            0.33074373482428826,
            0.9999999998381282,
            0.6295374478033786,
            0.4983779776274738,
            0.4490019127670815,
            0.571689855086509,
            0.6758172840089889,
            0.8278053795901337,
            0.32099378568245834,
            0.1323980757029829,
            0.07442506240635564,
            0.01133213514236954,
            0.10342195271901189,
            0.15430574217803125,
        ];
        let got = novelty(&x, 4);
        for (a, b) in got.iter().zip(want) {
            assert!((a - b).abs() < 1e-12, "{got:?}");
        }
    }

    #[test]
    fn peaks_keep_every_equal_maximum() {
        let x = [0.0, 0.5, 0.5, 0.1, 0.9, 0.0];
        assert_eq!(peaks(&x, 1, 0.2), vec![1, 2, 4]);
        assert_eq!(peaks(&x, 3, 0.2), vec![4]);
        assert_eq!(peaks(&x, 1, 0.6), vec![4]);
    }

    #[test]
    fn nearest_takes_the_first_of_two() {
        assert_eq!(nearest(&[0, 4, 8], 6), 4);
        assert_eq!(nearest(&[0, 4, 8], 7), 8);
        assert_eq!(nearest(&[3], 100), 3);
    }
}
