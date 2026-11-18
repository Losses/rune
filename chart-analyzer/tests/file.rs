//! `analyze_file` on a file on disk: it finds the pulse of a WAV, and stops early when told to.

use std::f64::consts::PI;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chart_analyzer::{Chart, analyze_file, decode::Cancelled};
use fsio::FsIo;

const RATE: u32 = 22050;

/// Clicks with a low thump under each, four to the second: the pulse of 240 beats per minute
/// for as long as `seconds`.
fn clicks(seconds: usize) -> Vec<i16> {
    let step = RATE as usize / 4;
    let mut x = vec![0i16; seconds * RATE as usize];
    for start in (0..x.len()).step_by(step) {
        for i in 0..2000.min(x.len() - start) {
            let t = i as f64 / f64::from(RATE);
            let thump = (2.0 * PI * 60.0 * t).sin() * (-t * 30.0).exp();
            let click = if i < 40 { 0.5 } else { 0.0 };
            x[start + i] = ((thump * 0.8 + click) * 30000.0) as i16;
        }
    }
    x
}

/// A mono WAV file of already encoded samples in a folder of its own.
fn write_wav<const N: usize>(
    name: &str,
    format: u16,
    samples: impl ExactSizeIterator<Item = [u8; N]>,
) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("chart_analyzer_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("clicks.wav");
    let len = samples.len() as u32;
    let sample_bytes = N as u16;
    let data = len * u32::from(sample_bytes);
    let float = format == 3;
    let header = if float { 48 } else { 36 };
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(header + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&format.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * u32::from(sample_bytes)).to_le_bytes());
    bytes.extend_from_slice(&sample_bytes.to_le_bytes());
    bytes.extend_from_slice(&(sample_bytes * 8).to_le_bytes());
    if float {
        bytes.extend_from_slice(b"fact");
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&len.to_le_bytes());
    }
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&s);
    }
    std::fs::write(&path, bytes).unwrap();
    path
}

fn remove(path: &Path) {
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_wav_is_analysed() {
    let path = write_wav("analysed", 1, clicks(30).into_iter().map(i16::to_le_bytes));
    let chart = analyze_file(&FsIo::new(), &path, &AtomicBool::new(false)).unwrap();
    remove(&path);
    assert_eq!(chart.title, "clicks");
    assert!((chart.duration - 30.0).abs() < 0.1, "{}", chart.duration);
    assert!(chart.beats.len() > 20, "{}", chart.beats.len());
}

#[test]
fn a_cancelled_analysis_stops_early() {
    // Ten minutes take several seconds to analyse.
    let path = write_wav(
        "cancelled",
        1,
        clicks(600).into_iter().map(i16::to_le_bytes),
    );
    let fsio = FsIo::new();
    let started = Instant::now();
    let flag = AtomicBool::new(true);
    let error = analyze_file(&fsio, &path, &flag).unwrap_err();
    assert!(error.downcast_ref::<Cancelled>().is_some(), "{error:#}");
    assert!(started.elapsed() < Duration::from_secs(1));

    // And while it runs.
    let flag = AtomicBool::new(false);
    let started = Instant::now();
    let error = std::thread::scope(|scope| {
        let job = scope.spawn(|| analyze_file(&fsio, &path, &flag));
        std::thread::sleep(Duration::from_millis(100));
        flag.store(true, Ordering::Relaxed);
        job.join().unwrap().unwrap_err()
    });
    remove(&path);
    assert!(error.downcast_ref::<Cancelled>().is_some(), "{error:#}");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn float_wav_analysis_returns_a_finite_chart_or_an_error() {
    let fsio = FsIo::new();
    let cancel = AtomicBool::new(false);
    let mut invalid = Vec::new();
    for (name, peak) in [
        ("normal", None),
        ("nan", Some(f32::NAN)),
        ("max", Some(f32::MAX)),
    ] {
        let samples = (0..RATE).map(|i| {
            let tone = 0.25 * (std::f32::consts::TAU * 440.0 * i as f32 / RATE as f32).sin();
            let sample = if i == RATE / 2 {
                peak.unwrap_or(tone)
            } else {
                tone
            };
            sample.to_le_bytes()
        });
        let path = write_wav(name, 3, samples); // IEEE float
        let result = analyze_file(&fsio, &path, &cancel);
        remove(&path);
        match result {
            Ok(chart) => {
                if peak.is_none() {
                    assert!(!chart.curves.loudness.is_empty());
                }
                // JSON encodes non-finite floats as null, which cannot be read
                // back into the chart's numeric fields.
                let json = serde_json::to_value(chart).unwrap();
                if let Err(err) = serde_json::from_value::<Chart>(json) {
                    invalid.push(format!("{name}: {err}"));
                }
            }
            Err(err) if peak.is_none() => panic!("{name}: {err:#}"),
            Err(_) => {}
        }
    }
    assert!(invalid.is_empty(), "{}", invalid.join("\n"));
}
