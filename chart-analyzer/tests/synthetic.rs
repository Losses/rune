//! A synthetic song (a kick and a snare-like burst every half second, a hat in between) analysed
//! by the Python analyzer, `analyze.back(analyze.front(x))`; the numbers below are what it gave.

use chart_analyzer::analyze;
use chart_analyzer::front::{SR, front};

/// 24 s at 120 bpm: a 70 Hz plus 2.5 kHz burst on the beat (from 0.25 s), a 6 kHz tick on the offbeat.
fn song() -> Vec<f32> {
    let mut x = vec![0.0f64; SR * 24];
    let tau = std::f64::consts::TAU;
    for k in 0..48 {
        let beat = 0.25 + 0.5 * k as f64;
        for (start, weight, decay, hz, second) in [
            (beat, 1.0, 0.05, 70.0, Some(2500.0)),
            (beat + 0.25, 0.2, 0.02, 6000.0, None),
        ] {
            let first = (start * SR as f64).ceil() as usize;
            for (i, v) in x.iter_mut().enumerate().skip(first) {
                let t = i as f64 / SR as f64 - start;
                let tone = (tau * hz * t).sin() + second.map_or(0.0, |s| 0.5 * (tau * s * t).sin());
                *v += weight * (-t / decay).exp() * tone;
            }
        }
    }
    x.iter().map(|v| (0.3 * v) as f32).collect()
}

fn close(got: f64, want: f64, rel: f64) {
    assert!(
        (got - want).abs() <= rel * want.abs().max(1.0),
        "{got} != {want}"
    );
}

#[test]
fn front_curves_match_the_python() {
    let f = front(&song());
    assert_eq!((f.flux.len(), f.loud.len()), (2406, 481));
    close(f.raw_low.iter().sum(), 378.54718017578125, 2e-3);
    close(f.band_mid.iter().sum(), 100.13359069824219, 2e-3);
    close(f.chroma.iter().flatten().sum(), 622.1732177734375, 2e-3);
    close(f.timbre.iter().flatten().sum(), 38403.602568465125, 2e-3);
    close(f.loud.iter().sum(), -16251.3857421875, 2e-3);
    close(f.cent.iter().sum(), 1230790.1457113526, 2e-3);
    close(f.energy_p.iter().sum(), 3.32454252243042, 2e-3);
}

#[test]
fn rhythm_matches_the_python() {
    let chart = analyze(&song());
    assert_eq!(
        (chart.tempo.bpm, chart.tempo.confidence, chart.tempo.feel),
        (119.97, 1.0, 5.79)
    );
    assert_eq!(
        (chart.meter.beats_per_bar, chart.meter.confidence),
        (4, 0.39)
    );
    assert_eq!(chart.beats.len(), 47);
    let (first, last) = (&chart.beats[0], &chart.beats[46]);
    assert_eq!((first.t, first.bar, first.pos), (0.249, -1, 3));
    assert_eq!((last.t, last.bar, last.pos), (23.247, 11, 1));
    assert_eq!(chart.duration, 24.0);
}

#[test]
fn silence_and_short_audio_have_no_beats() {
    for n in [0, 100, SR * 3, SR * 12] {
        let chart = analyze(&vec![0.0; n]);
        assert!(chart.beats.is_empty());
        assert_eq!(chart.tempo.bpm, 0.0);
    }
}
