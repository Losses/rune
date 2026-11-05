//! Fixtures the tests of the decisions share: a synthetic chart and a scene's decider.

use super::Decider;
use super::manifest::Manifest;
use crate::chart::Chart;

/// A chart at 120 bpm in 4/4 (a beat every 0.5 s from 0, a bar every 2 s) with these
/// sections (start, end, energy), labelled from A, heard throughout.
pub(crate) fn chart_json(sections: &[(f64, f64, f64)]) -> String {
    let end = sections.last().map_or(0.0, |s| s.1);
    let beats: Vec<String> = (0..(end * 2.0) as u32)
        .map(|i| {
            format!(
                r#"{{"t": {}, "bar": {}, "pos": {}, "strength": 1, "confidence": 0.9}}"#,
                f64::from(i) * 0.5,
                i / 4,
                i % 4
            )
        })
        .collect();
    let sections: Vec<String> = sections
        .iter()
        .enumerate()
        .map(|(i, (start, end, energy))| {
            format!(
                r#"{{"start": {start}, "end": {end}, "label": "{}", "energy": {energy}, "repeat": 0, "confidence": 1}}"#,
                char::from(b'A' + i as u8)
            )
        })
        .collect();
    format!(
        r#"{{"version": 1, "title": "t", "audio": "/a.wav", "duration": {end},
        "analysis_seconds": 1, "analyzer": "test",
        "tempo": {{"bpm": 120, "confidence": 0.9, "feel": 5.0}},
        "beats": [{}],
        "meter": {{"beats_per_bar": 4, "confidence": 0.9}},
        "sections": [{}],
        "streams": {{"low": {{"name": "x", "events": []}}, "mid": {{"name": "x", "events": []}},
            "high": {{"name": "x", "events": []}}, "harmonic": {{"name": "x", "events": []}},
            "phrase": {{"name": "x", "events": []}}}},
        "lead": [],
        "curves": {{"hop": 0.5, "loudness": [0.5], "brightness": [0], "percussive": [0],
            "beat_conf": [0.9], "feel": [5]}}}}"#,
        beats.join(","),
        sections.join(",")
    )
}

/// The chart of [`chart_json`] as the runtime looks it up.
pub(crate) fn chart(sections: &[(f64, f64, f64)]) -> Chart {
    Chart::from_analysis(serde_json::from_str(&chart_json(sections)).unwrap())
}

/// The manifest of the built-in scene Armillary: shots, a burst, looks and every lens effect.
pub(crate) fn manifest() -> Manifest {
    Manifest::parse(include_str!("../../scenes/armillary/scene.toml")).unwrap()
}

/// The decider of that scene.
pub(crate) fn decider() -> Decider {
    Decider::new(&manifest())
}

/// What a lab shows of the scene Armillary, with its own inputs and bindings.
pub(crate) fn lab_scene() -> crate::lab::LabScene {
    lab_scene_of(&manifest())
}

/// The same scene with the inputs and bindings of `bindings` (the text of `[[input]]`s and
/// `[[bind]]`s) in place of its own.
pub(crate) fn lab_scene_with(bindings: &str) -> crate::lab::LabScene {
    lab_scene_of(&manifest().with_bindings(bindings).unwrap())
}

fn lab_scene_of(manifest: &Manifest) -> crate::lab::LabScene {
    use super::manifest::Value;
    let decider = Decider::new(manifest);
    crate::lab::LabScene {
        shots: decider
            .shots()
            .into_iter()
            .map(|(name, size, _)| (name, super::decider::size_name(size)))
            .collect(),
        looks: decider.look_names(),
        inputs: manifest.inputs.clone(),
        params: manifest.params.clone(),
        binds: manifest.binds.clone(),
        param_values: manifest
            .params
            .iter()
            .map(|p| match p.value().unwrap() {
                Value::Float(v) => v,
                Value::Int(v) => v as f32,
                _ => 0.0,
            })
            .collect(),
    }
}
