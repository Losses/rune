//! The event streams and the lead, ported from `pick` and `lead_stream` of the prototype: the
//! peaks of each onset curve are the stream's events, and the lead says which stream stands
//! out at each moment, by how busy it is against its own busiest moments and how much of the
//! sound it is.
//!
//! The onset curves are `float32` in the Python, so `pick` compares in `f32`, as numpy does
//! with a `float32` array and a Python number; the shares of the sound in `lead_stream` are
//! `float32` up to the point where they meet a `float64`.

use crate::chart::LeadSpan;
use crate::front::{EPS, EPS32, FPS, Front};
use crate::ndimage::{gaussian_filter1d, maximum_filter1d, uniform_filter1d};
use crate::np::{interp, percentile, round_to};

/// The events of an onset curve are its peaks: highest within `gap` seconds either side
/// (the gaps of the low, mid, high and harmonic streams).
pub const GAPS: [f64; 4] = [0.06, 0.06, 0.04, 0.1];
/// A peak is at least this far above the mean of the curve around it (`DELTA`, over `AVERAGE`
/// seconds either side) and at least `FLOOR` high.
const DELTA: f32 = 0.12;
const AVERAGE: f64 = 0.25;
const FLOOR: f32 = 0.2;

/// `pick(odf, gap)`: the onset peaks of a 0 to about 1 strength curve (its values `f32`s held in
/// `f64`s), as `[t, strength]`: the local maximum within `gap` seconds, `DELTA` above the mean of
/// the surrounding `AVERAGE` seconds and at least `FLOOR`.
pub fn pick(odf: &[f64], gap: f64) -> Vec<[f64; 2]> {
    let w = 1.max((gap * FPS) as usize);
    let (mut buf, mut mx) = (Vec::new(), vec![0.0; odf.len()]);
    maximum_filter1d(odf, 2 * w + 1, &mut buf, &mut mx);
    let mean = uniform_filter1d(odf, 3.max((AVERAGE * 2.0 * FPS) as usize));
    (0..odf.len())
        .filter(|&i| {
            let v = odf[i] as f32;
            odf[i] == mx[i] && v >= mean[i] as f32 + DELTA && v >= FLOOR
        })
        .map(|i| [round_to(i as f64 / FPS, 3), round_to(odf[i].min(1.0), 2)])
        .collect()
}

/// How long (s) a lead has to last to stand on its own.
const MIN_LEAD: f64 = 1.5;
/// The lead is looked at every this many seconds.
const LEAD_STEP: f64 = 0.25;
/// Width (s) of the Gaussian that smooths the events into how busy a stream is.
const BUSY_SIGMA: f64 = 0.7;
/// The percentile of a stream's busyness (over the moments with sound) that reads 1, and the
/// most it reads.
const BUSY_PERCENTILE: f64 = 95.0;
const BUSY_CEILING: f64 = 1.5;
/// Frames of a stream's share of the sound are smoothed over this many seconds, and made
/// `SHARE_POWER` of the way to flat so that a loud bass does not swallow the rest.
const SHARE_SIGMA: f64 = 1.0;
const SHARE_POWER: f32 = 0.3;
/// A stream's salience is its busyness times `SHARE_BASE` plus its share of the sound.
const SHARE_BASE: f32 = 0.3;
/// Below this loudness (0 to 1) nothing leads, and below this salience nor does a stream.
const SILENCE: f64 = 0.05;
const MIN_SALIENCE: f64 = 0.15;
/// A stream takes the lead from the current one when it is this much more salient (so many
/// times in a row, `TAKEOVER_STEPS`).
const TAKEOVER_RATIO: f64 = 1.3;
const TAKEOVER_STEPS: usize = 4;
/// What counts as sound when a stream's busiest moments are looked for.
const SOUND: f64 = 0.1;

/// The names of the streams the lead chooses between, in the order of `Front`'s four.
pub const LEAD_NAMES: [&str; 4] = ["low", "mid", "high", "harmonic"];

/// `lead_stream(...)`: the stream that leads, per `LEAD_STEP` seconds, as contiguous spans with
/// the short ones (under `MIN_LEAD`) and the repeated ones merged. `events` are the four
/// streams' events as the chart has them, `nf` the fine frames, `loud01` the loudness of the
/// coarse frames at `ctimes`.
pub fn lead_stream(
    f: &Front,
    nf: usize,
    events: [&[[f64; 2]]; 4],
    loud01: &[f64],
    ctimes: &[f64],
    duration: f64,
) -> Vec<LeadSpan> {
    if nf == 0 || ctimes.is_empty() {
        return vec![];
    }
    let shares = [
        &f.share_low[..nf],
        &f.share_mid[..nf],
        &f.share_high[..nf],
        &f.energy_h[..nf],
    ];
    let comp: Vec<Vec<f32>> = shares
        .iter()
        .map(|v| {
            gaussian_filter1d(v, SHARE_SIGMA * FPS)
                .iter()
                .map(|&x| (x as f32).powf(SHARE_POWER))
                .collect()
        })
        .collect();
    let tot: Vec<f32> = (0..nf)
        .map(|t| ((comp[0][t] + comp[1][t]) + comp[2][t]) + comp[3][t] + EPS32)
        .collect();
    let loud_f: Vec<f64> = (0..nf)
        .map(|t| interp(t as f64 / FPS, ctimes, loud01))
        .collect();
    let sound: Vec<usize> = (0..nf).filter(|&t| loud_f[t] > SOUND).collect();
    let sal: Vec<Vec<f64>> = (0..4)
        .map(|k| {
            let mut imp = vec![0.0; nf];
            for &[t, s] in events[k] {
                imp[((t * FPS) as usize).min(nf - 1)] += s;
            }
            let act = gaussian_filter1d(&imp, BUSY_SIGMA * FPS);
            let reference = if sound.is_empty() {
                1.0
            } else {
                let loud: Vec<f64> = sound.iter().map(|&t| act[t]).collect();
                percentile(&loud, BUSY_PERCENTILE)
            };
            (0..nf)
                .map(|t| {
                    let share = SHARE_BASE + comp[k][t] / tot[t];
                    (act[t] / (reference + EPS)).min(BUSY_CEILING) * f64::from(share)
                })
                .collect()
        })
        .collect();

    let (mut cur, mut rival, mut since) = (None, None, 0);
    // The spans so far: start, end (not known until the next starts) and stream.
    let mut lead: Vec<(f64, f64, Option<usize>)> = vec![];
    for g in (0..nf).step_by(1.max((LEAD_STEP * FPS) as usize)) {
        let vals: [f64; 4] = std::array::from_fn(|k| sal[k][g]);
        let top = vals
            .iter()
            .enumerate()
            .fold(0, |best, (k, &v)| if v > vals[best] { k } else { best });
        let want = (loud_f[g] >= SILENCE && vals[top] >= MIN_SALIENCE).then_some(top);
        let new = match (cur, want) {
            (None, _) | (_, None) => want,
            (Some(c), Some(w)) if w != c && vals[w] > TAKEOVER_RATIO * vals[c] => {
                since = if rival == Some(w) { since + 1 } else { 1 };
                rival = Some(w);
                if since >= TAKEOVER_STEPS { want } else { cur }
            }
            (Some(_), Some(_)) => {
                (rival, since) = (None, 0);
                cur
            }
        };
        if lead.last().is_none_or(|l| l.2 != new) {
            lead.push((round_to(g as f64 / FPS, 3), f64::NAN, new));
        }
        cur = new;
    }
    for i in 1..lead.len() {
        lead[i - 1].1 = lead[i].0;
    }
    if let Some(n) = lead.len().checked_sub(1) {
        lead[n].1 = round_to(duration, 3);
        lead[0].0 = 0.0;
    }
    let mut merged: Vec<(f64, f64, Option<usize>)> = vec![];
    for s in lead {
        match merged.last_mut() {
            Some(m) if s.1 - s.0 < MIN_LEAD || m.2 == s.2 => m.1 = s.1,
            _ => merged.push(s),
        }
    }
    merged
        .into_iter()
        .map(|(start, end, stream)| LeadSpan {
            start,
            end,
            stream: stream.map(|k| LEAD_NAMES[k].to_string()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_matches_the_python() {
        // pick(odf, 0.06) of a float32 curve, the Python's events
        let odf: Vec<f64> = (0..80)
            .map(|i| {
                let i = f64::from(i);
                let loud = if i % 3.0 != 0.0 { 1.0 } else { 0.4 };
                f64::from(
                    (0.9 * (0.45 * i).sin().powi(2) * loud + 0.1 * (1.3 * i).cos()).max(0.0) as f32,
                )
            })
            .collect();
        let want = [
            [0.1, 0.95],
            [0.249, 0.89],
            [0.379, 0.94],
            [0.529, 0.93],
            [0.728, 0.96],
        ];
        assert_eq!(pick(&odf, 0.06), want);
        assert!(pick(&[0.0; 40], 0.06).is_empty());
    }
}
