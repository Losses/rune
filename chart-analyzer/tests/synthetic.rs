//! A synthetic song (a kick and a snare-like burst every half second, a hat in between) and a
//! noisy one without a clear pulse, analysed by the Python analyzer,
//! `analyze.back(analyze.front(x))`; the numbers below are what it gave.

use chart_analyzer::analyze;
use chart_analyzer::chart::Chart;
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

/// A noisy signal with a tone that comes and goes and clicks, `seconds` long (the noise from an
/// LCG, so that the Python makes the same one).
fn noisy(seconds: usize) -> Vec<f32> {
    let mut state = 12345u32;
    (0..SR * seconds)
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

/// What `back` made besides the rhythm: the sections as `(start, end, label, energy, repeat,
/// confidence)`, the number and first and last events of each stream, the lead and the curves'
/// lengths and sums.
fn assert_structure(
    chart: &Chart,
    sections: &[(f64, f64, &str, f64, u32, f64)],
    events: [(usize, [f64; 2], [f64; 2]); 5],
    lead: &[(f64, f64, Option<&str>)],
    sums: [f64; 5],
    len: usize,
) {
    let got: Vec<_> = chart
        .sections
        .iter()
        .map(|s| {
            (
                s.start,
                s.end,
                s.label.as_str(),
                s.energy,
                s.repeat,
                s.confidence,
            )
        })
        .collect();
    assert_eq!(got, sections);
    let s = &chart.streams;
    for (stream, (n, first, last)) in [&s.low, &s.mid, &s.high, &s.harmonic, &s.phrase]
        .into_iter()
        .zip(events)
    {
        assert_eq!(stream.events.len(), n, "{}", stream.name);
        assert_eq!(stream.events[0], first, "{}", stream.name);
        assert_eq!(stream.events[n - 1], last, "{}", stream.name);
    }
    let got: Vec<_> = chart
        .lead
        .iter()
        .map(|l| (l.start, l.end, l.stream.as_deref()))
        .collect();
    assert_eq!(got, lead);
    let c = &chart.curves;
    assert_eq!(c.hop, 0.049977);
    for (curve, sum) in [
        &c.loudness,
        &c.brightness,
        &c.percussive,
        &c.beat_conf,
        &c.feel,
    ]
    .into_iter()
    .zip(sums)
    {
        assert_eq!(curve.len(), len);
        close(curve.iter().sum(), sum, 1e-3);
    }
}

#[test]
fn structure_matches_the_python() {
    // With beats: the units are the beats, the sections' starts move to a downbeat.
    assert_structure(
        &analyze(&song()),
        &[
            (0.0, 10.746, "A", 0.54, 0, 1.0),
            (10.746, 24.0, "B", 0.56, 0, 0.21),
        ],
        [
            (48, [0.249, 1.0], [23.746, 1.0]),
            (48, [0.249, 1.0], [23.746, 1.0]),
            (79, [0.499, 1.0], [23.746, 0.21]),
            (48, [0.259, 1.0], [23.756, 1.0]),
            (2, [0.25, 1.0], [23.989, 0.93]),
        ],
        &[(0.0, 0.249, None), (0.249, 24.0, Some("harmonic"))],
        [264.99, 203.195, 94.648, 481.0, 2786.219],
        481,
    );
}

#[test]
fn structure_without_a_beat_matches_the_python() {
    // The units are half seconds.
    let chart = analyze(&noisy(40));
    assert_structure(
        &chart,
        &[
            (0.0, 20.404, "A", 0.67, 0, 1.0),
            (20.404, 40.0, "B", 0.67, 0, 0.27),
        ],
        [
            (177, [0.22, 1.0], [39.999, 0.35]),
            (176, [0.22, 0.99], [39.909, 0.99]),
            (176, [0.229, 0.95], [39.909, 0.92]),
            (177, [0.05, 0.78], [39.949, 1.0]),
            (6, [0.25, 1.0], [39.982, 0.79]),
        ],
        &[(0.0, 40.0, Some("harmonic"))],
        [537.905, 292.882, 334.883, 801.0, 5164.738],
        801,
    );
}

#[test]
fn short_and_silent_audio_make_a_whole_chart() {
    for n in [0, 100, SR / 2, SR * 3] {
        for x in [vec![0.0; n], noisy(1)[..n.min(SR)].to_vec()] {
            let chart = analyze(&x);
            assert_eq!(chart.curves.loudness.len(), chart.curves.feel.len());
            assert!(chart.sections.windows(2).all(|w| w[0].end == w[1].start));
            assert!(chart.lead.windows(2).all(|w| w[0].end == w[1].start));
        }
    }
}
