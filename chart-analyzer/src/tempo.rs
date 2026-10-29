//! Tempo and beats, ported from the "tempo", "how fast it sounds" and "beats" sections of
//! the prototype: the autocorrelation of the onset strength over 8 s windows, scored with the
//! lag's multiples and a prior; how fast the music sounds; the tempo path through the song
//! (Viterbi); and the beats by dynamic programming (Ellis 2007).

use realfft::RealFftPlanner;

use crate::front::{EPS, FPS};
use crate::np::{argmax, argmin, hanning, mean, round_half_even, std};

/// The multiples of a lag its autocorrelation is scored together with, and their weights: the
/// half counts for less.
const MULTIPLES: [(f64, f64); 5] = [(0.5, 0.5), (1.0, 1.0), (2.0, 1.0), (3.0, 1.0), (4.0, 1.0)];
pub const MIN_BPM: f64 = 50.0;
pub const MAX_BPM: f64 = 220.0;
/// The tempo prior's centre. When a tempo and its double fit about as well, the one nearer this
/// wins; it sits above 120 so that the faster reading wins (better too fast than too slow).
const PRIOR_BPM: f64 = 140.0;
/// The tempo windows: 8 s long, a second apart.
const TEMPO_WIN: f64 = 8.0;
const TEMPO_STEP: f64 = 1.0;

/// The perceived speed is measured over `FEEL_WIN` seconds every `FEEL_STEP` seconds.
const FEEL_WIN: f64 = 8.0;
const FEEL_STEP: f64 = 2.0;
/// The modulation frequencies (Hz) `feel` looks at.
const FEEL_BAND: (f64, f64) = (0.5, 12.0);
/// Music that sounds at least this fast (Hz, see `feel`) is read at the faster of a tempo below
/// `FAST_BELOW` bpm and its double: 105 bpm hardcore sounds like 210.
pub const FEEL_FAST: f64 = 5.8;
const FAST_BELOW: f64 = 108.0;
/// The tempo of the double has to be this close (as a ratio, from 2) to count as the double.
const DOUBLE_TOLERANCE: f64 = 0.1;

/// What changing the tempo costs from one window to the next (a second apart), per octave
/// squared. A real change of tempo wins after a few seconds of evidence; jitter costs next to
/// nothing and a jump to the double or the half needs a long stretch of strong evidence.
const TEMPO_CHANGE: f64 = 30.0;
/// The width (octaves) of the window around the song's tempo that the path may wander in.
const ANCHOR_WIDTH: f64 = 0.75;
/// The least evidence per lag: the log of anything weaker is the same.
const EVIDENCE_FLOOR: f64 = 1e-3;

/// How strictly the beat DP holds the beats to the tempo: the cost per squared log of the ratio
/// of an interval to the period.
pub const BEAT_TIGHTNESS: f64 = 100.0;

/// The windows of `window_acfs`: the normalised autocorrelation of each, its centre time and its
/// activity (the standard deviation of the onset strength in it).
pub struct Windows {
    pub acfs: Vec<Vec<f64>>,
    pub centers: Vec<f64>,
    pub act: Vec<f64>,
}

/// The number of samples in the tempo window.
pub fn tempo_window() -> usize {
    (TEMPO_WIN * FPS) as usize
}

/// `window_acfs(odf)`: the autocorrelation of the onset strength over 8 s windows a second
/// apart, up to four beats of the slowest tempo.
pub fn window_acfs(odf: &[f64]) -> Windows {
    let (w, step) = (tempo_window(), (TEMPO_STEP * FPS) as usize);
    let max_lag = (4.0 * 60.0 / MIN_BPM * FPS) as usize + 4;
    let mut planner = RealFftPlanner::<f64>::new();
    let mut out = Windows {
        acfs: vec![],
        centers: vec![],
        act: vec![],
    };
    let last_start = odf.len().saturating_sub(w) + 1;
    for s in (0..last_start.max(1)).step_by(step) {
        let seg = &odf[s..(s + w).min(odf.len())];
        let m = mean(seg);
        let n = 2 * seg.len();
        let (forward, inverse) = (planner.plan_fft_forward(n), planner.plan_fft_inverse(n));
        let mut input: Vec<f64> = seg.iter().map(|v| v - m).collect();
        input.resize(n, 0.0);
        let act = std(&input[..seg.len()]);
        let mut spectrum = forward.make_output_vec();
        forward
            .process(&mut input, &mut spectrum)
            .unwrap_or_default();
        // The power spectrum, whose inverse is the autocorrelation.
        let mut power: Vec<_> = spectrum
            .iter()
            .map(|c| realfft::num_complex::Complex::new(c.norm_sqr(), 0.0))
            .collect();
        let mut ac = inverse.make_output_vec();
        inverse.process(&mut power, &mut ac).unwrap_or_default();
        ac.iter_mut().for_each(|v| *v /= n as f64);
        ac.resize(max_lag + 1, 0.0);
        let zero = ac[0];
        out.acfs.push(if zero > EPS {
            ac.iter().map(|v| v / (zero + EPS)).collect()
        } else {
            vec![0.0; max_lag + 1]
        });
        out.centers.push((s as f64 + seg.len() as f64 / 2.0) / FPS);
        out.act.push(act);
    }
    out
}

/// The lags (in frames) the tempo is searched over.
fn tempo_lags() -> Vec<usize> {
    ((60.0 / MAX_BPM * FPS) as usize..=(60.0 / MIN_BPM * FPS) as usize).collect()
}

/// `tempo_scores(acfs)`: for every window and every lag, the autocorrelation at the lag and its
/// multiples, each the strongest within a few frames (the longer the multiple the wider). The
/// lags are returned with the scores.
pub fn tempo_scores(acfs: &[Vec<f64>]) -> (Vec<usize>, Vec<Vec<f64>>) {
    let lags = tempo_lags();
    let mut out = vec![vec![0.0; lags.len()]; acfs.len()];
    for (m, wgt) in MULTIPLES {
        let s = if m > 1.0 {
            (0.5 * (m - 1.0)) as usize
        } else {
            0
        };
        for (j, &lag) in lags.iter().enumerate() {
            let c = round_half_even(m * lag as f64) as usize;
            for (row, acf) in out.iter_mut().zip(acfs) {
                let window = &acf[c.saturating_sub(s)..(c + s + 1).min(acf.len())];
                row[j] += wgt * window.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            }
        }
    }
    (lags, out)
}

/// How fast the music sounds per window, the windows' centres and their activity.
pub struct Feel {
    pub values: Vec<f64>,
    pub centers: Vec<f64>,
    pub act: Vec<f64>,
}

/// `feel(flux)`. How fast the music sounds, every `FEEL_STEP` seconds: the log-frequency
/// centroid (Hz) of the modulation spectrum (0.5 to 12 Hz) of the spectrum's raw change over
/// `FEEL_WIN` seconds. Sparse, slow music changes once or twice a second, dense fast music five
/// to seven times; unlike a tempo it has no octave to get wrong. Measured on the songs of the
/// visualizer's playlist: ballads and classical 3 to 4, pop about 5.3, drum and bass, dubstep,
/// hardcore and metal 6 to 7.4.
pub fn feel(flux: &[f64]) -> Feel {
    let w = (FEEL_WIN * FPS) as usize;
    let step = (FEEL_STEP * FPS) as usize;
    // np.fft.rfftfreq(w, 1 / FPS)
    let val = 1.0 / (w as f64 * (1.0 / FPS));
    let freqs: Vec<(usize, f64)> = (0..=w / 2)
        .map(|k| (k, k as f64 * val))
        .filter(|&(_, f)| f >= FEEL_BAND.0 && f <= FEEL_BAND.1)
        .collect();
    let win = hanning(w);
    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(w);
    let mut spectrum = fft.make_output_vec();
    let mut out = Feel {
        values: vec![],
        centers: vec![],
        act: vec![],
    };
    for s0 in (0..(flux.len().saturating_sub(w) + 1).max(1)).step_by(step) {
        let mut seg = flux[s0..(s0 + w).min(flux.len())].to_vec();
        seg.resize(w, 0.0);
        let m = mean(&seg);
        seg.iter_mut()
            .zip(&win)
            .for_each(|(v, h)| *v = (*v - m) * h);
        let mut input = seg.clone();
        fft.process(&mut input, &mut spectrum).unwrap_or_default();
        let (mut weighted, mut total) = (0.0, 0.0);
        for &(k, f) in &freqs {
            let p = spectrum[k].norm_sqr();
            weighted += p * f.ln();
            total += p;
        }
        out.values.push((weighted / (total + EPS)).exp());
        out.centers.push((s0 as f64 + w as f64 / 2.0) / FPS);
        out.act.push(std(&seg));
    }
    out
}

/// The beat period of every window, the windows' centres and their pulse clarity.
pub struct Tempo {
    pub periods: Vec<usize>,
    pub clarity: Vec<f64>,
    pub centers: Vec<f64>,
}

/// `tempo(odf, song_feel)`. The beat period of every 8 s window (a second apart), the windows'
/// centres and their pulse clarity. The periods are the best path through the windows' tempo
/// scores (Viterbi), so the tempo may change through the song. `song_feel` is how fast the song
/// sounds.
pub fn tempo(odf: &[f64], song_feel: f64) -> Tempo {
    let Windows { acfs, centers, act } = window_acfs(odf);
    let (lags, scores) = tempo_scores(&acfs);
    let bpm: Vec<f64> = lags.iter().map(|&l| 60.0 * FPS / l as f64).collect();
    let prior: Vec<f64> = bpm
        .iter()
        .map(|b| (-0.5 * (b / PRIOR_BPM).log2().powi(2)).exp())
        .collect();
    // The whole song decides which octave the tempo lives in (summing every window's scores, so
    // the faster reading wins when it fits about as well, see PRIOR_BPM); the path may wander
    // from it, a real change of tempo (134 to 100 bpm is 0.42 octave) easily, a whole octave
    // hardly.
    let act_sum: f64 = act.iter().sum();
    let total: Vec<f64> = (0..lags.len())
        .map(|j| {
            scores
                .iter()
                .zip(&act)
                .map(|(row, a)| row[j] * a)
                .sum::<f64>()
                / (act_sum + EPS)
        })
        .collect();
    let weighted: Vec<f64> = total.iter().zip(&prior).map(|(t, p)| t * p).collect();
    let mut j = argmax(&weighted);
    if song_feel >= FEEL_FAST && bpm[j] < FAST_BELOW {
        let half = lags[j] as f64 / 2.0;
        let double = argmin(
            &lags
                .iter()
                .map(|&l| (l as f64 - half).abs())
                .collect::<Vec<_>>(),
        );
        if (bpm[double] / bpm[j] - 2.0).abs() < DOUBLE_TOLERANCE {
            j = double;
        }
    }
    let home = bpm[j].log2();
    let anchor: Vec<f64> = bpm
        .iter()
        .map(|b| (-0.5 * ((b.log2() - home) / ANCHOR_WIDTH).powi(2)).exp())
        .collect();
    // Each window's evidence as log-probabilities over the lags; quiet windows say less.
    let act_max = act.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let emit: Vec<Vec<f64>> = scores
        .iter()
        .zip(&act)
        .map(|(row, a)| {
            let ev: Vec<f64> = (0..lags.len())
                .map(|j| row[j] * prior[j] * anchor[j])
                .collect();
            let top = ev.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let weight = a / (act_max + EPS);
            ev.iter()
                .map(|e| (e / (top + EPS)).max(EVIDENCE_FLOOR).ln() * weight)
                .collect()
        })
        .collect();
    let path = viterbi(&emit, &bpm.iter().map(|b| b.log2()).collect::<Vec<_>>());
    let periods: Vec<usize> = path.iter().map(|&p| lags[p]).collect();
    // Pulse clarity per window: how strongly the onsets repeat at its period, its half or its
    // double (swung or sparse beats repeat best at one of them).
    let at = |a: &[f64], l: usize| {
        a[l.saturating_sub(2).min(a.len())..(l + 3).min(a.len())]
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max)
    };
    let clarity = acfs
        .iter()
        .zip(&periods)
        .map(|(a, &l)| at(a, l / 2).max(at(a, l)).max(at(a, 2 * l)))
        .collect();
    Tempo {
        periods,
        clarity,
        centers,
    }
}

/// The best path through the windows' `emit` log-probabilities (windows by lags), paying
/// `TEMPO_CHANGE` per octave squared of change between consecutive windows; `log_bpm` is the
/// octave of every lag.
fn viterbi(emit: &[Vec<f64>], log_bpm: &[f64]) -> Vec<usize> {
    let n = log_bpm.len();
    let mut best = emit[0].clone();
    let mut back = vec![vec![0usize; n]; emit.len()];
    let mut next = vec![0.0; n];
    for w in 1..emit.len() {
        for to in 0..n {
            // The first of the equal best predecessors, as np.argmax.
            let (mut arg, mut top) = (0, f64::NEG_INFINITY);
            for from in 0..n {
                let cand = best[from] - TEMPO_CHANGE * (log_bpm[from] - log_bpm[to]).powi(2);
                if cand > top {
                    (arg, top) = (from, cand);
                }
            }
            back[w][to] = arg;
            next[to] = top + emit[w][to];
        }
        std::mem::swap(&mut best, &mut next);
    }
    let mut path = vec![0; emit.len()];
    path[emit.len() - 1] = argmax(&best);
    for w in (1..emit.len()).rev() {
        path[w - 1] = back[w][path[w]];
    }
    path
}

/// `dp_beats(odf, period)`: beats (frame numbers) by Ellis's dynamic programming; `period` is
/// the beat period at every frame.
pub fn dp_beats(odf: &[f64], period: &[f64]) -> Vec<usize> {
    let n = odf.len();
    let mut score = odf.to_vec();
    let mut back = vec![usize::MAX; n];
    for t in 0..n {
        let p = period[t];
        let lo = round_half_even(p / 2.0) as usize;
        if t < lo {
            continue;
        }
        let hi = (round_half_even(2.0 * p) as usize).min(t);
        let (mut best, mut best_cand) = (f64::NEG_INFINITY, 0);
        for off in lo..=hi {
            let cand = t - off;
            let v = score[cand] - BEAT_TIGHTNESS * (off as f64 / p).ln().powi(2);
            if v > best {
                (best, best_cand) = (v, cand);
            }
        }
        if best > 0.0 {
            score[t] += best;
            back[t] = best_cand;
        }
    }
    let tail = n.saturating_sub((2.0 * period[n - 1]) as usize);
    let mut t = tail + argmax(&score[tail..]);
    let mut beats = vec![t];
    while back[t] != usize::MAX {
        t = back[t];
        beats.push(t);
    }
    beats.reverse();
    beats
}

/// `near_max(x, idx, r)`: the largest value of `x` within `r` frames of each index.
pub fn near_max(x: &[f64], idx: &[usize], r: usize) -> Vec<f64> {
    idx.iter()
        .map(|&i| {
            let i = i.min(x.len() - 1);
            x[i.saturating_sub(r)..(i + r + 1).min(x.len())]
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max)
        })
        .collect()
}
